//! The thread that turns mpv events into [`PlayerEvent`]s.

use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use libmpv2::events::{Event, PropertyData};
use libmpv2::{Format, Mpv, mpv_end_file_reason, mpv_format};

use crate::error::describe;
use crate::{PlayerEvent, UnloadReason, tracks};

/// Name of the observed property that signals the end of the media.
pub(crate) const EOF_REACHED: &str = "eof-reached";
const TIME_POS: &str = "time-pos";
const PAUSE: &str = "pause";
const VOLUME: &str = "volume";
const SPEED: &str = "speed";
const MUTE: &str = "mute";
const TRACK_COUNT: &str = "track-list/count";
const AID: &str = "aid";
const SID: &str = "sid";

/// Minimum distance between two reported positions.
const POSITION_STEP: Duration = Duration::from_millis(250);

/// Playback state shared between the [`crate::Player`] and its event thread.
#[derive(Debug, Default)]
pub(crate) struct Shared {
    /// True between `file-loaded` and `end-file`.
    pub(crate) loaded: AtomicBool,
    /// Set by `stop()` so the next stop-type end is reported as `Stopped`.
    pub(crate) stopping: AtomicBool,
}

/// Drops position updates that are too close to the last reported one.
#[derive(Debug)]
struct PositionThrottle {
    min_step: Duration,
    last: Option<Duration>,
}

impl PositionThrottle {
    fn new(min_step: Duration) -> Self {
        Self {
            min_step,
            last: None,
        }
    }

    fn accept(&mut self, position: Duration) -> bool {
        let moved = self
            .last
            .is_none_or(|last| position.abs_diff(last) >= self.min_step);
        if moved {
            self.last = Some(position);
        }
        moved
    }

    /// Makes the next position pass regardless of distance.
    fn force_next(&mut self) {
        self.last = None;
    }
}

/// Registers the property observers the event loop relies on.
pub(crate) fn observe(mpv: &Mpv) -> libmpv2::Result<()> {
    mpv.observe_property(EOF_REACHED, Format::Flag, 0)?;
    mpv.observe_property(TIME_POS, Format::Double, 0)?;
    mpv.observe_property(PAUSE, Format::Flag, 0)?;
    mpv.observe_property(VOLUME, Format::Double, 0)?;
    mpv.observe_property(SPEED, Format::Double, 0)?;
    mpv.observe_property(MUTE, Format::Flag, 0)?;
    observe_changes(mpv, TRACK_COUNT)?;
    observe_changes(mpv, AID)?;
    observe_changes(mpv, SID)
}

/// Observes `name` without a value, so mpv reports every change it flags
/// instead of only those that differ from the last value it read.
#[allow(unsafe_code)]
fn observe_changes(mpv: &Mpv, name: &str) -> libmpv2::Result<()> {
    let name = CString::new(name)?;
    // SAFETY: called before the event thread starts; `mpv.ctx` is a live handle
    // for the lifetime of `mpv`, `name` is a valid C string that mpv copies,
    // and the NONE format needs no data buffer.
    let code = unsafe {
        libmpv2_sys::mpv_observe_property(mpv.ctx.as_ptr(), 0, name.as_ptr(), mpv_format::None)
    };
    if code < 0 {
        Err(libmpv2::Error::Raw(code))
    } else {
        Ok(())
    }
}

