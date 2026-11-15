//! The In progress filter of the shows page asks for the series that have a next episode.

mod common;

use std::sync::{Arc, Mutex};

use common::{
    QueryLog, click, fixture, has_param, json, mount_browse_basics, run, save_session, start_with,
    wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{AppWindow, Screen};
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";

struct Items {
    log: QueryLog,
}

impl Respond for Items {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query = request.url.query().unwrap_or("").to_owned();
        let has_ids = query.split('&').any(|pair| pair.starts_with("ids="));
        self.log.lock().map(|mut log| log.push(query)).ok();
        if has_ids {
            json(fixture("items_shows"))
        } else {
            json(r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#)
        }
    }
}

async fn mount(mock: MockServer, log: QueryLog) -> MockServer {
    mount_browse_basics(&mock, &fixture("shows_next_up")).await;
    Mock::given(method("GET"))
        .and(path("/Items"))
        .respond_with(Items { log })
        .mount(&mock)
        .await;
    mock
}

async fn press(window: &AppWindow, label: &str, role: AccessibleRole) -> Result<(), String> {
    let element = wait_role(window, label, role).await?;
    click(window, &element);
    Ok(())
}

#[test]
fn in_progress_shows_the_series_with_a_next_episode() {
    let log: QueryLog = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::clone(&log);
    let flow = start_with(move |mock| mount(mock, served), save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        press(&window, "Shows", AccessibleRole::Tab).await?;
        wait_visible(&window, "Nothing here yet").await?;
        press(&window, "In progress", AccessibleRole::Button).await?;
        wait_visible(&window, "Test Show").await?;
        wait_visible(&window, "1 title").await?;
        let query = log
            .lock()
            .map(|log| log.last().cloned().unwrap_or_default())
            .unwrap_or_default();
        if !query.contains(SERIES) || !query.contains("includeItemTypes=Series") {
            return Err(format!("unexpected query: {query}"));
        }
        if has_param(&query, "isPlayed", "false") {
            return Err("In progress must not filter on played".into());
        }
        Ok(())
    });
}
