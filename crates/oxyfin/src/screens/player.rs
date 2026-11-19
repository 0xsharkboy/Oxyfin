//! Player screen: sends the user's commands and shows the controller's events.

use std::rc::Rc;
use std::time::Duration;

use oxyfin_core::{
    PlaybackCommand, PlaybackError, PlaybackEvent, SEEK_STEP, SPEEDS, SegmentKind, SkippableSegment,
};
use oxyfin_player::SeekTarget;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, ModelRc, SharedString, VecModel};

use crate::present::{
    audio_rows, clock, next_up_title, playback_error_message, playback_title, subtitle_rows,
};
use crate::{AppWindow, Message, NextUp, SkipKind};

/// Volume change of one step of the volume keys, in percent.
pub const VOLUME_STEP: f64 = 5.0;

/// Wires the player controls of `window` to `send`.
///
/// `send` is the only way out of the screen: the playback controller in the
/// application, a recorder in tests. Fullscreen is window state and is
/// handled here.
pub fn bind(window: &AppWindow, send: Rc<dyn Fn(PlaybackCommand)>) {
    window.set_player_speeds(ModelRc::new(VecModel::from(
        SPEEDS.iter().map(|speed| *speed as f32).collect::<Vec<_>>(),
    )));

    let command = |command: PlaybackCommand| {
        let send = Rc::clone(&send);
        move || send(command.clone())
    };
    window.on_player_command_stop(command(PlaybackCommand::Stop));
    window.on_player_command_previous(command(PlaybackCommand::Previous));
    window.on_player_command_next(command(PlaybackCommand::Next));
    window.on_player_command_play_next_now(command(PlaybackCommand::PlayNextNow));
    window.on_player_command_cancel_next_up(command(PlaybackCommand::CancelNextUp));

    let to = Rc::clone(&send);
    window.on_player_command_set_paused(move |paused| to(PlaybackCommand::SetPaused(paused)));
    let to = Rc::clone(&send);
    window.on_player_command_set_muted(move |muted| to(PlaybackCommand::SetMuted(muted)));
    let to = Rc::clone(&send);
    window.on_player_command_set_speed(move |speed| to(PlaybackCommand::SetSpeed(speed.into())));
    let to = Rc::clone(&send);
    window.on_player_command_set_volume(move |volume| {
        to(PlaybackCommand::SetVolume(
            f64::from(volume).clamp(0.0, 100.0),
        ))
    });
    let to = Rc::clone(&send);
    window.on_player_command_select_audio(move |index| to(PlaybackCommand::SelectAudio(index)));
    let to = Rc::clone(&send);
    window.on_player_command_select_subtitle(move |index| {
        to(PlaybackCommand::SelectSubtitle(subtitle_choice(index)))
    });

    let (to, weak) = (Rc::clone(&send), window.as_weak());
    window.on_player_command_seek(move |seconds| {
        let seconds = seconds.max(0.0);
        if let Some(window) = weak.upgrade() {
            window.set_player_position(seconds);
            window.set_player_elapsed(clock(Duration::from_secs_f32(seconds)));
            window.set_player_seek_pending(true);
        }
        to(PlaybackCommand::Seek(SeekTarget::Absolute(
            Duration::from_secs_f32(seconds),
        )));
    });
    let (to, weak) = (Rc::clone(&send), window.as_weak());
    window.on_player_command_skip(move |direction| {
        if let Some(window) = weak.upgrade() {
            window.set_player_seek_pending(true);
        }
        to(PlaybackCommand::Seek(skip_target(direction)));
    });
    let (to, weak) = (Rc::clone(&send), window.as_weak());
    window.on_player_command_step_speed(move |direction| {
        let Some(window) = weak.upgrade() else { return };
        if let Some(speed) = step_speed(window.get_player_speed().into(), direction) {
            to(PlaybackCommand::SetSpeed(speed));
        }
    });
    let (to, weak) = (send, window.as_weak());
    window.on_player_command_step_volume(move |direction| {
        let Some(window) = weak.upgrade() else { return };
        to(PlaybackCommand::SetVolume(step_volume(
            window.get_player_volume().into(),
            direction,
        )));
    });

    let weak = window.as_weak();
    window.on_player_cursor_hidden(move |x, y| {
        let weak = weak.clone();
        // Replayed after the callback returns, outside the timer that hid the controls.
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(window) = weak.upgrade() {
                window.window().dispatch_event(WindowEvent::PointerMoved {
                    position: LogicalPosition::new(x, y),
                });
            }
        });
    });

    let weak = window.as_weak();
    window.on_player_toggle_fullscreen(move || {
        if let Some(window) = weak.upgrade() {
            let fullscreen = !window.window().is_fullscreen();
            window.window().set_fullscreen(fullscreen);
        }
    });
}

