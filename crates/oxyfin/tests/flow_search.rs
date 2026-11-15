//! Searches with the keyboard: debounce, minimum length, Enter, empty and stale answers.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    QueryLog, click, exists, fixture, has_param, json, mount_browse_basics, pause, run,
    save_session, start_with, wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{AppWindow, Screen, TopBarPage};
use slint::ComponentHandle;
use slint::platform::{Key, WindowEvent};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

struct Items {
    log: QueryLog,
}

impl Respond for Items {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query = request.url.query().unwrap_or("").to_owned();
        self.log.lock().map(|mut log| log.push(query.clone())).ok();
        if has_param(&query, "searchTerm", "tes") {
            json(fixture("items_search"))
        } else if has_param(&query, "searchTerm", "slow") {
            json(
                fixture("items_search")
                    .replace("Tesseract", "Slow Hit")
                    .replace("Test Pattern", "Slow Show"),
            )
            .set_delay(Duration::from_millis(700))
        } else {
            json(EMPTY)
        }
    }
}

fn searches(log: &QueryLog) -> Vec<String> {
    log.lock()
        .map(|log| {
            log.iter()
                .filter(|query| query.contains("searchTerm="))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn press(window: &AppWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn type_text(window: &AppWindow, text: &str) {
    for character in text.chars() {
        press(window, character.to_string());
    }
}

fn field_value(window: &AppWindow) -> Option<String> {
    ElementHandle::find_by_accessible_label(window, "Search titles")
        .find(|element| element.accessible_role() == Some(AccessibleRole::TextInput))
        .and_then(|element| element.accessible_value())
        .map(|value| value.to_string())
}

#[test]
fn search_waits_for_the_typing_to_stop_and_ignores_stale_answers() {
    let log: QueryLog = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::clone(&log);
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, EMPTY).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(Items { log: served })
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

        let magnifier = wait_role(&window, "Search", AccessibleRole::Button).await?;
        click(&window, &magnifier);
        wait_until("the search screen", || {
            window.get_screen() == Screen::Search
        })
        .await?;
        wait_role(&window, "Search titles", AccessibleRole::TextInput).await?;
        if window.get_top_bar_page() != TopBarPage::Search {
            return Err("Search is not the underlined tab".into());
        }
        wait_visible(&window, "Search movies and shows").await?;
        if !searches(&log).is_empty() {
            return Err("a search started before any typing".into());
        }

        // Only a focused field receives the keys.
        type_text(&window, "te");
        pause(Duration::from_millis(100)).await;
        type_text(&window, "s");
        if field_value(&window).as_deref() != Some("tes") {
            return Err(format!("the field holds {:?}", field_value(&window)));
        }
        wait_visible(&window, "Tesseract").await?;
        wait_visible(&window, "Test Pattern").await?;
        wait_visible(&window, "2 titles").await?;
        pause(Duration::from_millis(500)).await;
        let requests = searches(&log);
        if requests.len() != 1 || !has_param(&requests[0], "searchTerm", "tes") {
            return Err(format!("expected one search for tes, got {requests:?}"));
        }
        for (key, value) in [("startIndex", "0"), ("limit", "60")] {
            if !has_param(&requests[0], key, value) {
                return Err(format!("{key}={value} missing from {}", requests[0]));
            }
        }

        for _ in 0..3 {
            press(&window, Key::Backspace);
        }
        type_text(&window, "x");
        wait_visible(&window, "Search movies and shows").await?;
        if exists(&window, "Tesseract") {
            return Err("the results of the previous text are still shown".into());
        }
        pause(Duration::from_millis(600)).await;
        if searches(&log).len() != 1 {
            return Err("a one-character text was searched".into());
        }

        press(&window, Key::Backspace);
        type_text(&window, "slow");
        press(&window, Key::Return);
        wait_until("the slow search", || searches(&log).len() == 2).await?;
        type_text(&window, "x");
        let edited_at = std::time::Instant::now();
        while edited_at.elapsed() < Duration::from_millis(200) {
            if exists(&window, "No results for “slow”") {
                return Err("an empty result is shown while the text is edited".into());
            }
            pause(Duration::from_millis(10)).await;
        }
        for _ in 0..5 {
            press(&window, Key::Backspace);
        }
        wait_visible(&window, "Search movies and shows").await?;
        pause(Duration::from_millis(1000)).await;
        if exists(&window, "Slow Hit") {
            return Err("an answer for a text that left the field was shown".into());
        }

        type_text(&window, "zz");
        press(&window, Key::Return);
        wait_visible(&window, "No results for “zz”").await?;
        if searches(&log).len() != 3 {
            return Err("Enter did not search at once".into());
        }

        let movies = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &movies);
        wait_until("the movies", || window.get_screen() == Screen::Library).await?;
        let magnifier = wait_role(&window, "Search", AccessibleRole::Button).await?;
        click(&window, &magnifier);
        wait_role(&window, "Search titles", AccessibleRole::TextInput).await?;
        wait_visible(&window, "Search movies and shows").await?;
        if field_value(&window).as_deref() != Some("") {
            return Err(format!("the text was kept: {:?}", field_value(&window)));
        }
        Ok(())
    });
}
