//! A change made on a card while the grid is being asked again is not undone by the answer asked
//! for before it.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::menu::{choose, right_click, wait_menu};
use common::person::press_escape;
use common::{
    click, click_at, json, mount_browse_basics, pause, run, save_session, start_with, wait_role,
    wait_until,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{AppWindow, Screen};
use slint::{ComponentHandle, LogicalPosition, Model};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;
/// How long the grid takes to answer: what it answers is what the server held when asked.
const SLOW: Duration = Duration::from_millis(700);

/// Ids of the movies the server holds played.
type Played = Arc<Mutex<HashSet<String>>>;

fn id(index: usize) -> String {
    format!("{index:032x}")
}

fn movie(index: usize, played: &Played) -> serde_json::Value {
    let watched = played
        .lock()
        .map(|played| played.contains(&id(index)))
        .unwrap_or(false);
    serde_json::json!({
        "Name": format!("Movie {index}"), "Id": id(index), "Type": "Movie",
        "IsFolder": false, "ProductionYear": 2020,
        "UserData": {"PlaybackPositionTicks": 0, "PlayCount": 0, "IsFavorite": false,
            "Played": watched}
    })
}

struct Items(Played);

impl Respond for Items {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let items: Vec<serde_json::Value> = (0..4).map(|index| movie(index, &self.0)).collect();
        json(
            serde_json::json!({"Items": items, "TotalRecordCount": 4, "StartIndex": 0}).to_string(),
        )
        .set_delay(SLOW)
    }
}

fn last_segment(request: &Request) -> String {
    let path = request.url.path();
    path.rsplit('/').next().unwrap_or("").to_owned()
}

struct Item(Played);

impl Respond for Item {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let wanted = last_segment(request);
        match (0..4).find(|index| id(*index) == wanted) {
            Some(index) => json(movie(index, &self.0).to_string()),
            None => ResponseTemplate::new(404),
        }
    }
}

struct MarkPlayed(Played);

impl Respond for MarkPlayed {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let item = last_segment(request);
        self.0.lock().map(|mut played| played.insert(item)).ok();
        json(r#"{"PlaybackPositionTicks":0,"PlayCount":1,"IsFavorite":false,"Played":true}"#)
    }
}

fn watched(window: &AppWindow, index: usize) -> bool {
    let cards = window.get_library_cards();
    (0..cards.row_count())
        .filter_map(|row| cards.row_data(row))
        .any(|card| card.id == id(index) && card.watched)
}

#[test]
fn a_change_survives_an_answer_asked_for_before_it() {
    let played: Played = Arc::default();
    let held = Arc::clone(&played);
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, EMPTY).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(Items(Arc::clone(&held)))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path_regex("^/Items/[0-9a-f]{32}$"))
                .respond_with(Item(Arc::clone(&held)))
                .mount(&mock)
                .await;
            Mock::given(method("POST"))
                .and(path_regex("^/UserPlayedItems/[0-9a-f]{32}$"))
                .respond_with(MarkPlayed(held))
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
        // A change on a detail page makes the grid ask again once Back shows it.
        let card = wait_role(&window, "Movie 0", AccessibleRole::Button).await?;
        let at = card.absolute_position();
        click_at(&window, LogicalPosition::new(at.x + 20., at.y + 20.));
        wait_until("the movie page", || window.get_screen() == Screen::Movie).await?;
        let mark = wait_role(&window, "Mark as watched", AccessibleRole::Button).await?;
        click(&window, &mark);
        wait_role(&window, "Mark as unwatched", AccessibleRole::Button).await?;
        press_escape(&window);
        wait_until("the library", || window.get_screen() == Screen::Library).await?;

        let card = wait_role(&window, "Movie 2", AccessibleRole::Button).await?;
        right_click(&window, &card);
        wait_menu(&window, "Movie 2").await?;
        choose(&window, "Mark as watched")?;
        wait_until("the badge of Movie 2", || watched(&window, 2)).await?;
        // Long enough for the stale answer, then for the one asked for after the change.
        for _ in 0..30 {
            pause(Duration::from_millis(80)).await;
            if !watched(&window, 2) {
                return Err("the stale answer took the badge off".into());
            }
        }
        if !watched(&window, 0) {
            return Err("the grid was not asked again".into());
        }
        Ok(())
    });
}
