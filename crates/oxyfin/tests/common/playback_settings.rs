//! Helpers of the playback settings flows: the settings screen opened signed
//! in, its chips and the choices saved.

use std::sync::{Arc, Mutex};

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{App, AppWindow, Screen};
use oxyfin_core::{Config, Paths, PlaybackPrefs};
use slint::{ComponentHandle, LogicalSize};
use wiremock::{Mock, MockServer};

use super::{click, mount_browse_basics, run, save_session, start_with, wait_role, wait_until};

const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;
/// Width of the window.
pub const WIDTH: f32 = 1280.;

/// Where the configuration of the run lives, once the app started.
pub type Shared = Arc<Mutex<Option<Paths>>>;

/// The playback choices saved.
pub fn saved(paths: &Shared) -> Option<PlaybackPrefs> {
    let paths = paths.lock().ok()?.clone()?;
    Config::load_or_create(&paths)
        .ok()
        .map(|config| config.playback)
}
/// The visible chips labelled `label`, in the order of the rows.
pub fn chips(window: &AppWindow, label: &str) -> Vec<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label)
        .filter(|element| {
            let size = element.size();
            element.accessible_role() == Some(AccessibleRole::Button)
                && size.width > 0.
                && size.height > 0.
        })
        .collect()
}

pub fn chip(window: &AppWindow, label: &str, nth: usize) -> Result<ElementHandle, String> {
    chips(window, label)
        .into_iter()
        .nth(nth)
        .ok_or_else(|| format!("no chip {label} number {nth}"))
}

pub fn checked(window: &AppWindow, label: &str, nth: usize) -> bool {
    chip(window, label, nth)
        .ok()
        .and_then(|element| element.accessible_checked())
        .unwrap_or(false)
}

pub fn enabled(window: &AppWindow, label: &str) -> bool {
    chip(window, label, 0)
        .ok()
        .and_then(|element| element.accessible_enabled())
        .unwrap_or(false)
}

/// Starts signed in, opens the settings and runs `scenario` there.
pub fn open_settings(
    scenario: impl AsyncFnOnce(&AppWindow, &Shared) -> Result<(), String> + 'static,
) {
    open_settings_with(Vec::new(), scenario);
}

/// Like [`open_settings`], with `mocks` mounted on the simulated server as well.
pub fn open_settings_with(
    mocks: Vec<Mock>,
    scenario: impl AsyncFnOnce(&AppWindow, &Shared) -> Result<(), String> + 'static,
) {
    open_settings_prepared(mocks, |_| {}, |_| {}, scenario);
}

/// Like [`open_settings_with`], with `prepare` filling the directories before the app starts
/// and `setup` adjusting the app before the scenario runs.
pub fn open_settings_prepared(
    mocks: Vec<Mock>,
    prepare: impl FnOnce(&Paths),
    setup: impl FnOnce(&App),
    scenario: impl AsyncFnOnce(&AppWindow, &Shared) -> Result<(), String> + 'static,
) {
    let shared: Shared = Arc::new(Mutex::new(None));
    let keep = Arc::clone(&shared);
    let flow = start_with(
        |mock: MockServer| async move {
            for extra in mocks {
                mock.register(extra).await;
            }
            mount_browse_basics(&mock, EMPTY).await;
            mock
        },
        move |paths, vault, url| {
            save_session(paths, vault, url);
            prepare(paths);
            if let Ok(mut slot) = keep.lock() {
                *slot = Some(paths.clone());
            }
        },
    );
    setup(&flow.app);
    flow.window
        .window()
        .set_size(LogicalSize::new(WIDTH, 1800.));
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let settings = wait_role(&window, "Settings", AccessibleRole::Button).await?;
        click(&window, &settings);
        wait_until("the settings", || window.get_screen() == Screen::Settings).await?;
        scenario(&window, &shared).await
    });
}
