//! The subtitle style is picked with the chips of the settings and saved at
//! once.

mod common;

use std::sync::{Arc, Mutex};

use common::{click, mount_browse_basics, run, save_session, start_with, wait_role, wait_until};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use oxyfin_core::{Config, Paths, SubtitleBackground, SubtitleSize, SubtitleStyle};
use slint::{ComponentHandle, LogicalSize};
use wiremock::MockServer;

const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

type Shared = Arc<Mutex<Option<Paths>>>;

fn saved(paths: &Shared) -> Option<SubtitleStyle> {
    let paths = paths.lock().ok()?.clone()?;
    Config::load_or_create(&paths)
        .ok()
        .map(|config| config.subtitles)
}

fn checked(window: &AppWindow, label: &str) -> bool {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .and_then(|element| element.accessible_checked())
        .unwrap_or(false)
}

/// Starts signed in and returns the flow with the directories the app writes to.
fn start() -> (common::Flow, Shared) {
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
        .set_size(LogicalSize::new(1280., 1000.));
    (flow, shared)
}

async fn open_settings(window: &AppWindow) -> Result<(), String> {
    wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
    let settings = wait_role(window, "Settings", AccessibleRole::Button).await?;
    click(window, &settings);
    wait_until("the settings", || window.get_screen() == Screen::Settings).await
}

#[test]
fn picking_chips_saves_the_subtitle_style() {
    let (flow, paths) = start();
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        open_settings(&window).await?;
        if !checked(&window, "Normal") || !checked(&window, "Outline") {
            return Err("the default style is not selected".into());
        }

        let large = wait_role(&window, "Large", AccessibleRole::Button).await?;
        click(&window, &large);
        wait_until("the large size saved", || {
            saved(&paths).is_some_and(|style| style.size == SubtitleSize::Large)
        })
        .await?;
        wait_until("the large chip selected", || checked(&window, "Large")).await?;
        if checked(&window, "Normal") {
            return Err("the normal chip stayed selected".into());
        }

        let boxed = wait_role(&window, "Box", AccessibleRole::Button).await?;
        click(&window, &boxed);
        wait_until("the box saved", || {
            saved(&paths)
                == Some(SubtitleStyle {
                    size: SubtitleSize::Large,
                    background: SubtitleBackground::Box,
                    ..SubtitleStyle::default()
                })
        })
        .await?;
        wait_until("the box chip selected", || checked(&window, "Box")).await
    });
}
