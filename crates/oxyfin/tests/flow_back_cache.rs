//! Back between detail pages shows the page left as it was, at once: no request, the same
//! scroll position, and Escape still goes back.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::person::{press_escape, reveal, scroll};
use common::{
    click, fixture, has_param, json, mount_browse_basics, pause, run, save_session, start_with,
    wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
const SECOND: &str = "b4c9a0c1d2e34f5a8b6c7d8e9f0a1b2c";
const ACTOR: &str = "11111111111111111111111111111111";
const DIRECTOR: &str = "77777777777777777777777777777777";
const BOX_SET: &str = "b0000000000000000000000000000001";
const OVERVIEW: &str = "A generated test movie.";
const SECOND_OVERVIEW: &str = "The second test movie.";
const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;
const PLAYED: &str =
    r#"{"PlaybackPositionTicks":0,"PlayCount":1,"IsFavorite":false,"Played":true}"#;

fn replaced(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "{from} is not in the fixture");
    text.replace(from, to)
}

/// The movie `id`, played by Maren Holt and directed by Karl Vinter.
fn movie(id: &str, name: &str, overview: &str) -> String {
    let movie = replaced(&fixture("item_movie"), "Ada Actor", "Maren Holt");
    let movie = replaced(
        &movie,
        r#""Role": "Extra", "Type": "Actor" }"#,
        &format!(
            r#""Role": "Extra", "Type": "Actor" }},
    {{ "Name": "Karl Vinter", "Id": "{DIRECTOR}", "Type": "Director" }}"#
        ),
    );
    let movie = replaced(&movie, OVERVIEW, overview);
    let movie = replaced(&movie, "Test Movie (2020)", name);
    movie.replace(MOVIE, id)
}

fn person(id: &str, name: &str) -> String {
    format!(r#"{{"Id":"{id}","Name":"{name}","Type":"Person","Overview":"A biography."}}"#)
}

fn listing(items: &[String]) -> String {
    format!(
        r#"{{"Items":[{}],"TotalRecordCount":{},"StartIndex":0}}"#,
        items.join(","),
        items.len()
    )
}

fn title(id: &str, name: &str, year: u32) -> String {
    format!(r#"{{"Id":"{id}","Name":"{name}","Type":"Movie","ProductionYear":{year}}}"#)
}

/// Requests counted per detail page.
#[derive(Clone, Default)]
struct Calls {
    movie: Arc<AtomicUsize>,
    second: Arc<AtomicUsize>,
    actor: Arc<AtomicUsize>,
    director: Arc<AtomicUsize>,
    box_set: Arc<AtomicUsize>,
    similar: Arc<AtomicUsize>,
    played: Arc<AtomicUsize>,
}

fn count(calls: &Arc<AtomicUsize>) -> usize {
    calls.load(Ordering::SeqCst)
}

struct Counted {
    calls: Arc<AtomicUsize>,
    body: String,
}

impl Respond for Counted {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.calls.fetch_add(1, Ordering::SeqCst);
        json(self.body.clone())
    }
}

/// Answers `/Items` by its query: the box sets, their movies, the titles of a person, or the
/// movies.
struct Items;

impl Respond for Items {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query = request.url.query().unwrap_or("");
        let both = [
            title(MOVIE, "Test Movie (2020)", 2020),
            title(SECOND, "Second Feature (2021)", 2021),
        ];
        if has_param(query, "includeItemTypes", "BoxSet") {
            json(listing(&[format!(
                r#"{{"Id":"{BOX_SET}","Name":"Northbound Collection","Type":"BoxSet"}}"#
            )]))
        } else if has_param(query, "parentId", BOX_SET) || query.contains("personIds=") {
            json(listing(&both))
        } else {
            json(fixture("items_movies"))
        }
    }
}

async fn mount(mock: &MockServer, calls: Calls) {
    mount_browse_basics(mock, EMPTY).await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .respond_with(Items)
        .mount(mock)
        .await;
    let box_set = format!(
        r#"{{"Id":"{BOX_SET}","Name":"Northbound Collection","Type":"BoxSet",
        "Overview":"Expeditions to the last unmapped coasts."}}"#
    );
    let counted = [
        (
            MOVIE,
            &calls.movie,
            movie(MOVIE, "Test Movie (2020)", OVERVIEW),
        ),
        (
            SECOND,
            &calls.second,
            movie(SECOND, "Second Feature (2021)", SECOND_OVERVIEW),
        ),
        (ACTOR, &calls.actor, person(ACTOR, "Maren Holt")),
        (DIRECTOR, &calls.director, person(DIRECTOR, "Karl Vinter")),
        (BOX_SET, &calls.box_set, box_set),
    ];
    for (id, calls, body) in counted {
        Mock::given(method("GET"))
            .and(path(format!("/Items/{id}")))
            .respond_with(Counted {
                calls: Arc::clone(calls),
                body,
            })
            .mount(mock)
            .await;
    }
    Mock::given(method("GET"))
        .and(path(format!("/Items/{MOVIE}/Similar")))
        .respond_with(Counted {
            calls: Arc::clone(&calls.similar),
            body: listing(&[title(SECOND, "Second Feature (2021)", 2021)]),
        })
        .mount(mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{SECOND}/Similar")))
        .respond_with(json(EMPTY))
        .mount(mock)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/UserPlayedItems/{SECOND}")))
        .respond_with(Counted {
            calls: Arc::clone(&calls.played),
            body: PLAYED.to_owned(),
        })
        .mount(mock)
        .await;
}

