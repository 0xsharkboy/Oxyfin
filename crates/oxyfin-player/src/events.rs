//! The thread that turns mpv events into [`PlayerEvent`]s.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use libmpv2::events::{Event, PropertyData};
use libmpv2::{Format, Mpv, mpv_end_file_reason};

use crate::{PlayerEvent, UnloadReason, tracks};

/// Name of the observed property that signals the end of the media.
pub(crate) const EOF_REACHED: &str = "eof-reached";

/// Playback state shared between the [`crate::Player`] and its event thread.
#[derive(Debug, Default)]
pub(crate) struct Shared {
    /// True between `file-loaded` and `end-file`.
    pub(crate) loaded: AtomicBool,
    /// True between `start-file` and `end-file`.
    pub(crate) active: AtomicBool,
    /// Set when a load replaces the active media.
    pub(crate) replacing: AtomicBool,
}

/// Registers the property observers the event loop relies on.
pub(crate) fn observe(mpv: &Mpv) -> libmpv2::Result<()> {
    mpv.observe_property(EOF_REACHED, Format::Flag, 0)
}

/// Runs the event loop until mpv shuts down.
pub(crate) fn run(mpv: &Mpv, shared: &Shared, on_event: &(impl Fn(PlayerEvent) + ?Sized)) {
    loop {
        let event = match mpv.wait_event(-1.0) {
            None => continue,
            Some(event) => event,
        };
        match event {
            Ok(Event::Shutdown) => return,
            Ok(Event::StartFile) => {
                shared.active.store(true, Ordering::SeqCst);
                shared.replacing.store(false, Ordering::SeqCst);
            }
            Ok(Event::FileLoaded) => {
                shared.loaded.store(true, Ordering::SeqCst);
                on_event(file_loaded(mpv));
            }
            Ok(Event::PropertyChange {
                name: EOF_REACHED,
                change: PropertyData::Flag(true),
                ..
            }) => on_event(PlayerEvent::Ended),
            Ok(Event::EndFile(reason)) => {
                shared.loaded.store(false, Ordering::SeqCst);
                shared.active.store(false, Ordering::SeqCst);
                match reason {
                    mpv_end_file_reason::Quit => {}
                    mpv_end_file_reason::Stop if shared.replacing.swap(false, Ordering::SeqCst) => {
                        on_event(PlayerEvent::Unloaded(UnloadReason::Replaced));
                    }
                    _ => on_event(PlayerEvent::Unloaded(UnloadReason::Stopped)),
                }
            }
            // libmpv2 reports an `end-file` carrying an error this way.
            Err(error) => {
                shared.loaded.store(false, Ordering::SeqCst);
                shared.active.store(false, Ordering::SeqCst);
                on_event(PlayerEvent::Unloaded(UnloadReason::Error(
                    error.to_string(),
                )));
            }
            Ok(_) => {}
        }
    }
}

fn file_loaded(mpv: &Mpv) -> PlayerEvent {
    let duration = mpv
        .get_property::<f64>("duration")
        .ok()
        .filter(|secs| secs.is_finite() && *secs >= 0.0)
        .map(Duration::from_secs_f64);
    let tracks = tracks::read(mpv).unwrap_or_default();
    PlayerEvent::FileLoaded { duration, tracks }
}
