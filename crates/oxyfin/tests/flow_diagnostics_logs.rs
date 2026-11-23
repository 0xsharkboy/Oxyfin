//! Settings > Diagnostics: "Copy logs" puts a header and the recent log, scrubbed and without
//! the server host, on the clipboard, "Open folder" opens the log folder, and "Show" opens the
//! keyboard shortcuts.

mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use common::playback_settings::{WIDTH, open_settings_prepared};
use common::{click, wait_role, wait_until};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::logs::{LOG_FILE, RECENT_LOG_BYTES, log_dir};
use oxyfin::{AppWindow, Shortcuts};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize};

fn press(window: &AppWindow, text: impl Into<slint::SharedString>) {
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
}

fn release(window: &AppWindow, text: impl Into<slint::SharedString>) {
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
}

/// Pastes the clipboard into the Quick Connect code field and returns what it then holds.
async fn paste(window: &AppWindow) -> Result<String, String> {
    let field = wait_role(window, "Quick Connect code", AccessibleRole::TextInput).await?;
    click(window, &field);
    press(window, Key::Control);
    press(window, "v");
    release(window, "v");
    release(window, Key::Control);
    Ok(window.get_settings_authorize_code().to_string())
}

fn button(window: &AppWindow, label: &str) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
}

// One app per test binary: the testing backend can be set up only once per process.
#[test]
fn logs_are_copied_scrubbed_and_their_folder_opens() {
    let opened: Rc<RefCell<Vec<PathBuf>>> = Rc::default();
    let record = Rc::clone(&opened);
    open_settings_prepared(
        Vec::new(),
        |paths| {
            let dir = log_dir(paths);
            std::fs::create_dir_all(&dir).unwrap();
            let old = "INFO oxyfin: an old line\n".repeat(4 * RECENT_LOG_BYTES / 24);
            std::fs::write(
                dir.join(LOG_FILE),
                old + "INFO oxyfin: hello from the log GET http://127.0.0.1/Items?api_key=s3cr3t&x=1 not http://other.example.net/x nor other.example.net",
            )
            .unwrap();
        },
        move |app| app.set_folder_opener(move |dir| record.borrow_mut().push(dir.to_path_buf())),
        async move |window, paths| {
            window.window().set_size(LogicalSize::new(WIDTH, 2600.));
            let dir = log_dir(
                &paths
                    .lock()
                    .map_err(|_| "paths poisoned")?
                    .clone()
                    .ok_or("no paths")?,
            );

            let copy = wait_role(window, "Copy logs", AccessibleRole::Button).await?;
            click(window, &copy);
            wait_until("the copied label", || button(window, "Copied").is_some()).await?;
            let pasted = paste(window).await?;
            if !pasted.contains("hello from the log") {
                return Err(format!("the clipboard holds {pasted:?}"));
            }
            if pasted.contains("s3cr3t") || !pasted.contains("api_key=<redacted>") {
                return Err(format!("the copied log is not scrubbed: {pasted:?}"));
            }
            // The single-line field turns the line breaks of the header into spaces.
            let header = format!("oxyfin {} System: ", env!("CARGO_PKG_VERSION"));
            if !pasted.starts_with(&header) || !pasted.contains(" mpv: ") {
                return Err(format!(
                    "no version header: {:?}",
                    pasted.chars().take(80).collect::<String>()
                ));
            }
            if pasted.contains("127.0.0.1") || !pasted.contains("GET http://<server>/Items") {
                return Err("the server host is in the copied log".into());
            }
            if pasted.contains("example.net")
                || !pasted.contains("not http://<server>/x nor <server>")
            {
                return Err(format!("another server is in the copied log: {pasted:?}"));
            }
            if pasted.chars().count() >= 65_536 {
                return Err(format!(
                    "{} characters do not fit an issue",
                    pasted.chars().count()
                ));
            }
            if ElementHandle::find_by_accessible_label(window, "hello from the log")
                .next()
                .is_some()
            {
                return Err("the copied log stays in the element tree".into());
            }

            let open = wait_role(window, "Open folder", AccessibleRole::Button).await?;
            click(window, &open);
            wait_until("the folder opened", || opened.borrow().len() == 1).await?;
            if opened.borrow()[0] != dir {
                return Err(format!(
                    "opened {:?} instead of {dir:?}",
                    opened.borrow()[0]
                ));
            }

            let show = wait_role(window, "Show", AccessibleRole::Button).await?;
            click(window, &show);
            wait_until("the shortcuts", || window.global::<Shortcuts>().get_open()).await
        },
    );
}