/// Clears what a previous playback left on the player screen.
pub fn reset(window: &AppWindow) {
    window.set_player_title(Default::default());
    window.set_player_subtitle(Default::default());
    window.set_player_loading(true);
    window.set_player_has_error(false);
    window.set_player_paused(false);
    window.set_player_speed(1.0);
    window.set_player_has_next(false);
    window.set_player_audio(ModelRc::default());
    window.set_player_subtitles(ModelRc::default());
    window.set_player_next_up(NextUp::default());
    window.set_player_scrubbing(false);
    show_segment(window, None);
    show_progress(window, Duration::ZERO, None);
}

/// Leaves the player screen: the window does not stay fullscreen behind the other screens.
pub fn leave(window: &AppWindow) {
    if window.window().is_fullscreen() {
        window.window().set_fullscreen(false);
    }
}

/// Shows a playback event on the player screen. Returns the message to show
/// in the notice banner when the event calls for one.
pub fn apply(window: &AppWindow, event: PlaybackEvent) -> Option<Message> {
    match event {
        PlaybackEvent::Loading { title, .. } => {
            let (title, subtitle) = playback_title(&title);
            window.set_player_title(title);
            window.set_player_subtitle(subtitle);
            window.set_player_loading(true);
            window.set_player_has_error(false);
            window.set_player_next_up(NextUp::default());
            show_segment(window, None);
            show_progress(window, Duration::ZERO, None);
        }
        PlaybackEvent::Ready {
            duration,
            audio,
            subtitles,
            has_next,
            ..
        } => {
            window.set_player_loading(false);
            window.set_player_has_next(has_next);
            show_tracks(window, &audio, &subtitles);
            window.set_player_seek_pending(false);
            window.set_player_duration(duration.map_or(0.0, |duration| duration.as_secs_f32()));
            window.set_player_total(clock(duration.unwrap_or_default()));
        }
        PlaybackEvent::Position(position) => {
            if !window.get_player_scrubbing() && !window.get_player_seek_pending() {
                window.set_player_position(position.as_secs_f32());
                window.set_player_elapsed(clock(position));
            }
        }
        PlaybackEvent::Paused(paused) => window.set_player_paused(paused),
        PlaybackEvent::Speed(speed) => window.set_player_speed(speed as f32),
        PlaybackEvent::Volume(volume) => window.set_player_volume(volume as f32),
        PlaybackEvent::Muted(muted) => window.set_player_muted(muted),
        PlaybackEvent::Seeking => {}
        PlaybackEvent::Seeked => window.set_player_seek_pending(false),
        PlaybackEvent::TracksChanged { audio, subtitles } => {
            show_tracks(window, &audio, &subtitles);
        }
        PlaybackEvent::NextUpCountdown { episode, remaining } => {
            let mut next = window.get_player_next_up();
            let id = SharedString::from(episode.id.as_str());
            if next.id != id {
                next = NextUp {
                    id,
                    title: next_up_title(&episode),
                    ..NextUp::default()
                };
            }
            next.visible = true;
            next.remaining = i32::try_from(remaining).unwrap_or(i32::MAX);
            window.set_player_next_up(next);
        }
        PlaybackEvent::NextUpCancelled => window.set_player_next_up(NextUp::default()),
        PlaybackEvent::Stopped => {
            window.set_player_next_up(NextUp::default());
            show_segment(window, None);
        }
        PlaybackEvent::Segment(segment) => show_segment(window, segment),
        PlaybackEvent::Ended | PlaybackEvent::SessionExpired => {}
        PlaybackEvent::Error(error @ PlaybackError::Command { .. }) => {
            log::warn!("playback command failed: {error}");
            window.set_player_seek_pending(false);
            return Some(playback_error_message(&error));
        }
        PlaybackEvent::Error(error) => {
            log::warn!("playback failed: {error}");
            window.set_player_loading(false);
            window.set_player_seek_pending(false);
            show_segment(window, None);
            window.set_player_error(playback_error_message(&error));
            window.set_player_has_error(true);
        }
    }
    None
}

