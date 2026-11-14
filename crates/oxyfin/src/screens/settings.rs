//! Settings screen: the signed-in account and the application version.

use crate::AppWindow;
use crate::state::SessionInfo;

/// Version of the application, shown on the settings screen.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Fills the settings screen with the signed-in session, or clears it without one.
pub fn show(window: &AppWindow, session: Option<&SessionInfo>) {
    window.set_app_version(VERSION.into());
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
