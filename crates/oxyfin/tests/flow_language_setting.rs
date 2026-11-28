//! Interface language: the settings switch it at once, the screen below shows its text in the
//! new language once back, and the choice is saved for the next launch.

mod common;

use std::sync::{Arc, Mutex};

use common::{
    click, mount_browse_basics, run, save_session, start_with, wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::{AccessibleRole, ElementQuery};
use oxyfin::{AppWindow, Screen};
use oxyfin_core::{Config, Paths};
use slint::{ComponentHandle, LogicalSize};
use wiremock::MockServer;

const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

type Shared = Arc<Mutex<Option<Paths>>>;

fn saved(paths: &Shared) -> Option<Option<String>> {
    let paths = paths.lock().ok()?.clone()?;
    Config::load_or_create(&paths)
        .ok()
        .map(|config| config.language)
}

async fn pick(window: &AppWindow, list: &str, option: &str) -> Result<(), String> {
    let dropdown = wait_role(window, list, AccessibleRole::Button).await?;
    click(window, &dropdown);
    let item = wait_role(window, option, AccessibleRole::ListItem).await?;
    let mut options: Vec<String> = Vec::new();
    for element in ElementQuery::from_root(window)
        .match_descendants()
        .match_accessible_role(AccessibleRole::ListItem)
        .find_all()
    {
        let label = element.accessible_label().unwrap_or_default().to_string();
        if !options.contains(&label) {
            options.push(label);
        }
    }
    if options.len() != 3 || options.iter().any(String::is_empty) {
        return Err(format!("the list offers {options:?}"));
    }
    click(window, &item);
    Ok(())
}

#[test]
fn the_interface_language_switches_at_once_and_is_saved() {
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
    flow.app.set_system_language(|| Some("de".to_owned()));
    flow.window
        .window()
        .set_size(LogicalSize::new(1280., 1400.));
    let window = flow.window.as_weak();
    let paths = shared;

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_visible(&window, "Continue Watching").await?;
        let settings = wait_role(&window, "Settings", AccessibleRole::Button).await?;
        click(&window, &settings);
        wait_until("the settings", || window.get_screen() == Screen::Settings).await?;

        // No translation for the system language: the default is English.
        pick(&window, "Language: System default (English)", "Français").await?;
        wait_visible(&window, "Paramètres").await?;
        wait_role(&window, "Langue: Français", AccessibleRole::Button).await?;
        wait_until("French saved", || {
            saved(&paths) == Some(Some("fr".to_owned()))
        })
        .await?;

        window.invoke_close_settings();
        wait_until("the home screen again", || {
            window.get_screen() == Screen::Home
        })
        .await?;
        wait_visible(&window, "Reprendre la lecture").await?;

        let settings = wait_role(&window, "Paramètres", AccessibleRole::Button).await?;
        click(&window, &settings);
        wait_until("the settings again", || {
            window.get_screen() == Screen::Settings
        })
        .await?;
        pick(&window, "Langue: Français", "English").await?;
        wait_visible(&window, "Settings").await?;
        wait_until("English saved", || {
            saved(&paths) == Some(Some("en".to_owned()))
        })
        .await?;
        pick(&window, "Language: English", "System default (English)").await?;
        wait_until("the system default saved", || saved(&paths) == Some(None)).await?;
        wait_role(
            &window,
            "Language: System default (English)",
            AccessibleRole::Button,
        )
        .await?;

        window.invoke_close_settings();
        wait_until("the home screen in English", || {
            window.get_screen() == Screen::Home
        })
        .await?;
        wait_visible(&window, "Continue Watching").await?;
        Ok(())
    });
}