fn show_progress(window: &AppWindow, position: Duration, duration: Option<Duration>) {
    window.set_player_seek_pending(false);
    window.set_player_position(position.as_secs_f32());
    window.set_player_elapsed(clock(position));
    window.set_player_duration(duration.map_or(0.0, |duration| duration.as_secs_f32()));
    window.set_player_total(clock(duration.unwrap_or_default()));
}

/// Offers to skip `segment`, or hides the offer.
fn show_segment(window: &AppWindow, segment: Option<SkippableSegment>) {
    window.set_player_skip_kind(segment.map_or(SkipKind::None, |segment| skip_kind(segment.kind)));
    window.set_player_skip_end(segment.map_or(0.0, |segment| segment.end.as_secs_f32()));
}

fn skip_kind(kind: SegmentKind) -> SkipKind {
    match kind {
        SegmentKind::Intro => SkipKind::Intro,
        SegmentKind::Recap => SkipKind::Recap,
        SegmentKind::Preview => SkipKind::Preview,
        SegmentKind::Credits => SkipKind::Credits,
    }
}

fn show_tracks(
    window: &AppWindow,
    audio: &[oxyfin_core::AudioOption],
    subtitles: &[oxyfin_core::SubtitleOption],
) {
    window.set_player_audio(ModelRc::new(VecModel::from(audio_rows(audio))));
    window.set_player_subtitles(ModelRc::new(VecModel::from(subtitle_rows(subtitles))));
}

/// Seek of the skip buttons and arrow keys: backward for a negative direction.
fn skip_target(direction: i32) -> SeekTarget {
    if direction < 0 {
        SeekTarget::Backward(SEEK_STEP)
    } else {
        SeekTarget::Forward(SEEK_STEP)
    }
}

/// The speed of [`SPEEDS`] after `current` in `direction`, if any.
fn step_speed(current: f64, direction: i32) -> Option<f64> {
    const SAME: f64 = 1e-3;
    if direction < 0 {
        SPEEDS
            .iter()
            .rev()
            .copied()
            .find(|speed| *speed < current - SAME)
    } else {
        SPEEDS.iter().copied().find(|speed| *speed > current + SAME)
    }
}

/// The volume one step from `current` in `direction`, within 0 to 100.
fn step_volume(current: f64, direction: i32) -> f64 {
    let step = if direction < 0 {
        -VOLUME_STEP
    } else {
        VOLUME_STEP
    };
    (current + step).clamp(0.0, 100.0)
}

/// The subtitle track of a menu row: the "Off" row has a negative index.
fn subtitle_choice(index: i32) -> Option<i32> {
    (index >= 0).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_go_one_step_either_way() {
        assert_eq!(skip_target(1), SeekTarget::Forward(SEEK_STEP));
        assert_eq!(skip_target(-1), SeekTarget::Backward(SEEK_STEP));
    }

    #[test]
    fn speed_steps_through_the_offered_speeds() {
        assert_eq!(step_speed(1.0, 1), Some(1.25));
        assert_eq!(step_speed(1.0, -1), Some(0.75));
        assert_eq!(step_speed(2.0, 1), None);
        assert_eq!(step_speed(0.5, -1), None);
    }

    #[test]
    fn speed_steps_from_an_unlisted_speed_to_its_neighbours() {
        assert_eq!(step_speed(1.1, 1), Some(1.25));
        assert_eq!(step_speed(1.1, -1), Some(1.0));
        assert_eq!(step_speed(3.0, -1), Some(2.0));
    }

    #[test]
    fn volume_steps_stay_within_bounds() {
        assert_eq!(step_volume(50.0, 1), 55.0);
        assert_eq!(step_volume(50.0, -1), 45.0);
        assert_eq!(step_volume(98.0, 1), 100.0);
        assert_eq!(step_volume(3.0, -1), 0.0);
    }

    #[test]
    fn off_row_turns_subtitles_off() {
        assert_eq!(subtitle_choice(-1), None);
        assert_eq!(subtitle_choice(0), Some(0));
        assert_eq!(subtitle_choice(3), Some(3));
    }
}