/// Runs the event loop until mpv shuts down.
pub(crate) fn run(mpv: &Mpv, shared: &Shared, on_event: &(impl Fn(PlayerEvent) + ?Sized)) {
    let mut throttle = PositionThrottle::new(POSITION_STEP);
    loop {
        let event = match mpv.wait_event(-1.0) {
            None => continue,
            Some(event) => event,
        };
        match event {
            Ok(Event::Shutdown) => return,
            Ok(Event::FileLoaded) => {
                shared.loaded.store(true, Ordering::SeqCst);
                throttle.force_next();
                on_event(file_loaded(mpv));
            }
            Ok(Event::Seek) => on_event(PlayerEvent::Seeking),
            Ok(Event::PlaybackRestart) => {
                report_position_now(mpv, &mut throttle, on_event);
                on_event(PlayerEvent::Seeked);
            }
            Ok(Event::PropertyChange {
                name: EOF_REACHED,
                change: PropertyData::Flag(true),
                ..
            }) => {
                on_event(PlayerEvent::Ended);
                // time-pos stops changing at the end, so the throttle could
                // otherwise drop the last position.
                report_position_now(mpv, &mut throttle, on_event);
            }
            Ok(Event::PropertyChange {
                name: TIME_POS,
                change: PropertyData::Double(secs),
                ..
            }) => {
                if let Some(position) = to_duration(secs)
                    && throttle.accept(position)
                {
                    on_event(PlayerEvent::Position(position));
                }
            }
            Ok(Event::PropertyChange {
                name: PAUSE,
                change: PropertyData::Flag(paused),
                ..
            }) => {
                on_event(PlayerEvent::Paused(paused));
                report_position_now(mpv, &mut throttle, on_event);
            }
            Ok(Event::PropertyChange {
                name: VOLUME,
                change: PropertyData::Double(volume),
                ..
            }) => on_event(PlayerEvent::Volume(volume)),
            Ok(Event::PropertyChange {
                name: SPEED,
                change: PropertyData::Double(speed),
                ..
            }) => on_event(PlayerEvent::Speed(speed)),
            Ok(Event::PropertyChange {
                name: MUTE,
                change: PropertyData::Flag(muted),
                ..
            }) => on_event(PlayerEvent::Muted(muted)),
            Ok(Event::PropertyChange {
                name: TRACK_COUNT | AID | SID,
                ..
            }) => {
                // On a read failure the previous list stays valid, so nothing is sent.
                if shared.loaded.load(Ordering::SeqCst)
                    && let Ok(tracks) = tracks::read(mpv)
                {
                    on_event(PlayerEvent::Tracks(tracks));
                }
            }
            Ok(Event::EndFile(reason)) => {
                shared.loaded.store(false, Ordering::SeqCst);
                let stopping = shared.stopping.swap(false, Ordering::SeqCst);
                match reason {
                    mpv_end_file_reason::Quit => {}
                    mpv_end_file_reason::Stop if !stopping => {
                        on_event(PlayerEvent::Unloaded(UnloadReason::Replaced));
                    }
                    _ => on_event(PlayerEvent::Unloaded(UnloadReason::Stopped)),
                }
            }
            // libmpv2 reports an `end-file` carrying an error this way.
            Err(error) => {
                shared.loaded.store(false, Ordering::SeqCst);
                shared.stopping.store(false, Ordering::SeqCst);
                on_event(PlayerEvent::Unloaded(UnloadReason::Error(describe(&error))));
            }
            Ok(_) => {}
        }
    }
}

fn to_duration(secs: f64) -> Option<Duration> {
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs))
}

/// Reports the current position even when `time-pos` did not change, as it
/// does not after a pause or a seek while paused.
fn report_position_now(
    mpv: &Mpv,
    throttle: &mut PositionThrottle,
    on_event: &(impl Fn(PlayerEvent) + ?Sized),
) {
    throttle.force_next();
    if let Some(position) = mpv.get_property::<f64>(TIME_POS).ok().and_then(to_duration)
        && throttle.accept(position)
    {
        on_event(PlayerEvent::Position(position));
    }
}

fn file_loaded(mpv: &Mpv) -> PlayerEvent {
    let duration = mpv
        .get_property::<f64>("duration")
        .ok()
        .and_then(to_duration);
    let tracks = tracks::read(mpv).unwrap_or_default();
    PlayerEvent::FileLoaded { duration, tracks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_accepts_only_moves_of_at_least_the_step() {
        let ms = Duration::from_millis;
        let mut throttle = PositionThrottle::new(ms(250));
        assert!(throttle.accept(ms(0)));
        assert!(!throttle.accept(ms(100)));
        assert!(throttle.accept(ms(300)));
        assert!(!throttle.accept(ms(200)));
        throttle.force_next();
        assert!(throttle.accept(ms(210)));
    }
}
