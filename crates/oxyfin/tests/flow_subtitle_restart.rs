//! A subtitle style saved by an earlier run is selected in the settings.

mod common;

use common::{click, mount_browse_basics, run, save_session, start_with, wait_role, wait_until};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use oxyfin_core::{Config, SubtitleColor, SubtitlePosition, SubtitleStyle};
use slint::{ComponentHandle, LogicalSize};
use wiremock::MockServer;

const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

fn checked(window: &AppWindow, label: &str) -> bool {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .and_then(|element| element.accessible_checked())
        .unwrap_or(false)
}

#[test]
fn the_saved_subtitle_style_is_shown_after_a_restart() {
    let flow = start_with(
        |mock: MockServer| async move {
            mount_browse_basics(&mock, EMPTY).await;
            mock
        },
        |paths, vault, url| {
            save_session(paths, vault, url);
            let mut config =
                Config::load_or_create(paths).unwrap_or_else(|err| panic!("cannot load: {err}"));
            config.subtitles = SubtitleStyle {
                color: SubtitleColor::Yellow,
                position: SubtitlePosition::Raised,
                ..SubtitleStyle::default()
            };
            config
                .save(paths)
                .unwrap_or_else(|err| panic!("cannot save: {err}"));
        },
    );
    flow.window
        .window()
        .set_size(LogicalSize::new(1280., 1600.));
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let settings = wait_role(&window, "Settings", AccessibleRole::Button).await?;
        click(&window, &settings);
        wait_until("the settings", || window.get_screen() == Screen::Settings).await?;
        for label in ["Normal", "Yellow", "Outline", "Raised"] {
            wait_until(&format!("{label} selected"), || checked(&window, label)).await?;
        }
        for label in ["White", "Bottom"] {
            if checked(&window, label) {
                return Err(format!("{label} is selected"));
            }
        }
        Ok(())
    });
}