async fn open_movie_tab(window: &AppWindow) -> Result<(), String> {
    let movies = wait_role(window, "Movies", AccessibleRole::Tab).await?;
    click(window, &movies);
    let card = wait_visible(window, "Test Movie (2020)").await?;
    click(window, &card);
    wait_until("the movie page", || {
        window.get_screen() == Screen::Movie && window.get_movie_data().overview == OVERVIEW
    })
    .await
}

/// Waits for the screen `screen` again after Back, then for the answers a request would bring.
async fn back_to(window: &AppWindow, what: &str, screen: Screen) -> Result<(), String> {
    press_escape(window);
    wait_until(what, || window.get_screen() == screen).await?;
    pause(Duration::from_millis(300)).await;
    Ok(())
}

fn same(what: &str, before: f32, after: f32) -> Result<(), String> {
    if (before - after).abs() > 0.5 {
        return Err(format!("{what} moved from {before} to {after}"));
    }
    Ok(())
}

fn y_of(window: &AppWindow, label: &str, role: AccessibleRole) -> Option<f32> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role) && element.size().height > 0.)
        .map(|element| element.absolute_position().y)
}

/// Movie, a movie of its "More like this" row, Back: the first movie comes back as it was.
async fn movie_similar_back(window: &AppWindow, calls: &Calls) -> Result<(), String> {
    open_movie_tab(window).await?;
    let card = reveal(window, "Second Feature (2021)").await?;
    let (offset, row) = (
        window.get_movie_viewport_y(),
        y_of(window, "More like this", AccessibleRole::List),
    );
    if offset >= 0. {
        return Err("the movie page did not scroll".into());
    }
    click(window, &card);
    wait_until("the second movie", || {
        window.get_movie_data().overview == SECOND_OVERVIEW
    })
    .await?;
    if window.get_movie_viewport_y() != 0. {
        return Err("the second movie does not start at its top".into());
    }
    back_to(window, "the first movie again", Screen::Movie).await?;
    if window.get_movie_data().overview != OVERVIEW {
        return Err("Back does not show the first movie".into());
    }
    if count(&calls.movie) != 1 {
        return Err(format!(
            "the first movie was asked {} times",
            count(&calls.movie)
        ));
    }
    same("the movie page", offset, window.get_movie_viewport_y())?;
    let after = y_of(window, "More like this", AccessibleRole::List);
    match (row, after) {
        (Some(row), Some(after)) => same("the row", row, after)?,
        _ => return Err("the row is not shown after Back".into()),
    }
    back_to(window, "the movies tab", Screen::Library).await
}

/// Movie, one of its cast, Back: the movie keeps its scroll position without a request.
async fn movie_person_back(window: &AppWindow, calls: &Calls) -> Result<(), String> {
    open_movie_tab(window).await?;
    let asked = count(&calls.movie);
    let card = reveal(window, "Maren Holt").await?;
    let (offset, y) = (window.get_movie_viewport_y(), card.absolute_position().y);
    click(window, &card);
    wait_until("the person page", || window.get_screen() == Screen::Person).await?;
    back_to(window, "the movie page", Screen::Movie).await?;
    if count(&calls.movie) != asked {
        return Err("the movie was asked again after Back".into());
    }
    same("the movie page", offset, window.get_movie_viewport_y())?;
    let card = wait_role(window, "Maren Holt", AccessibleRole::Button).await?;
    same("the cast card", y, card.absolute_position().y)?;
    back_to(window, "the movies tab", Screen::Library).await
}

