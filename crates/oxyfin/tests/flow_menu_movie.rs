//! The context menu of a movie in a library: its rows, a watched change shown in place, Details.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use common::menu::{choose, is_open, right_click, wait_menu};
use common::{
    click, fixture, json, mount_browse_basics, run, save_session, start_with, wait_role, wait_until,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;
use slint::Model;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
const TITLE: &str = "Test Movie (2020)";

/// Serves `body` with the played flag of the movie as the server holds it.
struct Played {
    body: String,
    played: Arc<AtomicBool>,
}

impl Respond for Played {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let mut body: serde_json::Value = serde_json::from_str(&self.body).unwrap_or_default();
        let played = self.played.load(Ordering::SeqCst);
        let mark = |item: &mut serde_json::Value| {
            if item["Id"] == MOVIE {
                item["UserData"]["Played"] = played.into();
                if played {
                    item["UserData"]["PlaybackPositionTicks"] = 0.into();
                }
            }
        };
        match body["Items"].as_array_mut() {
            Some(items) => items.iter_mut().for_each(mark),
            None => mark(&mut body),
        }
        json(body.to_string())
    }
}

/// Marks the movie played, as the server does.
struct MarkPlayed {
    played: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}

impl Respond for MarkPlayed {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.played.store(true, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
        json(r#"{"PlaybackPositionTicks":0,"PlayCount":1,"IsFavorite":true,"Played":true}"#)
    }
}

#[test]
fn a_movie_menu_marks_it_watched_in_place() {
    let played = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let posted = Arc::clone(&calls);
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, &fixture("shows_next_up")).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(Played {
                    body: fixture("items_movies"),
                    played: Arc::clone(&played),
                })
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Items/{MOVIE}")))
                .respond_with(Played {
                    body: fixture("item_movie"),
                    played: Arc::clone(&played),
                })
                .mount(&mock)
                .await;
            Mock::given(method("POST"))
                .and(path(format!("/UserPlayedItems/{MOVIE}")))
                .respond_with(MarkPlayed {
                    played,
                    calls: posted,
                })
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

        right_click(&window, &card);
        let rows = wait_menu(&window, TITLE).await?;
        let expected = [
            "Resume from 0:03",
            "Play from the beginning",
            "Mark as watched",
            "Remove from favorites",
            "Details",
        ];
        if rows != expected {
            return Err(format!("rows {rows:?}, expected {expected:?}"));
        }

        choose(&window, "Mark as watched")?;
        if is_open(&window) {
            return Err("the menu stays open after a row".into());
        }
        let watched = || {
            let cards = window.get_library_cards();
            (0..cards.row_count())
                .filter_map(|index| cards.row_data(index))
                .any(|card| card.id == MOVIE && card.watched)
        };
        wait_until("the watched badge on the card", watched).await?;

        let card = wait_role(&window, TITLE, AccessibleRole::Button).await?;
        right_click(&window, &card);
        let rows = wait_menu(&window, TITLE).await?;
        if rows.first().map(String::as_str) != Some("Play")
            || !rows.iter().any(|row| row == "Mark as unwatched")
        {
            return Err(format!("the menu does not follow the change: {rows:?}"));
        }
        choose(&window, "Details")?;
        wait_until("the movie page", || window.get_screen() == Screen::Movie).await?;
        Ok(())
    });

    assert_eq!(calls.load(Ordering::SeqCst), 1, "one played request");
}
