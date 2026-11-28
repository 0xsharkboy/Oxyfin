//! Browses movies and favorites: paging request, filters, genre, stale answers, empty and expired.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    QueryLog, click, exists, fixture, has_param, json, mount_browse_basics, pause, run,
    save_session, start_with, wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::state::Msg;
use oxyfin::{AppWindow, Message, Screen};
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn page(titles: &[&str]) -> String {
    let items: Vec<String> = titles
        .iter()
        .enumerate()
        .map(|(index, title)| {
            format!(
                r#"{{"Name":"{title}","Id":"{index:032x}","Type":"Movie","IsFolder":false,"ProductionYear":2020,
                "UserData":{{"PlaybackPositionTicks":0,"PlayCount":0,"IsFavorite":false,"Played":false}}}}"#
            )
        })
        .collect();
    format!(
        r#"{{"Items":[{}],"TotalRecordCount":{},"StartIndex":0}}"#,
        items.join(","),
        titles.len()
    )
}

struct Items {
    log: QueryLog,
}

impl Respond for Items {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query = request.url.query().unwrap_or("").to_owned();
        self.log.lock().map(|mut log| log.push(query.clone())).ok();
        let has = |key, value| has_param(&query, key, value);
        if has("includeItemTypes", "Series") {
            ResponseTemplate::new(401)
        } else if has("isFavorite", "true") {
            json(page(&[]))
        } else if has("genres", "Comedy") && !has("isPlayed", "true") {
            json(page(&["Comedy Movie"]))
        } else if has("isPlayed", "true") {
            json(page(&["Slow Watched"])).set_delay(Duration::from_millis(700))
        } else if has("isPlayed", "false") {
            json(page(&["Unplayed Movie"]))
        } else {
            json(fixture("items_movies"))
        }
    }
}

async fn mount(mock: MockServer, log: QueryLog) -> MockServer {
    mount_browse_basics(&mock, r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#).await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .respond_with(Items { log })
        .mount(&mock)
        .await;
    mock
}

fn queries(log: &QueryLog) -> Vec<String> {
    log.lock().map(|log| log.clone()).unwrap_or_default()
}

fn last(log: &QueryLog) -> String {
    queries(log).pop().unwrap_or_default()
}

fn selected(window: &AppWindow, label: &str, role: AccessibleRole) -> Option<bool> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.accessible_role() == Some(role))
        .and_then(|element| match role {
            AccessibleRole::Tab => element.accessible_item_selected(),
            _ => element.accessible_checked(),
        })
}

async fn press(window: &AppWindow, label: &str, role: AccessibleRole) -> Result<(), String> {
    let element = wait_role(window, label, role).await?;
    click(window, &element);
    Ok(())
}

#[test]
fn library_filters_pages_and_recovers() {
    let log: QueryLog = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::clone(&log);
    let flow = start_with(move |mock| mount(mock, served), save_session);
    let window = flow.window.as_weak();
    let app = flow.app.clone();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;

        press(&window, "Movies", AccessibleRole::Tab).await?;
        wait_visible(&window, "2 titles").await?;
        wait_visible(&window, "Test Movie (2020)").await?;
        wait_visible(&window, "Second Feature (2021)").await?;
        let first = queries(&log).remove(0);
        for (key, value) in [
            ("includeItemTypes", "Movie"),
            ("sortBy", "DateCreated"),
            ("sortOrder", "Descending"),
            ("startIndex", "0"),
            ("limit", "60"),
        ] {
            if !has_param(&first, key, value) {
                return Err(format!("{key}={value} missing from {first}"));
            }
        }
        if window.get_screen() != Screen::Library {
            return Err("the library screen is not shown".into());
        }
        if selected(&window, "Movies", AccessibleRole::Tab) != Some(true) {
            return Err("Movies is not the underlined tab".into());
        }

        press(&window, "Unwatched", AccessibleRole::Button).await?;
        wait_visible(&window, "Unplayed Movie").await?;
        wait_visible(&window, "1 title").await?;
        if !has_param(&last(&log), "isPlayed", "false") {
            return Err(format!("no isPlayed=false in {}", last(&log)));
        }
        if exists(&window, "Second Feature (2021)") {
            return Err("the previous grid is still shown".into());
        }

        press(&window, "Genre: All genres", AccessibleRole::Button).await?;
        press(&window, "Comedy", AccessibleRole::ListItem).await?;
        wait_visible(&window, "Comedy Movie").await?;
        let genre = last(&log);
        if !has_param(&genre, "genres", "Comedy") || !has_param(&genre, "isPlayed", "false") {
            return Err(format!("genre query: {genre}"));
        }

        // The slow answer for Watched arrives after the filter went back to All.
        press(&window, "Watched", AccessibleRole::Button).await?;
        wait_until("the Watched request", || {
            has_param(&last(&log), "isPlayed", "true")
        })
        .await?;
        press(&window, "All", AccessibleRole::Button).await?;
        wait_until("the All request", || {
            !has_param(&last(&log), "isPlayed", "true")
                && !has_param(&last(&log), "isPlayed", "false")
        })
        .await?;
        wait_visible(&window, "Comedy Movie").await?;
        pause(Duration::from_millis(1000)).await;
        if exists(&window, "Slow Watched") {
            return Err("a stale page was shown".into());
        }
        if !exists(&window, "Comedy Movie") {
            return Err("the current page vanished".into());
        }

        press(&window, "Favorites", AccessibleRole::Tab).await?;
        wait_visible(&window, "Nothing here yet").await?;
        if !common::bones(&window).is_empty() {
            return Err("loading bones are left over an empty library".into());
        }
        if !has_param(&last(&log), "isFavorite", "true") {
            return Err(format!("no isFavorite=true in {}", last(&log)));
        }
        press(&window, "Unwatched", AccessibleRole::Button).await?;
        wait_visible(&window, "No titles match these filters").await?;

        let before = queries(&log).len();
        press(&window, "Movies", AccessibleRole::Tab).await?;
        wait_visible(&window, "Comedy Movie").await?;
        // The grid shows its last answer at once and asks the server for it anew behind it.
        let movies = || -> Vec<String> {
            let mut asked = queries(&log).split_off(before);
            asked.retain(|query| !has_param(query, "isFavorite", "true"));
            asked
        };
        wait_until("the Movies request", || !movies().is_empty()).await?;
        if let Some(query) = movies()
            .iter()
            .find(|query| !has_param(query, "genres", "Comedy"))
        {
            return Err(format!("the genre was not kept: {query}"));
        }
        if selected(&window, "All", AccessibleRole::Button) != Some(true) {
            return Err("the All filter was not kept".into());
        }
        wait_role(&window, "Genre: Comedy", AccessibleRole::Button).await?;

        press(&window, "Settings", AccessibleRole::Button).await?;
        wait_until("the settings", || window.get_screen() == Screen::Settings).await?;
        if selected(&window, "Movies", AccessibleRole::Tab) != Some(false) {
            return Err("Movies is still the underlined tab in the settings".into());
        }
        app.dispatch(Msg::CloseSettings);
        wait_until("the library", || window.get_screen() == Screen::Library).await?;

        press(&window, "Shows", AccessibleRole::Tab).await?;
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;
        let notice = window.get_notice();
        if !notice.visible || notice.message != Message::SessionExpired {
            return Err(format!("unexpected notice: {notice:?}"));
        }
        Ok(())
    });
}
