//! Helpers of the collection flows: a movie in the Northbound Collection, its box set page and
//! a search finding the box set.

use std::sync::{Arc, Mutex};

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::{
    Flow, QueryLog, click, fixture, has_param, json, mount_browse_basics, save_session, start_with,
    wait_role, wait_until, wait_visible,
};

pub const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
pub const BOX_SET: &str = "b0000000000000000000000000000001";
pub const OTHER_BOX_SET: &str = "b0000000000000000000000000000002";
pub const OVERVIEW: &str = "A generated test movie.";
pub const LINK: &str = "Part of the Northbound Collection";
pub const ROW: &str = "In this collection";
pub const CAPTION: &str = "3 movies \u{b7} 2019 to 2024 \u{b7} 1 watched";
pub const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

fn listing(items: &str) -> ResponseTemplate {
    json(format!(
        r#"{{"Items":[{items}],"TotalRecordCount":3,"StartIndex":0}}"#
    ))
}

fn box_set(id: &str, name: &str) -> String {
    format!(r#"{{"Id":"{id}","Name":"{name}","Type":"BoxSet","ProductionYear":2019}}"#)
}

/// The three movies of the Northbound Collection, out of premiere order.
fn children() -> String {
    format!(
        r#"{{"Id":"c0000000000000000000000000000003","Name":"The Surveyor","Type":"Movie",
            "ProductionYear":2024,"PremiereDate":"2024-02-01T00:00:00.0000000Z"}},
        {{"Id":"c0000000000000000000000000000001","Name":"The Long Coast","Type":"Movie",
            "ProductionYear":2019,"PremiereDate":"2019-05-01T00:00:00.0000000Z",
            "UserData":{{"Played":true,"IsFavorite":false,"PlaybackPositionTicks":0}}}},
        {{"Id":"{MOVIE}","Name":"Test Movie (2020)","Type":"Movie",
            "ProductionYear":2020,"PremiereDate":"2020-01-01T00:00:00.0000000Z"}}"#
    )
}

/// Answers `/Items` by its query: the box sets, their movies, a search, or the movies.
struct Items {
    log: QueryLog,
    in_collection: bool,
}

impl Respond for Items {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query = request.url.query().unwrap_or("").to_owned();
        self.log.lock().map(|mut log| log.push(query.clone())).ok();
        if has_param(&query, "includeItemTypes", "BoxSet") {
            let (id, name) = if self.in_collection {
                (BOX_SET, "Northbound Collection")
            } else {
                (OTHER_BOX_SET, "Other Collection")
            };
            listing(&box_set(id, name))
        } else if has_param(&query, "parentId", BOX_SET) {
            listing(&children())
        } else if has_param(&query, "parentId", OTHER_BOX_SET) {
            listing(
                r#"{"Id":"c0000000000000000000000000000009","Name":"Elsewhere","Type":"Movie"}"#,
            )
        } else if has_param(&query, "searchTerm", "north") {
            listing(&box_set(BOX_SET, "Northbound Collection"))
        } else {
            json(fixture("items_movies"))
        }
    }
}

pub fn start(in_collection: bool) -> (Flow, QueryLog) {
    let log: QueryLog = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::clone(&log);
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, EMPTY).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(Items {
                    log: served,
                    in_collection,
                })
                .mount(&mock)
                .await;
            let box_set = format!(
                r#"{{"Id":"{BOX_SET}","Name":"Northbound Collection","Type":"BoxSet",
                "Overview":"Expeditions to the last unmapped coasts.",
                "UserData":{{"Played":false,"IsFavorite":false,"PlaybackPositionTicks":0}}}}"#
            );
            let routes = [
                (format!("/Items/{MOVIE}"), fixture("item_movie")),
                (format!("/Items/{MOVIE}/Similar"), EMPTY.to_owned()),
                (format!("/Items/{BOX_SET}"), box_set),
            ];
            for (route, body) in routes {
                Mock::given(method("GET"))
                    .and(path(route))
                    .respond_with(json(body))
                    .mount(&mock)
                    .await;
            }
            mock
        },
        save_session,
    );
    (flow, log)
}

pub fn press(window: &AppWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

pub fn requested(log: &QueryLog, key: &str, value: &str) -> bool {
    log.lock()
        .map(|log| log.iter().any(|query| has_param(query, key, value)))
        .unwrap_or(false)
}

pub async fn open_movie(window: &AppWindow) -> Result<(), String> {
    wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
    let movies = wait_role(window, "Movies", AccessibleRole::Tab).await?;
    click(window, &movies);
    let card = wait_visible(window, "Test Movie (2020)").await?;
    click(window, &card);
    wait_until("the movie page", || window.get_screen() == Screen::Movie).await?;
    wait_visible(window, OVERVIEW).await?;
    Ok(())
}

/// Opens the movie, checks the link sits between the genres and the overview, and follows it.
pub async fn open_collection(window: &AppWindow) -> Result<(), String> {
    open_movie(window).await?;
    let link = wait_role(window, LINK, AccessibleRole::Button).await?;
    let genre = wait_visible(window, "Drama").await?;
    let overview = wait_visible(window, OVERVIEW).await?;
    let (top, bottom) = (
        link.absolute_position().y,
        link.absolute_position().y + link.size().height,
    );
    if top < genre.absolute_position().y + genre.size().height
        || bottom > overview.absolute_position().y
    {
        return Err("the collection link is not between the genres and the overview".into());
    }
    click(window, &link);
    wait_until("the collection page", || {
        window.get_screen() == Screen::Collection
    })
    .await?;
    wait_visible(window, "Expeditions to the last unmapped coasts.").await?;
    Ok(())
}

/// The titles of the poster cards inside `row`, from left to right.
pub fn row_titles(window: &AppWindow, row: &ElementHandle) -> Vec<String> {
    let top = row.absolute_position().y;
    let bottom = top + row.size().height;
    let mut cards: Vec<ElementHandle> =
        ElementHandle::find_by_element_type_name(window, "PosterCard")
            .filter(|card| {
                let y = card.absolute_position().y;
                card.size().height > 0. && y >= top && y + card.size().height <= bottom
            })
            .collect();
    cards.sort_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x));
    cards
        .iter()
        .filter_map(|card| card.accessible_label().map(|label| label.to_string()))
        .collect()
}

/// Scrolls the page by `delta`, negative to go down.
pub fn scroll(window: &AppWindow, delta: f32) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(640., 400.),
            delta_x: 0.,
            delta_y: delta,
        });
}

/// Presses and releases Escape.
pub fn press_escape(window: &AppWindow) {
    press(window, Key::Escape);
}