/// Movie, person, one of their movies, another person, then Back to each.
async fn person_movie_person_back(window: &AppWindow, calls: &Calls) -> Result<(), String> {
    open_movie_tab(window).await?;
    let asked = count(&calls.movie);
    let card = reveal(window, "Maren Holt").await?;
    click(window, &card);
    wait_until("Maren Holt", || {
        window.get_screen() == Screen::Person && window.get_person_data().name == "Maren Holt"
    })
    .await?;
    let card = reveal(window, "Second Feature (2021)").await?;
    let offset = window.get_person_viewport_y();
    click(window, &card);
    wait_until("the second movie", || {
        window.get_screen() == Screen::Movie && window.get_movie_data().overview == SECOND_OVERVIEW
    })
    .await?;
    let card = reveal(window, "Karl Vinter").await?;
    click(window, &card);
    wait_until("Karl Vinter", || {
        window.get_screen() == Screen::Person && window.get_person_data().name == "Karl Vinter"
    })
    .await?;
    let (actor, second) = (count(&calls.actor), count(&calls.second));
    back_to(window, "the second movie", Screen::Movie).await?;
    if window.get_movie_data().overview != SECOND_OVERVIEW || count(&calls.second) != second {
        return Err("the second movie is not shown as it was".into());
    }
    back_to(window, "Maren Holt again", Screen::Person).await?;
    if window.get_person_data().name != "Maren Holt" || count(&calls.actor) != actor {
        return Err("Maren Holt is not shown as she was".into());
    }
    same("the person page", offset, window.get_person_viewport_y())?;
    back_to(window, "the first movie", Screen::Movie).await?;
    if window.get_movie_data().overview != OVERVIEW || count(&calls.movie) != asked {
        return Err("the first movie is not shown as it was".into());
    }
    back_to(window, "the movies tab", Screen::Library).await
}

/// Movie, its collection, a movie of the collection, Back to each.
async fn collection_movie_back(window: &AppWindow, calls: &Calls) -> Result<(), String> {
    open_movie_tab(window).await?;
    let asked = count(&calls.movie);
    let link = wait_role(
        window,
        "Part of the Northbound Collection",
        AccessibleRole::Button,
    )
    .await?;
    click(window, &link);
    wait_until("the collection page", || {
        window.get_screen() == Screen::Collection && window.get_collection_loaded()
    })
    .await?;
    scroll(window, -600.);
    pause(Duration::from_millis(50)).await;
    let card = reveal(window, "Second Feature (2021)").await?;
    let (offset, y) = (
        window.get_collection_viewport_y(),
        card.absolute_position().y,
    );
    let box_set = count(&calls.box_set);
    click(window, &card);
    wait_until("the second movie", || {
        window.get_screen() == Screen::Movie && window.get_movie_data().overview == SECOND_OVERVIEW
    })
    .await?;
    back_to(window, "the collection page", Screen::Collection).await?;
    if count(&calls.box_set) != box_set {
        return Err("the collection was asked again after Back".into());
    }
    same(
        "the collection page",
        offset,
        window.get_collection_viewport_y(),
    )?;
    let card = wait_role(window, "Second Feature (2021)", AccessibleRole::Button).await?;
    same("the collection card", y, card.absolute_position().y)?;
    back_to(window, "the first movie", Screen::Movie).await?;
    if window.get_movie_data().overview != OVERVIEW || count(&calls.movie) != asked {
        return Err("the first movie is not shown as it was".into());
    }
    back_to(window, "the movies tab", Screen::Library).await
}

/// A change on the page above reloads the page below in place, its row included, on Back.
async fn change_refreshes_the_page_below(window: &AppWindow, calls: &Calls) -> Result<(), String> {
    open_movie_tab(window).await?;
    wait_until("the row of the first movie", || count(&calls.similar) > 0).await?;
    pause(Duration::from_millis(200)).await;
    let (asked, similar) = (count(&calls.movie), count(&calls.similar));
    let card = reveal(window, "Second Feature (2021)").await?;
    let offset = window.get_movie_viewport_y();
    click(window, &card);
    wait_until("the second movie", || {
        window.get_movie_data().overview == SECOND_OVERVIEW
    })
    .await?;
    let watched = wait_role(window, "Mark as watched", AccessibleRole::Button).await?;
    click(window, &watched);
    wait_until("the change", || count(&calls.played) == 1).await?;
    pause(Duration::from_millis(200)).await;
    back_to(window, "the first movie", Screen::Movie).await?;
    wait_until("the first movie and its row asked again", || {
        count(&calls.movie) == asked + 1 && count(&calls.similar) == similar + 1
    })
    .await?;
    pause(Duration::from_millis(200)).await;
    same("the movie page", offset, window.get_movie_viewport_y())?;
    back_to(window, "the movies tab", Screen::Library).await
}

#[test]
fn back_shows_detail_pages_as_they_were() {
    let calls = Calls::default();
    let served = calls.clone();
    let flow = start_with(
        move |mock: MockServer| async move {
            mount(&mock, served).await;
            mock
        },
        save_session,
    );
    let window = flow.window.as_weak();
    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        movie_similar_back(&window, &calls).await?;
        movie_person_back(&window, &calls).await?;
        person_movie_person_back(&window, &calls).await?;
        collection_movie_back(&window, &calls).await?;
        change_refreshes_the_page_below(&window, &calls).await?;
        Ok(())
    });
}
