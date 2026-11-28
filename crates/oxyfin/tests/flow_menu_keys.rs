//! A context menu opened with the mouse leaves the keyboard where it was: once Escape closed it,
//! `?`, F11 and Escape work on the page again.

mod common;

use common::menu::{is_open, right_click, wait_menu};
use common::person::press_escape;
use common::{
    click, fixture, json, mount_browse_basics, pause, run, save_session, start_with, wait_role,
    wait_until,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen, Shortcuts};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use std::time::Duration;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";
const SEASON_1: &str = "0407e3d02b7d70abe4a2d37241b02fea";
const EPISODE: &str = "a2000000000000000000000000000005";

async fn mount(mock: MockServer) -> MockServer {
    mount_browse_basics(&mock, &fixture("next_up_series")).await;
    let get = |route: String, body: String| {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body))
    };
    get("/Items".into(), fixture("items_shows"))
        .mount(&mock)
        .await;
    get(format!("/Items/{SERIES}"), fixture("item_series"))
        .mount(&mock)
        .await;
    get(format!("/Shows/{SERIES}/Seasons"), fixture("seasons"))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Shows/{SERIES}/Episodes")))
        .and(query_param("seasonId", SEASON_1))
        .respond_with(json(fixture("episodes_season_1")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Shows/{SERIES}/Episodes")))
        .respond_with(json(fixture("episodes_season_2")))
        .mount(&mock)
        .await;
    let mut episodes: serde_json::Value =
        serde_json::from_str(&fixture("episodes_season_2")).unwrap_or_default();
    let episode = episodes["Items"]
        .as_array_mut()
        .and_then(|items| items.iter().find(|item| item["Id"] == EPISODE).cloned())
        .unwrap_or_default();
    get(format!("/Items/{EPISODE}"), episode.to_string())
        .mount(&mock)
        .await;
    mock
}

fn press(window: &AppWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

/// Waits for the episode row `title` inside the window, scrolling the page a little.
async fn wait_row(window: &AppWindow, title: &str) -> Result<ElementHandle, String> {
    let start = std::time::Instant::now();
    let bottom = window.window().size().height as f32 / window.window().scale_factor();
    loop {
        window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(640., 400.),
                delta_x: 0.,
                delta_y: -40.,
            });
        if let Some(row) = ElementHandle::find_by_accessible_label(window, title).find(|row| {
            row.size().height > 0.
                && row.absolute_position().y + row.size().height < bottom
                && row.accessible_role() == Some(AccessibleRole::Button)
        }) {
            return Ok(row);
        }
        if start.elapsed() > Duration::from_secs(5) {
            return Err(format!("timed out waiting for the row {title}"));
        }
        pause(Duration::from_millis(10)).await;
    }
}

#[test]
fn the_keys_work_again_after_a_menu_opened_with_the_mouse() {
    let flow = start_with(mount, save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let tab = wait_role(&window, "Shows", AccessibleRole::Tab).await?;
        click(&window, &tab);
        let card = wait_role(&window, "Test Show", AccessibleRole::Button).await?;
        click(&window, &card);
        wait_until("the series page", || window.get_screen() == Screen::Series).await?;
        let row = wait_row(&window, "The Pact").await?;

        right_click(&window, &row);
        wait_menu(&window, "Test Show").await?;
        press_escape(&window);
        if is_open(&window) {
            return Err("Escape leaves the menu open".into());
        }
        if window.get_screen() != Screen::Series {
            return Err("the Escape closing the menu also left the page".into());
        }

        press(&window, "?");
        if !window.global::<Shortcuts>().get_open() {
            return Err("? does nothing after the menu closed".into());
        }
        press_escape(&window);
        if window.global::<Shortcuts>().get_open() {
            return Err("Escape leaves the shortcuts open".into());
        }

        let fullscreen = window.window().is_fullscreen();
        press(&window, Key::F11);
        wait_until("F11 to toggle fullscreen", || {
            window.window().is_fullscreen() != fullscreen
        })
        .await?;
        press(&window, Key::F11);
        wait_until("F11 to toggle fullscreen back", || {
            window.window().is_fullscreen() == fullscreen
        })
        .await?;

        press_escape(&window);
        wait_until("Escape to go back to the library", || {
            window.get_screen() == Screen::Library
        })
        .await?;
        Ok(())
    });
}
