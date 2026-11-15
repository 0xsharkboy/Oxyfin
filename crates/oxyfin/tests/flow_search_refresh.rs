//! Coming back from the player reloads the search results without losing the text.

mod common;

use std::rc::Rc;
use std::sync::{Arc, Mutex};

use common::{
    QueryLog, fixture, has_param, json, mount_browse_basics, run, save_session, start_with,
    wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::state::Msg;
use oxyfin::{AppWindow, Screen};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

struct Items {
    log: QueryLog,
}

impl Respond for Items {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query = request.url.query().unwrap_or("").to_owned();
        if !has_param(&query, "searchTerm", "tes") {
            return json(EMPTY);
        }
        let seen = self
            .log
            .lock()
            .map(|mut log| {
                log.push(query);
                log.len()
            })
            .unwrap_or(0);
        if seen == 1 {
            json(fixture("items_search"))
        } else {
            json(fixture("items_search").replace("Tesseract", "Tesseract Reloaded"))
        }
    }
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

fn field_value(window: &AppWindow) -> Option<String> {
    ElementHandle::find_by_accessible_label(window, "Search titles")
        .find(|element| element.accessible_role() == Some(AccessibleRole::TextInput))
        .and_then(|element| element.accessible_value())
        .map(|value| value.to_string())
}

#[test]
fn search_reloads_its_first_page_after_the_player() {
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
    let app = Rc::clone(&flow.app);

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let magnifier = wait_role(&window, "Search", AccessibleRole::Button).await?;
        common::click(&window, &magnifier);
        wait_role(&window, "Search titles", AccessibleRole::TextInput).await?;
        for character in "tes".chars() {
            press(&window, character.to_string());
        }
        press(&window, Key::Return);
        wait_visible(&window, "Tesseract").await?;

        app.dispatch(Msg::OpenPlayer);
        app.dispatch(Msg::ClosePlayer);
        if window.get_search_cards().row_count() == 0 {
            return Err("the results were wiped while reloading".into());
        }
        wait_visible(&window, "Tesseract Reloaded").await?;
        let requests = log.lock().map(|log| log.clone()).unwrap_or_default();
        if requests.len() != 2 || !has_param(&requests[1], "startIndex", "0") {
            return Err(format!("expected a reload of the first page: {requests:?}"));
        }
        if field_value(&window).as_deref() != Some("tes") {
            return Err(format!("the text was lost: {:?}", field_value(&window)));
        }
        Ok(())
    });
}
