//! The countdown before the next episode cannot be picked while the next
//! episode waits for the user.

mod common;

use std::time::Duration;

use common::playback_settings::{checked, chip, enabled, open_settings, saved};
use common::{click, pause, wait_until};

#[test]
fn countdown_hidden_in_ask_mode() {
    open_settings(async |window, paths| {
        if !enabled(window, "15 s") {
            return Err("the countdown is disabled while playing automatically".into());
        }
        click(window, &chip(window, "Ask", 0)?);
        wait_until("the countdown disabled", || !enabled(window, "15 s")).await?;
        click(window, &chip(window, "15 s", 0)?);
        pause(Duration::from_millis(100)).await;
        if saved(paths).map(|prefs| prefs.countdown) != Some(Duration::from_secs(10)) {
            return Err(format!(
                "a disabled chip changed the countdown: {:?}",
                saved(paths)
            ));
        }
        if !checked(window, "10 s", 0) {
            return Err("the countdown chosen is no longer shown".into());
        }
        click(window, &chip(window, "Automatically", 0)?);
        wait_until("the countdown enabled", || enabled(window, "15 s")).await
    });
}
