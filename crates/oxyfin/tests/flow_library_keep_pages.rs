//! A change made on a card of the second page, from its menu or from its page, keeps every
//! page loaded and the scroll position of the grid.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use common::menu::{choose, right_click, wait_menu};
use common::person::press_escape;
use common::{
    QueryLog, click, click_at, has_param, json, mount_browse_basics, run, save_session, start_with,
    wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{AppWindow, Screen};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, Model};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const TOTAL: usize = 70;
const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

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
        "Name": format!("Movie {index:03}"), "Id": id(index), "Type": "Movie",
        "IsFolder": false, "ProductionYear": 2020,
        "UserData": {"PlaybackPositionTicks": 0, "PlayCount": 0, "IsFavorite": false,
            "Played": watched}
    })
}

struct Items {
    log: QueryLog,
    played: Played,
}

impl Respond for Items {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query = request.url.query().unwrap_or("").to_owned();
        self.log.lock().map(|mut log| log.push(query.clone())).ok();
        let param = |key: &str| {
            query.split('&').find_map(|pair| {
                pair.strip_prefix(&format!("{key}="))
                    .and_then(|value| value.parse::<usize>().ok())
            })
        };
        let start = param("startIndex").unwrap_or(0).min(TOTAL);
        let end = (start + param("limit").unwrap_or(TOTAL)).min(TOTAL);
        let items: Vec<serde_json::Value> = (start..end)
            .map(|index| movie(index, &self.played))
            .collect();
        json(
            serde_json::json!({"Items": items, "TotalRecordCount": TOTAL, "StartIndex": start})
                .to_string(),
        )
    }
}

/// One movie, as `Items/{id}` gives it.
struct Item {
    played: Played,
}

impl Respond for Item {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let wanted = request.url.path().rsplit('/').next().unwrap_or("");
        match (0..TOTAL).find(|index| id(*index) == wanted) {
            Some(index) => json(movie(index, &self.played).to_string()),
            None => ResponseTemplate::new(404),
        }
    }
}

struct MarkPlayed {
    played: Played,
}

impl Respond for MarkPlayed {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let item = request
            .url
            .path()
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_owned();
        self.played
            .lock()
            .map(|mut played| played.insert(item))
            .ok();
        json(r#"{"PlaybackPositionTicks":0,"PlayCount":1,"IsFavorite":false,"Played":true}"#)
    }
}

fn first_pages(log: &QueryLog) -> usize {
    log.lock()
        .map(|log| {
            log.iter()
                .filter(|query| has_param(query, "startIndex", "0"))
                .count()
        })
        .unwrap_or(0)
}

fn cards(window: &AppWindow) -> Vec<(String, bool)> {
    let cards = window.get_library_cards();
    (0..cards.row_count())
        .filter_map(|index| cards.row_data(index))
        .map(|card| (card.id.to_string(), card.watched))
        .collect()
}

fn watched(window: &AppWindow, index: usize) -> bool {
    cards(window)
        .iter()
        .any(|(card, watched)| *card == id(index) && *watched)
}

#[test]
fn changes_keep_every_page_of_the_library() {
    let log: QueryLog = Arc::new(Mutex::new(Vec::new()));
    let played: Played = Arc::new(Mutex::new(HashSet::new()));
    let (served, held) = (Arc::clone(&log), Arc::clone(&played));
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, EMPTY).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(Items {
                    log: served,
                    played: Arc::clone(&held),
                })
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path_regex("^/Items/[0-9a-f]{32}$"))
                .respond_with(Item {
                    played: Arc::clone(&held),
                })
                .mount(&mock)
                .await;
            Mock::given(method("POST"))
                .and(path_regex("^/UserPlayedItems/[0-9a-f]{32}$"))
                .respond_with(MarkPlayed { played: held })
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
        wait_visible(&window, "70 titles").await?;
        wait_until("the first page", || cards(&window).len() == 60).await?;
        window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(640., 400.),
                delta_x: 0.,
                delta_y: -20000.,
            });
        wait_until("the second page", || cards(&window).len() == TOTAL).await?;
        let reloads = first_pages(&log);

        let card = wait_role(&window, "Movie 065", AccessibleRole::Button).await?;
        right_click(&window, &card);
        wait_menu(&window, "Movie 065").await?;
        choose(&window, "Mark as watched")?;
        wait_until("the watched badge of Movie 065", || watched(&window, 65)).await?;
        if cards(&window).len() != TOTAL {
            return Err(format!(
                "{} cards left after the change",
                cards(&window).len()
            ));
        }
        if first_pages(&log) != reloads {
            return Err("a change from the menu reloads the grid".into());
        }

        let card = wait_role(&window, "Movie 064", AccessibleRole::Button).await?;
        let at = card.absolute_position();
        click_at(&window, LogicalPosition::new(at.x + 20., at.y + 20.));
        wait_until("the movie page", || window.get_screen() == Screen::Movie).await?;
        // The click gave the card the focus, which scrolled it into view.
        let scrolled = window.get_library_viewport_y();
        let mark = wait_role(&window, "Mark as watched", AccessibleRole::Button).await?;
        click(&window, &mark);
        wait_role(&window, "Mark as unwatched", AccessibleRole::Button).await?;
        press_escape(&window);
        wait_until("the library", || window.get_screen() == Screen::Library).await?;
        wait_until("the watched badge of Movie 064", || watched(&window, 64)).await?;
        if cards(&window).len() != TOTAL || !watched(&window, 65) {
            return Err(format!(
                "the reload kept {} cards, Movie 065 watched: {}",
                cards(&window).len(),
                watched(&window, 65)
            ));
        }
        let viewport = window.get_library_viewport_y();
        if (viewport - scrolled).abs() > 0.5 {
            return Err(format!("the grid moved from {scrolled} to {viewport}"));
        }
        Ok(())
    });
}
