//! Settings screen: the signed-in account, the subtitle style and the
//! application version.

use oxyfin_core::{
    SubtitleBackground, SubtitleColor, SubtitlePosition, SubtitleSize, SubtitleStyle,
};

use crate::state::SessionInfo;
use crate::{AppWindow, SubtitleChoice};

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
    window.set_settings_token_in_file(session.is_some_and(|info| info.token_in_file));
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
            token_in_file: false,
            show_file_token_warning: false,
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
