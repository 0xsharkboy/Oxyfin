//! The mouse back button pressed over the top bar goes back, as it does over the page.

mod common;

use common::{
    click, fixture, json, mount_browse_basics, run, save_session, start_with, wait_role, wait_until,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";

async fn mount(mock: MockServer) -> MockServer {
    mount_browse_basics(&mock, &fixture("shows_next_up")).await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .respond_with(json(fixture("items_movies")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{MOVIE}")))
        .respond_with(json(fixture("item_movie")))
        .mount(&mock)
        .await;
    mock
}

/// Presses and releases the back button of the mouse at `position`.
fn mouse_back(window: &AppWindow, position: LogicalPosition) {
    for event in [
        WindowEvent::PointerMoved { position },
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Back,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Back,
        },
    ] {
        window.window().dispatch_event(event);
    }
}

/// A point of the top bar between the Favorites tab and the search button.
fn on_the_bar(window: &AppWindow) -> Result<LogicalPosition, String> {
    let bar = ElementHandle::find_by_accessible_label(window, "Navigation")
        .find(|element| element.accessible_role() == Some(AccessibleRole::TabList))
        .ok_or("no top bar")?;
    let favorites = ElementHandle::find_by_accessible_label(window, "Favorites")
        .find(|element| element.accessible_role() == Some(AccessibleRole::Tab))
        .ok_or("no Favorites tab")?;
    let (at, size) = (bar.absolute_position(), bar.size());
    let after = favorites.absolute_position().x + favorites.size().width;
    Ok(LogicalPosition::new(
        after + (at.x + size.width - after) / 3.,
        at.y + size.height / 2.,
    ))
}

#[test]
fn the_mouse_back_button_over_the_top_bar_goes_back() {
    let flow = start_with(mount, save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let tab = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &tab);
        let card = wait_role(&window, "Test Movie (2020)", AccessibleRole::Button).await?;
        click(&window, &card);
        wait_until("the movie page", || window.get_screen() == Screen::Movie).await?;
        wait_role(&window, "Mark as watched", AccessibleRole::Button).await?;

        let point = on_the_bar(&window)?;
        mouse_back(&window, point);
        wait_until("the library after the back button", || {
            window.get_screen() == Screen::Library
        })
        .await?;

        // Over a tab or an icon the bar goes back too, and neither is activated.
        for (label, role) in [
            ("Shows", AccessibleRole::Tab),
            ("Settings", AccessibleRole::Button),
        ] {
            let card = wait_role(&window, "Test Movie (2020)", AccessibleRole::Button).await?;
            click(&window, &card);
            wait_until("the movie page", || window.get_screen() == Screen::Movie).await?;
            let control = wait_role(&window, label, role).await?;
            let (at, size) = (control.absolute_position(), control.size());
            mouse_back(
                &window,
                LogicalPosition::new(at.x + size.width / 2., at.y + size.height / 2.),
            );
            wait_until(label, || window.get_screen() == Screen::Library).await?;
            wait_role(&window, "Test Movie (2020)", AccessibleRole::Button).await?;
        }
        Ok(())
    });
}
