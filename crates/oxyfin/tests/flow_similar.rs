//! "More like this" on the movie page: the row opens its items, and stays hidden when the
//! server finds nothing or fails.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::{
    click, exists, fixture, json, mount_browse_basics, pause, run, save_session, start_with,
    wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
const SECOND: &str = "b4c9a0c1d2e34f5a8b6c7d8e9f0a1b2c";
const THIRD: &str = "c5d0b1d2e3f45a6b9c7d8e9f0a1b2c3d";
const ROW: &str = "More like this";
const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

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

fn counted(calls: &Arc<AtomicUsize>, response: ResponseTemplate) -> Counted {
    Counted {
        calls: Arc::clone(calls),
        response,
    }
}

fn two_movies() -> String {
    format!(
        r#"{{"Items":[
            {{"Id":"{SECOND}","Name":"Second Feature (2021)","Type":"Movie","ProductionYear":2021}},
            {{"Id":"{THIRD}","Name":"Third Feature (2022)","Type":"Movie","ProductionYear":2022}}
        ],"TotalRecordCount":2,"StartIndex":0}}"#
    )
}

fn replaced(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "{from} is not in the fixture");
    text.replace(from, to)
}

/// The movie fixture as the item `id` with `overview`.
fn movie(id: &str, overview: &str) -> String {
    replaced(
        &replaced(&fixture("item_movie"), MOVIE, id),
        "A generated test movie.",
        overview,
    )
}

/// Requests counted per route.
#[derive(Clone, Default)]
struct Calls {
    similar: Arc<AtomicUsize>,
    second: Arc<AtomicUsize>,
    second_similar: Arc<AtomicUsize>,
    third: Arc<AtomicUsize>,
    third_similar: Arc<AtomicUsize>,
}

async fn mount(mock: &MockServer, calls: Calls) {
    mount_browse_basics(mock, EMPTY).await;
    let routes = [
        ("/Items".to_owned(), None, json(fixture("items_movies"))),
        (format!("/Items/{MOVIE}"), None, json(fixture("item_movie"))),
        (
            format!("/Items/{MOVIE}/Similar"),
            Some(&calls.similar),
            json(two_movies()),
        ),
        (
            format!("/Items/{SECOND}"),
            Some(&calls.second),
            json(movie(SECOND, "The second test movie.")),
        ),
        (
            format!("/Items/{SECOND}/Similar"),
            Some(&calls.second_similar),
            json(EMPTY),
        ),
        (
            format!("/Items/{THIRD}"),
            Some(&calls.third),
            json(movie(THIRD, "The third test movie.")),
        ),
        (
            format!("/Items/{THIRD}/Similar"),
            Some(&calls.third_similar),
            ResponseTemplate::new(500),
        ),
    ];
    for (route, counter, response) in routes {
        let mock_route = Mock::given(method("GET")).and(path(route));
        match counter {
            Some(calls) => mock_route.respond_with(counted(calls, response)),
            None => mock_route.respond_with(response),
        }
        .mount(mock)
        .await;
    }
}

fn press_escape(window: &AppWindow) {
    let text: slint::SharedString = Key::Escape.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn scroll_down(window: &AppWindow) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(640., 400.),
            delta_x: 0.,
            delta_y: -800.,
        });
}

/// The titles of the cards inside `row`, from left to right; the cards must not overlap.
fn row_cards(window: &AppWindow, row: &ElementHandle) -> Result<Vec<String>, String> {
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
    for pair in cards.windows(2) {
        let (left, right) = (&pair[0], &pair[1]);
        if left.absolute_position().x + left.size().width > right.absolute_position().x {
            return Err("the cards of the row overlap".into());
        }
    }
    Ok(cards
        .iter()
        .filter_map(|card| card.accessible_label().map(|label| label.to_string()))
        .collect())
}

/// Scrolls to the row, checks its two cards and clicks the one titled `title`.
async fn click_similar(window: &AppWindow, title: &str) -> Result<(), String> {
    let row = wait_role(window, ROW, AccessibleRole::List).await?;
    let play = wait_role(window, "Play", AccessibleRole::Button).await?;
    if row.absolute_position().y < play.absolute_position().y + play.size().height {
        return Err("the row is not below the header".into());
    }
    scroll_down(window);
    pause(Duration::from_millis(50)).await;
    let row = wait_role(window, ROW, AccessibleRole::List).await?;
    let cards = row_cards(window, &row)?;
    if cards != ["Second Feature (2021)", "Third Feature (2022)"] {
        return Err(format!("unexpected cards: {cards:?}"));
    }
    let card = wait_role(window, title, AccessibleRole::Button).await?;
    let bottom = window.window().size().height as f32 / window.window().scale_factor();
    if card.absolute_position().y + card.size().height / 2. > bottom {
        return Err("the card is below the window".into());
    }
    click(window, &card);
    Ok(())
}

/// Waits for the movie page showing `overview`, scrolled to its top.
async fn wait_page(window: &AppWindow, overview: &str) -> Result<(), String> {
    wait_until("a movie page", || window.get_screen() == Screen::Movie).await?;
    let text = wait_visible(window, overview).await?;
    if text.absolute_position().y < 0. {
        return Err(format!("the page of {overview} is not scrolled to its top"));
    }
    Ok(())
}

/// Waits until `calls` reached one and leaves the app time to handle the answer.
async fn wait_answer(what: &str, calls: &Arc<AtomicUsize>) -> Result<(), String> {
    wait_until(what, || calls.load(Ordering::SeqCst) >= 1).await?;
    pause(Duration::from_millis(300)).await;
    Ok(())
}

fn no_row_nor_notice(window: &AppWindow) -> Result<(), String> {
    if exists(window, ROW) {
        return Err("a row without items is shown".into());
    }
    if window.get_notice().visible {
        return Err(format!("unexpected notice: {:?}", window.get_notice()));
    }
    if !window.get_movie_error().is_empty() {
        return Err("the page shows an error".into());
    }
    Ok(())
}

#[test]
fn similar_row_opens_item_and_hides_when_empty_or_failed() {
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
        let movies = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &movies);
        let card = wait_visible(&window, "Test Movie (2020)").await?;
        click(&window, &card);
        wait_page(&window, "A generated test movie.").await?;

        click_similar(&window, "Second Feature (2021)").await?;
        wait_until("the second movie", || {
            calls.second.load(Ordering::SeqCst) == 1
        })
        .await?;
        wait_page(&window, "The second test movie.").await?;
        if exists(&window, "A generated test movie.") {
            return Err("the first movie is still shown".into());
        }
        wait_answer("the items like the second movie", &calls.second_similar).await?;
        no_row_nor_notice(&window)?;

        press_escape(&window);
        wait_page(&window, "A generated test movie.").await?;
        click_similar(&window, "Third Feature (2022)").await?;
        wait_until("the third movie", || {
            calls.third.load(Ordering::SeqCst) == 1
        })
        .await?;
        wait_page(&window, "The third test movie.").await?;
        wait_answer("the items like the third movie", &calls.third_similar).await?;
        wait_role(&window, "Play", AccessibleRole::Button).await?;
        wait_visible(&window, "The third test movie.").await?;
        no_row_nor_notice(&window)?;
        Ok(())
    });
}
