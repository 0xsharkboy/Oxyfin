//! Copied logs hide the server last typed at the sign-in, not the words typed on the way to it.

mod common;

use common::playback_settings::{WIDTH, open_settings_prepared};
use common::{click, wait_role, wait_until};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::AppWindow;
use oxyfin::logs::{LOG_FILE, log_dir};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize};

fn key(window: &AppWindow, text: impl Into<slint::SharedString>, pressed: bool) {
    let text = text.into();
    let event = if pressed {
        WindowEvent::KeyPressed { text }
    } else {
        WindowEvent::KeyReleased { text }
    };
    window.window().dispatch_event(event);
}

/// Pastes the clipboard into the Quick Connect code field and returns what it then holds.
async fn paste(window: &AppWindow) -> Result<String, String> {
    let field = wait_role(window, "Quick Connect code", AccessibleRole::TextInput).await?;
    click(window, &field);
    key(window, Key::Control, true);
    key(window, "v", true);
    key(window, "v", false);
    key(window, Key::Control, false);
    Ok(window.get_settings_authorize_code().to_string())
}

#[test]
fn only_the_last_typed_server_is_hidden() {
    open_settings_prepared(
        Vec::new(),
        |paths| {
            let dir = log_dir(paths);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join(LOG_FILE),
                "INFO oxyfin: plain http words, na na, reached nas.invalid now",
            )
            .unwrap();
        },
        |_| {},
        async move |window, _| {
            window.window().set_size(LogicalSize::new(WIDTH, 2600.));
            // Pauses while typing an address, each checked on its own.
            for typed in ["http", "na", "nas.invalid"] {
                window.invoke_server_confirmed(typed.into());
            }
            let copy = wait_role(window, "Copy logs", AccessibleRole::Button).await?;
            click(window, &copy);
            wait_until("the copied label", || {
                ElementHandle::find_by_accessible_label(window, "Copied")
                    .any(|element| element.accessible_role() == Some(AccessibleRole::Button))
            })
            .await?;
            let pasted = paste(window).await?;
            if !pasted.contains("plain http words, na na, reached <server> now") {
                return Err(format!("the clipboard holds {pasted:?}"));
            }
            Ok(())
        },
    );
}
