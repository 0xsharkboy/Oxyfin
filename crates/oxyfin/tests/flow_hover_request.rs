//! Hovering a home poster shows its details, fetched once for the session however often it is hovered.

mod common;

use std::time::Duration;

use common::{fixture, json, mount_me, pause, run, save_session, start_with, wait_until};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer};

const MOVIES: &str = "f137a2dd21bbc1b99aa5c0f6bf02a805";
const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

async fn mount_home(mock: MockServer) -> MockServer {
    mount_me(&mock).await;
    Mock::given(method("GET"))
        .and(path("/UserViews"))
        .respond_with(json(fixture("user_views")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items/Latest"))
        .and(query_param("parentId", MOVIES))
        .respond_with(json(
            fixture("items_latest").replace("Test Movie (2020)", "Fresh Arrival"),
        ))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items/Latest"))
        .respond_with(json("[]"))
        .mount(&mock)
        .await;
    for route in ["/UserItems/Resume", "/Shows/NextUp"] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(EMPTY))
            .mount(&mock)
            .await;
    }
    Mock::given(method("GET"))
        .and(path(format!("/Items/{MOVIE}")))
        .respond_with(json(fixture("item_movie")).set_delay(Duration::from_millis(1000)))
        .mount(&mock)
        .await;
    mock
}

fn find(window: &AppWindow, label: &str, role: AccessibleRole) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(window, label).find(|element| {
        let size = element.size();
        element.accessible_role() == Some(role) && size.width > 0. && size.height > 0.
    })
}

fn move_to(window: &AppWindow, x: f32, y: f32) {
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(x, y),
    });
}

fn hover_poster(window: &AppWindow) -> Result<(), String> {
    let poster =
        find(window, "Fresh Arrival", AccessibleRole::Button).ok_or("no Fresh Arrival poster")?;
    let at = poster.absolute_position();
    move_to(window, at.x + 20., at.y + 20.);
    Ok(())
}

/// Moves the pointer onto the empty side of the window, away from every card.
fn leave(window: &AppWindow) {
    move_to(window, 1200., 40.);
}

async fn item_requests(mock: &MockServer) -> usize {
    mock.received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path() == format!("/Items/{MOVIE}"))
        .count()
}

#[test]
fn a_hovered_poster_is_fetched_once() {
    let flow = start_with(mount_home, save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_until("the poster row", || {
            find(&window, "Fresh Arrival", AccessibleRole::Button).is_some()
        })
        .await?;

        hover_poster(&window)?;
        pause(Duration::from_millis(500)).await;
        leave(&window);
        pause(Duration::from_millis(50)).await;
        hover_poster(&window)?;
        wait_until("the hover card", || {
            find(&window, "Test Movie (2020)", AccessibleRole::Groupbox).is_some()
        })
        .await?;
        let card =
            find(&window, "Test Movie (2020)", AccessibleRole::Groupbox).ok_or("no hover card")?;
        let texts: Vec<String> = card
            .query_descendants()
            .match_accessible_role(AccessibleRole::Text)
            .find_all()
            .into_iter()
            .filter_map(|element| element.accessible_label().map(|label| label.to_string()))
            .collect();
        for text in ["Drama", "Comedy", "A generated test movie."] {
            if !texts.iter().any(|t| t == text) {
                return Err(format!("{text} is missing from {texts:?}"));
            }
        }

        leave(&window);
        wait_until("the card to hide", || {
            find(&window, "Test Movie (2020)", AccessibleRole::Groupbox).is_none()
        })
        .await?;
        hover_poster(&window)?;
        pause(Duration::from_millis(450)).await;
        wait_until("the card again", || {
            find(&window, "Test Movie (2020)", AccessibleRole::Groupbox).is_some()
        })
        .await?;
        Ok(())
    });

    let count = flow.runtime.block_on(item_requests(&flow.mock));
    assert_eq!(count, 1, "the movie is fetched once across three hovers");
}
