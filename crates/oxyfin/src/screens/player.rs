//! Player screen: sends the user's commands and shows the controller's events.

use std::rc::Rc;

use oxyfin_core::{PlaybackCommand, PlaybackError, PlaybackEvent};

use crate::AppWindow;
use crate::present::{playback_error_message, playback_title};

/// Wires the player controls of `window` to `send`.
pub fn bind(window: &AppWindow, send: Rc<dyn Fn(PlaybackCommand)>) {
    window.on_player_stop(move || send(PlaybackCommand::Stop));
}

/// Clears what a previous playback left on the player screen.
pub fn reset(window: &AppWindow) {
    window.set_player_title(Default::default());
    window.set_player_subtitle(Default::default());
    window.set_player_loading(true);
    window.set_player_has_error(false);
}

/// Shows a playback event on the player screen.
pub fn apply(window: &AppWindow, event: PlaybackEvent) {
    match event {
        PlaybackEvent::Loading { title, .. } => {
            let (title, subtitle) = playback_title(&title);
            window.set_player_title(title);
            window.set_player_subtitle(subtitle);
            window.set_player_loading(true);
            window.set_player_has_error(false);
        }
        PlaybackEvent::Ready { .. } => window.set_player_loading(false),
        PlaybackEvent::Error(error @ PlaybackError::Command { .. }) => {
            log::warn!("playback command failed: {error}");
            window.set_notice(playback_error_message(&error));
            window.set_notice_detail(Default::default());
            window.set_notice_visible(true);
        }
        PlaybackEvent::Error(error) => {
            log::warn!("playback failed: {error}");
            window.set_player_loading(false);
            window.set_player_error(playback_error_message(&error));
            window.set_player_has_error(true);
        }
        _ => {}
    }
}
