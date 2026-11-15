//! Opens a detail page from the home screen, from a poster and from the banner, and comes back.

mod common;

use common::{
    click, exists, fixture, json, mount_me, run, save_session, start_with, wait_role, wait_until,
    wait_visible,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;
use slint::LogicalPosition;
use slint::platform::{PointerEventButton, WindowEvent};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer};

const MOVIES: &str = "f137a2dd21bbc1b99aa5c0f6bf02a805";
const SHOWS: &str = "a656b907eb3a73532e40e44b968d0225";
const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";
const LATEST_SHOW: &str = r#"[{"Name":"Fresh Show","Id":"9ea196e7ccc3ae17d2c3f75c3ab34406","ProductionYear":2021,"IsFolder":true,"Type":"Series","UserData":{"PlaybackPositionTicks":0,"PlayCount":0,"IsFavorite":false,"Played":false},"ImageTags":{},"BackdropImageTags":[]}]"#;
const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

async fn mount_home(mock: MockServer) -> MockServer {
    mount_me(&mock).await;
    Mock::given(method("GET"))
        .and(path("/UserViews"))
        .respond_with(json(fixture("user_views")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/UserItems/Resume"))
        .respond_with(json(fixture("user_items_resume")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Shows/NextUp"))
        .respond_with(json(EMPTY))
        .mount(&mock)
        .await;
    let arrival = fixture("items_latest").replace("Test Movie (2020)", "Fresh Arrival");
    Mock::given(method("GET"))
        .and(path("/Items/Latest"))
        .and(query_param("parentId", MOVIES))
        .respond_with(json(arrival))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items/Latest"))
        .and(query_param("parentId", SHOWS))
        .respond_with(json(LATEST_SHOW))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{MOVIE}")))
        .respond_with(json(fixture("item_movie")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{SERIES}")))
        .respond_with(json(fixture("item_series")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Shows/{SERIES}/Seasons")))
        .respond_with(json(fixture("seasons")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Shows/{SERIES}/Episodes")))
        .respond_with(json(fixture("episodes_season_2")))
        .mount(&mock)
        .await;
    mock
}

#[test]
fn home_posters_and_banner_open_the_details() {
    let flow = start_with(mount_home, save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_until("the poster row", || {
            slint::Model::iter(&window.get_home_rows())
                .any(|row| row.title == "Recently Added in Movies")
        })
        .await?;
        window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(640., 400.),
                delta_x: 0.,
                delta_y: -400.,
            });
        let poster = wait_visible(&window, "Fresh Arrival").await?;
        click(&window, &poster);
        wait_until("the movie", || window.get_screen() == Screen::Movie).await?;
        wait_visible(&window, "Drama").await?;
        let back = wait_role(&window, "Back", AccessibleRole::Button).await?;
        click(&window, &back);
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;

        let more = wait_role(&window, "More info", AccessibleRole::Button).await?;
        click(&window, &more);
        wait_until("the movie", || window.get_screen() == Screen::Movie).await?;
        wait_visible(&window, "Drama").await?;
        if exists(&window, "Fresh Arrival") {
            return Err("the home screen is still shown".into());
        }

        // The mouse back button, when the windowing system reports it.
        let position = LogicalPosition::new(1150., 300.);
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        for event in [
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
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;

        // A series poster opens the series page.
        window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(640., 400.),
                delta_x: 0.,
                delta_y: -1200.,
            });
        let poster = wait_visible(&window, "Fresh Show").await?;
        click(&window, &poster);
        wait_until("the series", || window.get_screen() == Screen::Series).await?;
        wait_visible(&window, "Mystery").await?;
        press_escape(&window);
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        Ok(())
    });
}

fn press_escape(window: &oxyfin::AppWindow) {
    let text: slint::SharedString = slint::platform::Key::Escape.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
