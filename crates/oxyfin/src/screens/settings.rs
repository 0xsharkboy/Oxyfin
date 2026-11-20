//! Settings screen: the signed-in account, the playback choices, the subtitle
//! style, the Quick Connect code entry and the application version.

use oxyfin_core::{
    COUNTDOWNS, CoreError, PlaybackPrefs, SubtitleBackground, SubtitleColor, SubtitlePosition,
    SubtitleSize, SubtitleStyle, TokenStorage, quick_connect_code,
};

use crate::state::SessionInfo;
use crate::{AppWindow, AuthorizeState, PlaybackChoice, SubtitleChoice, TokenStore};

/// Version of the application, shown on the settings screen.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Fills the settings screen with the signed-in session, or clears it without one.
pub fn show(window: &AppWindow, session: Option<&SessionInfo>) {
    window.set_app_version(VERSION.into());
    window.set_settings_server_name(
        session
            .and_then(|info| info.server_name.as_deref())
            .unwrap_or_default()
            .into(),
    );
    window.set_settings_server_address(
        session
            .map(|info| info.server_url.as_str())
            .unwrap_or_default()
            .into(),
    );
    window.set_settings_user_name(
        session
            .map(|info| info.user_name.as_str())
            .unwrap_or_default()
            .into(),
    );
    window.set_settings_token_store(match session.map(|info| info.token_storage) {
        Some(TokenStorage::File) => TokenStore::File,
        Some(TokenStorage::Memory) => TokenStore::Memory,
        Some(TokenStorage::Keyring) | None => TokenStore::Keyring,
    });
    // A code still being sent keeps its state until the server answers.
    if window.get_settings_authorize_state() != AuthorizeState::Busy {
        window.set_settings_authorize_state(AuthorizeState::Idle);
        window.set_settings_authorize_code("".into());
    }
}

/// Whether `code` holds a Quick Connect code to send, spaces aside.
pub fn code_ready(code: &str) -> bool {
    !quick_connect_code(code).is_empty()
}

/// What the Devices section shows once the server answered a code; a session that
/// expired is left to the caller.
pub fn authorize_state(result: &Result<bool, CoreError>) -> AuthorizeState {
    match result {
        Ok(true) => AuthorizeState::Done,
        Ok(false) => AuthorizeState::WrongCode,
        Err(CoreError::QuickConnectDisabled) => AuthorizeState::Unavailable,
        Err(_) => AuthorizeState::Failed,
    }
}

/// The chips selected for `style`.
pub fn subtitle_choice(style: SubtitleStyle) -> SubtitleChoice {
    SubtitleChoice {
        size: position_of(&SubtitleSize::ALL, style.size),
        color: position_of(&SubtitleColor::ALL, style.color),
        background: position_of(&SubtitleBackground::ALL, style.background),
        position: position_of(&SubtitlePosition::ALL, style.position),
    }
}

/// The style the chips select; an index out of range picks the default.
pub fn subtitle_style(choice: &SubtitleChoice) -> SubtitleStyle {
    SubtitleStyle {
        size: pick(&SubtitleSize::ALL, choice.size),
        color: pick(&SubtitleColor::ALL, choice.color),
        background: pick(&SubtitleBackground::ALL, choice.background),
        position: pick(&SubtitlePosition::ALL, choice.position),
    }
}

/// The chips selected for `prefs`. A countdown the chips do not offer selects none.
pub fn playback_choice(prefs: PlaybackPrefs) -> PlaybackChoice {
    PlaybackChoice {
        autoplay: i32::from(prefs.autoplay_next),
        countdown: COUNTDOWNS
            .iter()
            .position(|countdown| *countdown == prefs.countdown)
            .and_then(|index| i32::try_from(index).ok())
            .unwrap_or(-1),
        intros: i32::from(prefs.skip_intros),
        recaps: i32::from(prefs.skip_recaps),
        tracks: i32::from(prefs.remember_tracks),
    }
}

/// The choices the chips select; a countdown index out of range keeps the one of `previous`.
pub fn playback_prefs(choice: &PlaybackChoice, previous: PlaybackPrefs) -> PlaybackPrefs {
    PlaybackPrefs {
        autoplay_next: choice.autoplay == 1,
        countdown: usize::try_from(choice.countdown)
            .ok()
            .and_then(|index| COUNTDOWNS.get(index).copied())
            .unwrap_or(previous.countdown),
        skip_intros: choice.intros == 1,
        skip_recaps: choice.recaps == 1,
        remember_tracks: choice.tracks == 1,
    }
}

fn position_of<T: PartialEq>(all: &[T], value: T) -> i32 {
    all.iter()
        .position(|item| *item == value)
        .and_then(|index| i32::try_from(index).ok())
        .unwrap_or_default()
}

