//! Helpers of the cast and crew flows: a movie whose first actor is Maren Holt, her person
//! page and its titles.

use std::time::Duration;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer};

use super::{
    Flow, click, exists, fixture, json, mount_browse_basics, pause, save_session, start_with,
    wait_role, wait_until, wait_visible,
};

pub const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
pub const PERSON: &str = "11111111111111111111111111111111";
const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;
pub const OVERVIEW: &str = "A generated test movie.";
const ROW: &str = "Cast and crew";
pub const LONG_BIO: &str = "Maren Holt is a Norwegian actress known for her work in Nordic \
    thrillers and survival dramas. She trained at the Oslo National Academy of the Arts and \
    broke through with the miniseries Whiteout, before leading the international production \
    Northbound. She later moved to London, where she played on stage for several seasons, then \
    came back to film with a string of quiet dramas about the coast, the sea and the people who \
    live by it. Critics praised her restraint, her timing and the way she carries silence.";

fn replaced(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "{from} is not in the fixture");
    text.replace(from, to)
}

/// The movie fixture, whose first actor is Maren Holt, followed by a director.
fn movie() -> String {
    replaced(
        &replaced(&fixture("item_movie"), "Ada Actor", "Maren Holt"),
        r#""Role": "Extra", "Type": "Actor" }"#,
        r#""Role": "Extra", "Type": "Actor" },
    { "Name": "Karl Vinter", "Id": "77777777777777777777777777777777", "Type": "Director" }"#,
    )
}

/// Maren Holt with a biography, a birth date and a birthplace.
pub fn person_full() -> String {
    format!(
        r#"{{"Id":"{PERSON}","Name":"Maren Holt","Type":"Person","Overview":"{LONG_BIO}",
        "PremiereDate":"1984-03-12T00:00:00.0000000Z","ProductionLocations":["Bergen, Norway"]}}"#
    )
}

/// Maren Holt without image, biography nor birth.
pub fn person_bare() -> String {
    format!(r#"{{"Id":"{PERSON}","Name":"Maren Holt","Type":"Person"}}"#)
}

fn title(id: &str, name: &str, kind: &str, year: u32, role: &str) -> String {
    format!(
        r#"{{"Id":"{id}","Name":"{name}","Type":"{kind}","ProductionYear":{year},
        "People":[{{"Name":"Maren Holt","Id":"{PERSON}","Role":"{role}","Type":"Actor"}}]}}"#
    )
}

/// A listing of `items`.
pub fn titles(items: &[String]) -> String {
    format!(
        r#"{{"Items":[{}],"TotalRecordCount":{},"StartIndex":0}}"#,
        items.join(","),
        items.len()
    )
}

/// A series and two movies, the first movie being the one of the cast row.
pub fn three_titles() -> String {
    titles(&[
        title(
            "c1000000000000000000000000000001",
            "Northbound",
            "Series",
            2023,
            "Ada",
        ),
        title(MOVIE, "Test Movie (2020)", "Movie", 2020, "Hero"),
        title(
            "c1000000000000000000000000000003",
            "Driftwood",
            "Movie",
            2019,
            "",
        ),
    ])
}

/// `count` movies named "Title 000" onwards.
pub fn many_titles(count: usize) -> String {
    let items: Vec<String> = (0..count)
        .map(|n| {
            title(
                &format!("{:032x}", 0xd000_0000_u64 + n as u64),
                &format!("Title {n:03}"),
                "Movie",
                2000,
                "Role",
            )
        })
        .collect();
    titles(&items)
}

async fn mount(mock: &MockServer, person: String, credits: String) {
    Mock::given(method("GET"))
        .and(path("/Items"))
        .and(query_param("personIds", PERSON))
        .respond_with(json(credits))
        .with_priority(1)
        .mount(mock)
        .await;
    mount_browse_basics(mock, EMPTY).await;
    let routes = [
        ("/Items".to_owned(), fixture("items_movies")),
        (format!("/Items/{MOVIE}"), movie()),
        (format!("/Items/{MOVIE}/Similar"), EMPTY.to_owned()),
        (format!("/Items/{PERSON}"), person),
    ];
    for (route, body) in routes {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body))
            .mount(mock)
            .await;
    }
}

/// Starts the app signed in, serving `person` and their `credits`.
pub fn start_person(person: String, credits: String) -> Flow {
    start_with(
        move |mock: MockServer| async move {
            mount(&mock, person, credits).await;
            mock
        },
        save_session,
    )
}

pub fn press_escape(window: &AppWindow) {
    let text: slint::SharedString = Key::Escape.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

pub fn scroll(window: &AppWindow, delta: f32) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(640., 400.),
            delta_x: 0.,
            delta_y: delta,
        });
}

