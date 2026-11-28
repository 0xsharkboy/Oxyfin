//! "Go to season" reaches a season that the last answer for the series, shown at once, lacks.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::menu::{choose, right_click, wait_menu};
use common::{
    fixture, json, mount_me, pause, run, save_session, start_with, wait_role, wait_until,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::state::{Msg, Tab};
use oxyfin::{AppWindow, Screen};
use slint::{ComponentHandle, Model};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";
const SEASON_1: &str = "0407e3d02b7d70abe4a2d37241b02fea";
const SEASON_2: &str = "1507e3d02b7d70abe4a2d37241b02feb";
const EPISODE: &str = "a2000000000000000000000000000005";
const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

/// Episode 5 of season 2, 24 minutes long, 12 of them watched.
fn episode() -> serde_json::Value {
    serde_json::json!({
        "Name": "Sword", "Id": EPISODE, "Type": "Episode", "IsFolder": false,
        "RunTimeTicks": 14_400_000_000_i64, "ParentIndexNumber": 2, "IndexNumber": 5,
        "SeriesName": "Northern Saga", "SeriesId": SERIES, "SeasonId": SEASON_2,
        "UserData": {"PlayedPercentage": 50, "PlaybackPositionTicks": 7_200_000_000_i64,
            "PlayCount": 1, "IsFavorite": false, "Played": false}
    })
}

/// How long the answer with both seasons takes.
const SLOW: Duration = Duration::from_millis(600);

/// The seasons: only the first one until the server gets the second.
struct Seasons {
    both: Arc<AtomicBool>,
}

impl Respond for Seasons {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let mut page: serde_json::Value =
            serde_json::from_str(&fixture("seasons")).unwrap_or_default();
        if self.both.load(Ordering::SeqCst) {
            return json(page.to_string()).set_delay(SLOW);
        }
        if let Some(items) = page["Items"].as_array_mut() {
            items.retain(|season| season["Id"] == SEASON_1);
        }
        json(page.to_string())
    }
}

async fn mount(mock: MockServer, both: Arc<AtomicBool>) -> MockServer {
    mount_me(&mock).await;
    let get = |route: String, body: String| {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body))
    };
    get("/UserViews".into(), fixture("user_views"))
        .mount(&mock)
        .await;
    get(
        "/UserItems/Resume".into(),
        serde_json::json!({"Items": [episode()], "TotalRecordCount": 1, "StartIndex": 0})
            .to_string(),
    )
    .mount(&mock)
    .await;
    get("/Shows/NextUp".into(), EMPTY.into()).mount(&mock).await;
    get("/Items/Latest".into(), "[]".into()).mount(&mock).await;
    get(format!("/Items/{EPISODE}"), episode().to_string())
        .mount(&mock)
        .await;
    get(format!("/Items/{SERIES}"), fixture("item_series"))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Shows/{SERIES}/Seasons")))
        .respond_with(Seasons { both })
        .mount(&mock)
        .await;
    for (season, body) in [
        (SEASON_1, "episodes_season_1"),
        (SEASON_2, "episodes_season_2"),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/Shows/{SERIES}/Episodes")))
            .and(query_param("seasonId", season))
            .respond_with(json(fixture(body)))
            .mount(&mock)
            .await;
    }
    get(
        format!("/Shows/{SERIES}/Episodes"),
        fixture("episodes_season_1"),
    )
    .mount(&mock)
    .await;
    mock
}

/// Whether the series page shows season `index`, its episodes loaded.
fn shows_season(window: &AppWindow, index: i32) -> bool {
    window.get_screen() == Screen::Series
        && window.get_series_loaded()
        && window.get_series_season() == index
        && !window.get_series_episodes_stale()
        && window.get_series_episodes().row_count() > 0
}

/// Chooses `row` in the menu of the episode of season 2, from the home screen.
async fn from_home(window: &AppWindow, row: &str) -> Result<(), String> {
    wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
    // The change of screen closes the menus, on the next turn.
    pause(Duration::from_millis(200)).await;
    let card = wait_role(window, "Sword", AccessibleRole::Button).await?;
    right_click(window, &card);
    wait_menu(window, "Northern Saga").await?;
    choose(window, row)
}

#[test]
fn go_to_season_reaches_a_season_the_last_answer_lacks() {
    let both = Arc::new(AtomicBool::new(false));
    let served = Arc::clone(&both);
    let flow = start_with(move |mock| mount(mock, served), save_session);
    let window = flow.window.as_weak();
    let app = std::rc::Rc::clone(&flow.app);

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        let only_season =
            || shows_season(&window, 0) && window.get_series_seasons().row_count() == 1;
        for chooses in [true, false] {
            both.store(false, Ordering::SeqCst);
            from_home(&window, "Go to series").await?;
            wait_until("the only season", only_season).await?;
            app.dispatch(Msg::OpenTab(Tab::Home));
            both.store(true, Ordering::SeqCst);
            from_home(&window, "Go to season 2").await?;
            if chooses {
                wait_until("the last answer", only_season).await?;
                window.invoke_series_select_season(0);
            }
            wait_until("both seasons", || {
                window.get_series_seasons().row_count() == 2
            })
            .await?;
            let expected = if chooses { 0 } else { 1 };
            wait_until("the season wanted", || shows_season(&window, expected)).await?;
            app.dispatch(Msg::OpenTab(Tab::Home));
        }
        Ok(())
    });
}
