//! A change the server refuses from the context menu shows a notice and changes nothing.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::menu::{choose, right_click, wait_menu};
use common::{
    click, fixture, json, mount_browse_basics, pause, run, save_session, start_with, wait_role,
    wait_until,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{Message, Screen};
use slint::ComponentHandle;
use slint::Model;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
const TITLE: &str = "Test Movie (2020)";

struct Counted {
    calls: Arc<AtomicUsize>,
    response: ResponseTemplate,
}

impl Respond for Counted {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.response.clone()
    }
}

#[test]
fn a_refused_change_shows_a_notice() {
    let listings = Arc::new(AtomicUsize::new(0));
    let listed = Arc::clone(&listings);
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, &fixture("shows_next_up")).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(Counted {
                    calls: listed,
                    response: json(fixture("items_movies")),
                })
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Items/{MOVIE}")))
                .respond_with(json(fixture("item_movie")))
                .mount(&mock)
                .await;
            Mock::given(method("POST"))
                .and(path(format!("/UserPlayedItems/{MOVIE}")))
                .respond_with(ResponseTemplate::new(403))
                .mount(&mock)
                .await;
            mock
        },
        save_session,
    );
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let tab = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &tab);
        let card = wait_role(&window, TITLE, AccessibleRole::Button).await?;
        let before = listings.load(Ordering::SeqCst);

        right_click(&window, &card);
        wait_menu(&window, TITLE).await?;
        choose(&window, "Mark as watched")?;
        wait_until("the notice", || window.get_notice().visible).await?;
        let notice = window.get_notice();
        if notice.message != Message::ServerError {
            return Err(format!("notice {:?}", notice.message));
        }
        pause(Duration::from_millis(200)).await;
        let cards = window.get_library_cards();
        let watched = (0..cards.row_count())
            .filter_map(|index| cards.row_data(index))
            .any(|card| card.id == MOVIE && card.watched);
        if watched {
            return Err("the card shows a change the server refused".into());
        }
        if listings.load(Ordering::SeqCst) != before {
            return Err("the library reloaded after a refused change".into());
        }
        Ok(())
    });
}