pub fn window_height(window: &AppWindow) -> f32 {
    window.window().size().height as f32 / window.window().scale_factor()
}

/// Opens the movie page from the Movies tab.
async fn open_movie(window: &AppWindow) -> Result<(), String> {
    wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
    let movies = wait_role(window, "Movies", AccessibleRole::Tab).await?;
    click(window, &movies);
    let card = wait_visible(window, "Test Movie (2020)").await?;
    click(window, &card);
    wait_until("the movie page", || window.get_screen() == Screen::Movie).await?;
    wait_visible(window, OVERVIEW).await?;
    Ok(())
}

/// The visible elements of type `kind`, from left to right and top to bottom.
pub fn cards(window: &AppWindow, kind: &str) -> Vec<ElementHandle> {
    let mut found: Vec<ElementHandle> = ElementHandle::find_by_element_type_name(window, kind)
        .filter(|card| card.size().width > 0. && card.size().height > 0.)
        .collect();
    found.sort_by(|a, b| {
        let (a, b) = (a.absolute_position(), b.absolute_position());
        a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x))
    });
    found
}

pub fn labels(cards: &[ElementHandle]) -> Vec<String> {
    cards
        .iter()
        .filter_map(|card| card.accessible_label().map(|label| label.to_string()))
        .collect()
}

/// Opens the movie, checks its cast row and opens Maren Holt.
pub async fn open_person(window: &AppWindow) -> Result<(), String> {
    open_movie(window).await?;
    let row = wait_role(window, ROW, AccessibleRole::List).await?;
    let play = wait_role(window, "Play", AccessibleRole::Button).await?;
    if row.absolute_position().y < play.absolute_position().y + play.size().height {
        return Err("the cast row is not below the header".into());
    }
    scroll(window, -600.);
    pause(Duration::from_millis(50)).await;
    let people = cards(window, "PersonCard");
    let names = labels(&people);
    if names.first().map(String::as_str) != Some("Maren Holt")
        || names.last().map(String::as_str) != Some("Karl Vinter")
    {
        return Err(format!("unexpected cast: {names:?}"));
    }
    for pair in people.windows(2) {
        let (left, right) = (&pair[0], &pair[1]);
        if left.absolute_position().x + left.size().width > right.absolute_position().x {
            return Err("the person cards overlap".into());
        }
    }
    let first = &people[0];
    if first.size().width < 112. {
        return Err(format!("the person card is {:?} wide", first.size().width));
    }
    if first.accessible_description().as_deref() != Some("Hero") {
        return Err(format!(
            "unexpected role: {:?}",
            first.accessible_description()
        ));
    }
    let director = people.last().and_then(|card| card.accessible_description());
    if director.as_deref() != Some("Director") {
        return Err(format!("unexpected director caption: {director:?}"));
    }
    if ElementHandle::find_by_element_type_name(window, "BodyText")
        .filter_map(|text| text.accessible_label())
        .any(|label| label.starts_with("Starring"))
    {
        return Err("the Starring line is still shown".into());
    }
    click(window, first);
    wait_until("the person page", || window.get_screen() == Screen::Person).await?;
    Ok(())
}

/// Scrolls down until the button `label` is inside the window, and returns it.
pub async fn scroll_to(window: &AppWindow, label: &str) -> Result<ElementHandle, String> {
    let height = window_height(window);
    for _ in 0..400 {
        let card = wait_role(window, label, AccessibleRole::Button).await?;
        if card.absolute_position().y + card.size().height <= height {
            return Ok(card);
        }
        scroll(window, -200.);
        pause(Duration::from_millis(10)).await;
    }
    Err(format!("{label} cannot be scrolled into the window"))
}

pub fn chip_checked(window: &AppWindow, label: &str) -> Option<bool> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        .and_then(|element| element.accessible_checked())
}

/// The cast card opened the person page: name, then birth and title count.
pub async fn cast_card_opens_person(window: &AppWindow) -> Result<(), String> {
    open_person(window).await?;
    let name = wait_visible(window, "Maren Holt").await?;
    if name.absolute_position().y < 0. {
        return Err("the name is above the window".into());
    }
    wait_visible(
        window,
        "Born 12 March 1984 in Bergen, Norway \u{b7} 3 titles in your library",
    )
    .await?;
    if exists(window, "Karl Vinter") {
        return Err("the movie page is still shown".into());
    }
    Ok(())
}

fn bio_height(window: &AppWindow) -> Result<f32, String> {
    ElementHandle::find_by_accessible_label(window, LONG_BIO)
        .next()
        .map(|bio| bio.size().height)
        .ok_or_else(|| "no biography".to_owned())
}

