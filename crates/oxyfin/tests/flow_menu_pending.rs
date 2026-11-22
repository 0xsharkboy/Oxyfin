//! The context menu shows at once, headed with the card, while its rows are still asked for;
//! a click outside closes it then, and the late answer opens nothing.

mod common;

use std::time::Duration;

use common::menu::{is_open, right_click, wait_menu};
use common::{
    click, click_at, fixture, json, mount_browse_basics, pause, run, save_session, start_with,
    wait_role, wait_until,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use slint::{ComponentHandle, LogicalPosition};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
const TITLE: &str = "Test Movie (2020)";
const ANSWER: Duration = Duration::from_millis(800);

async fn mount(mock: MockServer) -> MockServer {
    mount_browse_basics(&mock, &fixture("shows_next_up")).await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .respond_with(json(fixture("items_movies")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{MOVIE}")))
        .respond_with(json(fixture("item_movie")).set_delay(ANSWER))
        .mount(&mock)
        .await;
    mock
}

fn panel(window: &AppWindow) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, TITLE).find(|element| {
        element.accessible_role() == Some(AccessibleRole::List)
            && element.size().width > 0.
            && element.size().height > 0.
    })
}

#[test]
fn the_menu_shows_at_once_and_closes_before_its_rows_come() {
    let flow = start_with(mount, save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let tab = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &tab);
        let card = wait_role(&window, TITLE, AccessibleRole::Button).await?;

        right_click(&window, &card);
        pause(Duration::from_millis(50)).await;
        let shown = panel(&window).ok_or("nothing shows while the rows are asked for")?;
        let rows = shown
            .query_descendants()
            .match_accessible_role(AccessibleRole::Button)
            .find_all();
        if !rows.is_empty() {
            return Err("placeholder rows can be chosen".into());
        }

        click_at(&window, LogicalPosition::new(1200., 700.));
        if is_open(&window) || panel(&window).is_some() {
            return Err("a click outside leaves the waiting menu open".into());
        }
        if window.get_screen() != Screen::Library {
            return Err("the click closing the menu reached the page".into());
        }
        pause(ANSWER + Duration::from_millis(300)).await;
        if is_open(&window) || panel(&window).is_some() {
            return Err("the late answer opened the menu again".into());
        }

        right_click(&window, &card);
        let rows = wait_menu(&window, TITLE).await?;
        if rows.first().map(String::as_str) != Some("Resume from 0:03") {
            return Err(format!("rows once answered: {rows:?}"));
        }
        Ok(())
    });
}
