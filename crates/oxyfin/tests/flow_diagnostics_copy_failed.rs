//! Settings > Diagnostics: when the clipboard cannot be reached, "Copy logs" says so and points
//! to "Open folder" instead of claiming the logs were copied.

mod common;

use common::playback_settings::{WIDTH, open_settings_prepared};
use common::{click, wait_role, wait_until};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use slint::{ComponentHandle, LogicalSize};

const FAILED: &str =
    "The clipboard cannot be reached. Open the folder and attach oxyfin.log to the report.";

// One app per test binary: the testing backend can be set up only once per process.
#[test]
fn a_failed_copy_points_to_the_folder() {
    open_settings_prepared(
        Vec::new(),
        |_| {},
        |app| app.set_clipboard(|_, _| false),
        async move |window, _| {
            window.window().set_size(LogicalSize::new(WIDTH, 2600.));
            let copy = wait_role(window, "Copy logs", AccessibleRole::Button).await?;
            click(window, &copy);
            wait_until("the failure", || window.get_settings_logs_copy_failed()).await?;
            wait_role(window, FAILED, AccessibleRole::Text).await?;
            if ElementHandle::find_by_accessible_label(window, "Copied")
                .next()
                .is_some()
            {
                return Err("the button says the logs were copied".into());
            }
            wait_role(window, "Copy logs", AccessibleRole::Button).await?;
            wait_role(window, "Open folder", AccessibleRole::Button).await?;
            Ok(())
        },
    );
}
