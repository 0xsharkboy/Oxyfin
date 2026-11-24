//! Hovering the card of a series loads its page, which then opens at once.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::answers::open_tab;
use common::{
    click_at, fixture, json, mount_browse_basics, pause, run, save_session, start_with, wait_role,
    wait_until,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";

struct Seasons(Arc<AtomicUsize>);

impl Respond for Seasons {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.0.fetch_add(1, Ordering::SeqCst);
        json(fixture("seasons"))
    }
}

async fn mount(mock: MockServer, asked: Arc<AtomicUsize>) -> MockServer {
    Mock::given(method("GET"))
        .and(path(format!("/Shows/{SERIES}/Seasons")))
        .respond_with(Seasons(asked))
        .mount(&mock)
        .await;
    for (route, body) in [
        ("/Items".to_owned(), fixture("items_shows")),
        (format!("/Items/{SERIES}"), fixture("item_series")),
        (
            format!("/Shows/{SERIES}/Episodes"),
            fixture("episodes_season_1"),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body))
            .mount(&mock)
            .await;
    }
    mount_browse_basics(&mock, &fixture("next_up_series")).await;
    mock
}

#[test]
fn a_hovered_series_opens_at_once() {
    let asked = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&asked);
    let flow = start_with(move |mock| mount(mock, count), save_session);
    flow.app.set_prefetch(true);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        open_tab(&window, "Shows").await?;
        let card = wait_role(&window, "Test Show", AccessibleRole::Button).await?;
        let at = card.absolute_position();
        let over = LogicalPosition::new(at.x + 30., at.y + 30.);
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: over });
        wait_until("the seasons", || asked.load(Ordering::SeqCst) == 1).await?;
        // The next episode, asked for at the same time, answers too.
        pause(Duration::from_millis(400)).await;
        click_at(&window, over);
        if window.get_screen() != Screen::Series || !window.get_series_loaded() {
            return Err("the series page waits for the server".into());
        }
        Ok(())
    });
}