fn pick<T: Copy + Default>(all: &[T], index: i32) -> T {
    usize::try_from(index)
        .ok()
        .and_then(|index| all.get(index).copied())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(server_name: Option<&str>) -> SessionInfo {
        SessionInfo {
            user_name: "alice".into(),
            server_url: "http://h:8096".into(),
            server_name: server_name.map(str::to_owned),
            token_storage: TokenStorage::Keyring,
            show_file_token_warning: false,
        }
    }

    #[test]
    fn the_token_store_follows_the_session() {
        i_slint_backend_testing::init_no_event_loop();
        let window = AppWindow::new().unwrap();

        for (storage, store) in [
            (TokenStorage::Keyring, TokenStore::Keyring),
            (TokenStorage::File, TokenStore::File),
            (TokenStorage::Memory, TokenStore::Memory),
        ] {
            let session = SessionInfo {
                token_storage: storage,
                ..info(None)
            };
            show(&window, Some(&session));
            assert_eq!(window.get_settings_token_store(), store);
        }
    }

    #[test]
    fn the_server_name_and_address_are_shown_together() {
        i_slint_backend_testing::init_no_event_loop();
        let window = AppWindow::new().unwrap();

        show(&window, Some(&info(Some("Home"))));
        assert_eq!(window.get_settings_server_name(), "Home");
        assert_eq!(window.get_settings_server_address(), "http://h:8096");

        show(&window, Some(&info(None)));
        assert_eq!(window.get_settings_server_name(), "");
        assert_eq!(window.get_settings_server_address(), "http://h:8096");

        show(&window, None);
        assert_eq!(window.get_settings_server_name(), "");
    }

    #[test]
    fn showing_the_settings_clears_a_finished_code() {
        i_slint_backend_testing::init_no_event_loop();
        let window = AppWindow::new().unwrap();

        window.set_settings_authorize_code("482913".into());
        window.set_settings_authorize_state(AuthorizeState::Done);
        show(&window, Some(&info(None)));
        assert_eq!(window.get_settings_authorize_state(), AuthorizeState::Idle);
        assert_eq!(window.get_settings_authorize_code(), "");

        window.set_settings_authorize_code("482913".into());
        window.set_settings_authorize_state(AuthorizeState::Busy);
        show(&window, Some(&info(None)));
        assert_eq!(window.get_settings_authorize_state(), AuthorizeState::Busy);
        assert_eq!(window.get_settings_authorize_code(), "482913");
    }

    #[test]
    fn a_code_is_ready_once_it_has_more_than_spaces() {
        assert!(!code_ready(""));
        assert!(!code_ready("  \t"));
        assert!(code_ready(" 4"));
        assert!(code_ready("482 913"));
    }

    #[test]
    fn the_server_answer_picks_the_message() {
        assert_eq!(authorize_state(&Ok(true)), AuthorizeState::Done);
        assert_eq!(authorize_state(&Ok(false)), AuthorizeState::WrongCode);
        assert_eq!(
            authorize_state(&Err(CoreError::QuickConnectDisabled)),
            AuthorizeState::Unavailable
        );
        assert_eq!(
            authorize_state(&Err(CoreError::Network("down".into()))),
            AuthorizeState::Failed
        );
        assert_eq!(
            authorize_state(&Err(CoreError::Server { status: 500 })),
            AuthorizeState::Failed
        );
    }

    #[test]
    fn subtitle_choices_follow_the_order_of_the_chips() {
        let style = SubtitleStyle {
            size: SubtitleSize::ExtraLarge,
            color: SubtitleColor::Yellow,
            background: SubtitleBackground::Box,
            position: SubtitlePosition::Raised,
        };
        let choice = subtitle_choice(style);
        assert_eq!(
            (
                choice.size,
                choice.color,
                choice.background,
                choice.position
            ),
            (3, 1, 1, 1)
        );
        assert_eq!(subtitle_style(&choice), style);
        assert_eq!(
            subtitle_choice(SubtitleStyle::default()),
            SubtitleChoice {
                size: 1,
                ..SubtitleChoice::default()
            }
        );
    }

    #[test]
    fn playback_choices_follow_the_order_of_the_chips() {
        let defaults = playback_choice(PlaybackPrefs::default());
        assert_eq!(
            defaults,
            PlaybackChoice {
                autoplay: 1,
                countdown: 1,
                intros: 0,
                recaps: 0,
                tracks: 1,
            }
        );
        assert_eq!(
            playback_prefs(&defaults, PlaybackPrefs::default()),
            PlaybackPrefs::default()
        );
        let prefs = PlaybackPrefs {
            autoplay_next: false,
            countdown: std::time::Duration::from_secs(30),
            skip_intros: true,
            skip_recaps: true,
            remember_tracks: false,
        };
        let choice = playback_choice(prefs);
        assert_eq!(choice.countdown, 3);
        assert_eq!(playback_prefs(&choice, PlaybackPrefs::default()), prefs);
        let odd = PlaybackChoice {
            countdown: 7,
            ..choice
        };
        assert_eq!(playback_prefs(&odd, prefs).countdown, prefs.countdown);
    }

    #[test]
    fn out_of_range_choices_pick_the_default() {
        let choice = SubtitleChoice {
            size: 9,
            color: -1,
            background: 2,
            position: 1,
        };
        assert_eq!(
            subtitle_style(&choice),
            SubtitleStyle {
                position: SubtitlePosition::Raised,
                ..SubtitleStyle::default()
            }
        );
    }
}
