//! Fullscreen at launch: a window launched fullscreen stays so across the
//! screens, and the setting is saved from the settings for the next launch.

mod common;

use std::sync::{Arc, Mutex};

use common::{click, mount_browse_basics, run, save_session, start_with, wait_role, wait_until};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use oxyfin_core::{Config, Paths};
use slint::{ComponentHandle, LogicalSize};
use wiremock::MockServer;

const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

type Shared = Arc<Mutex<Option<Paths>>>;

fn saved(paths: &Shared) -> Option<bool> {
    let paths = paths.lock().ok()?.clone()?;
    Config::load_or_create(&paths)
        .ok()
        .map(|config| config.start_fullscreen)
}

fn checked(window: &AppWindow, label: &str) -> bool {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .and_then(|element| element.accessible_checked())
        .unwrap_or(false)
}

#[test]
fn a_fullscreen_launch_stays_fullscreen_and_the_setting_is_saved() {
    let shared: Shared = Arc::new(Mutex::new(None));
    let keep = Arc::clone(&shared);
    let flow = start_with(
        |mock: MockServer| async move {
            mount_browse_basics(&mock, EMPTY).await;
            mock
        },
        move |paths, vault, url| {
            save_session(paths, vault, url);
            if let Ok(mut slot) = keep.lock() {
                *slot = Some(paths.clone());
            }
        },
    );
    flow.window
        .window()
        .set_size(LogicalSize::new(1280., 1400.));
    flow.window.window().set_fullscreen(true);
    let window = flow.window.as_weak();
    let paths = shared;

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let settings = wait_role(&window, "Settings", AccessibleRole::Button).await?;
        click(&window, &settings);
        wait_until("the settings", || window.get_screen() == Screen::Settings).await?;
        if !window.window().is_fullscreen() {
            return Err("showing a screen left fullscreen".into());
        }

        if !checked(&window, "Off") || checked(&window, "On") {
            return Err("fullscreen at launch is not off by default".into());
        }
        let on = wait_role(&window, "On", AccessibleRole::Button).await?;
        click(&window, &on);
        wait_until("the setting saved", || saved(&paths) == Some(true)).await?;
        wait_until("the on chip selected", || checked(&window, "On")).await?;
        let off = wait_role(&window, "Off", AccessibleRole::Button).await?;
        click(&window, &off);
        wait_until("the setting saved off", || saved(&paths) == Some(false)).await?;
        if !window.window().is_fullscreen() {
            return Err("the setting changed the window before the next launch".into());
        }

        window.invoke_close_settings();
        wait_until("the home screen again", || {
            window.get_screen() == Screen::Home
        })
        .await?;
        if !window.window().is_fullscreen() {
            return Err("going back left fullscreen".into());
        }
        Ok(())
    });
}