/// The biography is clamped to 3 lines; "Read more" shows it all and "Read less" clamps it again.
pub async fn person_bio_read_more(window: &AppWindow) -> Result<(), String> {
    let more = wait_role(window, "Read more", AccessibleRole::Button).await?;
    let clamped = bio_height(window)?;
    if !(60. ..=70.).contains(&clamped) {
        return Err(format!("the biography is {clamped} px high, not 3 lines"));
    }
    click(window, &more);
    let less = wait_role(window, "Read less", AccessibleRole::Button).await?;
    let full = bio_height(window)?;
    if full <= clamped + 10. {
        return Err(format!("the biography did not grow: {clamped} -> {full}"));
    }
    click(window, &less);
    wait_role(window, "Read more", AccessibleRole::Button).await?;
    if (bio_height(window)? - clamped).abs() > 0.5 {
        return Err("the biography is not clamped again".into());
    }
    Ok(())
}

/// The chips count the titles and filter the grid; the filter survives a visit to a title.
pub async fn person_filters_counts(window: &AppWindow) -> Result<(), String> {
    wait_role(window, "All (3)", AccessibleRole::Button).await?;
    wait_role(window, "Shows (1)", AccessibleRole::Button).await?;
    let movies = wait_role(window, "Movies (2)", AccessibleRole::Button).await?;
    let grid = cards(window, "PosterCard");
    let subtitles: Vec<_> = grid
        .iter()
        .map(|card| {
            card.accessible_description()
                .map(|text| text.to_string())
                .unwrap_or_default()
        })
        .collect();
    if labels(&grid) != ["Northbound", "Test Movie (2020)", "Driftwood"]
        || subtitles != ["2023 \u{b7} Series \u{b7} Ada", "2020 \u{b7} Hero", "2019"]
    {
        return Err(format!(
            "unexpected grid: {:?} {subtitles:?}",
            labels(&grid)
        ));
    }
    click(window, &movies);
    wait_until("two movie cards", || cards(window, "PosterCard").len() == 2).await?;
    if labels(&cards(window, "PosterCard")) != ["Test Movie (2020)", "Driftwood"] {
        return Err("the Movies filter shows other titles".into());
    }
    if chip_checked(window, "Movies (2)") != Some(true) {
        return Err("the Movies chip is not selected".into());
    }

    let card = scroll_to(window, "Test Movie (2020)").await?;
    click(window, &card);
    wait_until("the movie page opened from the grid", || {
        window.get_screen() == Screen::Movie
    })
    .await?;
    wait_visible(window, OVERVIEW).await?;
    press_escape(window);
    wait_until("the person page", || window.get_screen() == Screen::Person).await?;
    wait_role(window, "Movies (2)", AccessibleRole::Button).await?;
    if chip_checked(window, "Movies (2)") != Some(true) {
        return Err("the filter is lost after Back".into());
    }
    if cards(window, "PosterCard").len() != 2 {
        return Err("the grid is not filtered after Back".into());
    }
    Ok(())
}

/// Escape goes back to the movie page, then to the Movies tab.
pub async fn back_returns_to_detail(window: &AppWindow) -> Result<(), String> {
    press_escape(window);
    wait_until("the movie page after Escape", || {
        window.get_screen() == Screen::Movie
    })
    .await?;
    wait_visible(window, OVERVIEW).await?;
    wait_role(window, ROW, AccessibleRole::List).await?;
    press_escape(window);
    wait_until("the movies tab", || window.get_screen() == Screen::Library).await?;
    Ok(())
}

/// Scrolls down until the button `label` is inside the window, and returns it; rows below the
/// window are only built once scrolled near.
pub async fn reveal(window: &AppWindow, label: &str) -> Result<ElementHandle, String> {
    let height = window_height(window);
    let width = window.window().size().width as f32 / window.window().scale_factor();
    for _ in 0..60 {
        let found = ElementHandle::find_by_accessible_label(window, label).find(|element| {
            let (y, size) = (element.absolute_position().y, element.size());
            element.accessible_role() == Some(AccessibleRole::Button)
                && size.height > 0.
                && y >= 0.
                && y + size.height <= height
        });
        if let Some(found) = found {
            let (x, y) = (found.absolute_position().x, found.absolute_position().y);
            if x + found.size().width <= width {
                return Ok(found);
            }
            // A card of a row past the right edge: the row scrolls sideways.
            window
                .window()
                .dispatch_event(WindowEvent::PointerScrolled {
                    position: LogicalPosition::new(width / 2., y + found.size().height / 2.),
                    delta_x: -300.,
                    delta_y: 0.,
                });
            pause(Duration::from_millis(20)).await;
            continue;
        }
        scroll(window, -200.);
        pause(Duration::from_millis(20)).await;
    }
    Err(format!("{label} cannot be scrolled into the window"))
}
