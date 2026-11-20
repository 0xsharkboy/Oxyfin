//! Discord shows what plays only once the user turns it on in the settings.

mod common;

use common::playback_settings::{Shared, chips, open_settings};
use common::{click, wait_until};
use i_slint_backend_testing::ElementHandle;
use oxyfin::AppWindow;
use oxyfin_core::Config;

fn config(paths: &Shared) -> Option<Config> {
    let paths = paths.lock().ok()?.clone()?;
    Config::load_or_create(&paths).ok()
}

fn saved(paths: &Shared) -> Option<bool> {
    config(paths).map(|config| config.discord)
}

fn fullscreen(paths: &Shared) -> Option<bool> {
    config(paths).map(|config| config.start_fullscreen)
}

/// The chip labelled `label` on the row of the Discord setting.
fn discord_chip(window: &AppWindow, label: &str) -> Result<ElementHandle, String> {
    let row = ElementHandle::find_by_accessible_label(window, "Discord")
        .find(|element| element.size().height > 0.)
        .ok_or("no Discord setting")?;
    let top = row.absolute_position().y;
    chips(window, label)
        .into_iter()
        .find(|chip| {
            let y = chip.absolute_position().y;
            y >= top - 8. && y <= top + 40.
        })
        .ok_or_else(|| format!("no {label} chip next to the Discord setting"))
}

fn checked(window: &AppWindow, label: &str) -> bool {
    discord_chip(window, label)
        .ok()
        .and_then(|chip| chip.accessible_checked())
        .unwrap_or(false)
}

// One app per test binary: the testing backend can be set up only once per process.
#[test]
fn discord_setting_off_by_default_and_saves() {
    open_settings(async |window, paths| {
        wait_until("the Discord setting", || {
            discord_chip(window, "Off").is_ok()
        })
        .await?;
        if !checked(window, "Off") || checked(window, "On") {
            return Err("Discord is not off by default".into());
        }
        if saved(paths) != Some(false) {
            return Err(format!("Discord saved as {:?}", saved(paths)));
        }
        let hint =
            "Shows what you watch on your Discord profile. Your server address is never shared.";
        if ElementHandle::find_by_accessible_label(window, hint)
            .next()
            .is_none()
        {
            return Err("the Discord hint is missing".into());
        }

        click(window, &discord_chip(window, "On")?);
        wait_until("Discord saved on", || saved(paths) == Some(true)).await?;
        wait_until("the on chip selected", || checked(window, "On")).await?;
        if fullscreen(paths) != Some(false) {
            return Err("the Discord chip changed the fullscreen setting".into());
        }
        click(window, &discord_chip(window, "Off")?);
        wait_until("Discord saved off", || saved(paths) == Some(false)).await?;
        wait_until("the off chip selected", || checked(window, "Off")).await
    });
}
