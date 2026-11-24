//! The mouse back button pressed over a card, an episode row, a person, a season or a button of a
//! page goes back and activates nothing.

mod common;

use common::{
    click, fixture, json, mount_browse_basics, run, save_session, start_with, wait_role,
    wait_until, wait_visible,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{AppWindow, Screen};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";

async fn mount(mock: MockServer) -> MockServer {
    mount_browse_basics(&mock, &fixture("next_up_series")).await;
    let similar = r#"{"Items":[
        {"Id":"b4c9a0c1d2e34f5a8b6c7d8e9f0a1b2c","Name":"Second Feature","Type":"Movie"}
    ],"TotalRecordCount":1,"StartIndex":0}"#;
    for (route, body) in [
        ("/Items".to_owned(), fixture("items_shows")),
        (format!("/Items/{SERIES}"), fixture("item_series")),
        (format!("/Items/{SERIES}/Similar"), similar.to_owned()),
        (format!("/Shows/{SERIES}/Seasons"), fixture("seasons")),
        (
            format!("/Shows/{SERIES}/Episodes"),
            fixture("episodes_season_2"),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body))
            .mount(&mock)
            .await;
    }
    mock
}

/// Presses and releases the back button of the mouse over the button `label`.
async fn mouse_back(window: &AppWindow, label: &str) -> Result<(), String> {
    let element = wait_role(window, label, AccessibleRole::Button).await?;
    let (at, size) = (element.absolute_position(), element.size());
    let position = LogicalPosition::new(at.x + size.width / 2., at.y + size.height / 2.);
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
    Ok(())
}

#[test]
fn the_mouse_back_button_goes_back_over_what_can_be_clicked() {
    let flow = start_with(mount, save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        // Tall enough to show the whole page.
        window.window().set_size(LogicalSize::new(1280., 3000.));
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let shows = wait_role(&window, "Shows", AccessibleRole::Tab).await?;
        click(&window, &shows);
        for label in [
            "Second Feature",
            "The Lighthouse",
            "Gia Gate",
            "Season 1",
            "Mark as watched",
            "Back",
        ] {
            let card = wait_role(&window, "Test Show", AccessibleRole::Button).await?;
            click(&window, &card);
            wait_until("the series", || window.get_screen() == Screen::Series).await?;
            wait_visible(&window, "A generated test show.").await?;
            mouse_back(&window, label).await?;
            wait_until(label, || window.get_screen() == Screen::Library).await?;
        }
        // The library is the root: nothing to go back to, and the card does not open.
        mouse_back(&window, "Test Show").await?;
        if window.get_screen() != Screen::Library {
            return Err("the card opened".into());
        }
        Ok(())
    });
}
