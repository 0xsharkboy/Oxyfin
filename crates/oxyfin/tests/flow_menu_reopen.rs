//! A right click on another card while a context menu is open opens the menu of that card.

mod common;

use common::menu::{right_click, wait_menu};
use common::{
    click, fixture, json, mount_browse_basics, run, save_session, start_with, wait_role, wait_until,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{ContextMenu, Screen};
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

const FIRST: &str = "76282b1c408f374beeee50519e9157a4";
const SECOND: &str = "b4c9a0c1d2e34f5a8b6c7d8e9f0a1b2c";

async fn mount(mock: MockServer) -> MockServer {
    mount_browse_basics(&mock, &fixture("shows_next_up")).await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .respond_with(json(fixture("items_movies")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{FIRST}")))
        .respond_with(json(fixture("item_movie")))
        .mount(&mock)
        .await;
    let mut second: serde_json::Value =
        serde_json::from_str(&fixture("items_movies")).unwrap_or_default();
    let second = second["Items"]
        .as_array_mut()
        .and_then(|items| items.iter().find(|item| item["Id"] == SECOND).cloned())
        .unwrap_or_default();
    Mock::given(method("GET"))
        .and(path(format!("/Items/{SECOND}")))
        .respond_with(json(second.to_string()))
        .mount(&mock)
        .await;
    mock
}

#[test]
fn a_right_click_on_another_card_opens_its_menu() {
    let flow = start_with(mount, save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let tab = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &tab);
        let first = wait_role(&window, "Test Movie (2020)", AccessibleRole::Button).await?;
        let second = wait_role(&window, "Second Feature (2021)", AccessibleRole::Button).await?;

        // The second menu opens right of the second card, clear of the first one.
        right_click(&window, &second);
        wait_menu(&window, "Second Feature (2021)").await?;
        right_click(&window, &first);
        wait_menu(&window, "Test Movie (2020)").await?;
        let menu = window.global::<ContextMenu>();
        if menu.get_id() != FIRST {
            return Err(format!("the open menu is about {}", menu.get_id()));
        }
        Ok(())
    });
}
