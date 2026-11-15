//! Settings screen: the signed-in account and the application version.

use crate::AppWindow;
use crate::state::SessionInfo;

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
}
