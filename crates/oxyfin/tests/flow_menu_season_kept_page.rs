//! "Go to season" opens the series on that season also while a page of the same series is kept
//! below, which stays as it was left.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use common::menu::{choose, right_click, wait_menu};
use common::{
    fixture, json, mount_me, pause, run, save_session, start_with, wait_role, wait_until,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{AppWindow, ContextMenu, Screen};
use slint::{ComponentHandle, Model};
use wiremock::matchers::{body_json, method, path, query_param};
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

/// The resume list: the episode until it is taken out.
struct Resume {
    hidden: Arc<AtomicBool>,
}

impl Respond for Resume {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        if self.hidden.load(Ordering::SeqCst) {
            return json(EMPTY);
        }
        json(
            serde_json::json!({"Items": [episode()], "TotalRecordCount": 1, "StartIndex": 0})
                .to_string(),
        )
    }
}

struct Hide {
    hidden: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}

impl Respond for Hide {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.hidden.store(true, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
        json(r#"{"PlaybackPositionTicks":0,"PlayCount":1,"IsFavorite":false,"Played":false}"#)
    }
}

async fn mount(mock: MockServer, hidden: Arc<AtomicBool>, calls: Arc<AtomicUsize>) -> MockServer {
    mount_me(&mock).await;
    let get = |route: String, body: String| {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body))
    };
    get("/UserViews".into(), fixture("user_views"))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/UserItems/Resume"))
        .respond_with(Resume {
            hidden: Arc::clone(&hidden),
        })
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
    get(format!("/Shows/{SERIES}/Seasons"), fixture("seasons"))
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
    Mock::given(method("POST"))
        .and(path(format!("/UserItems/{EPISODE}/UserData")))
        .and(body_json(serde_json::json!({"PlaybackPositionTicks": 0})))
        .respond_with(Hide { hidden, calls })
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

#[test]
fn go_to_season_opens_that_season_above_a_kept_page() {
    let hidden = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let flow = start_with(move |mock| mount(mock, hidden, calls), save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let card = wait_role(&window, "Sword", AccessibleRole::Button).await?;
        right_click(&window, &card);
        wait_menu(&window, "Northern Saga").await?;
        choose(&window, "Go to series")?;
        wait_until("the first season", || shows_season(&window, 0)).await?;

        // A page over the series, where an episode of it has a menu.
        window.invoke_series_open_person("someone".into());
        wait_until("the person page", || window.get_screen() == Screen::Person).await?;
        // The change of screen closes the menus, on the next turn.
        pause(Duration::from_millis(200)).await;
        window.global::<ContextMenu>().invoke_show(
            EPISODE.into(),
            false,
            200.,
            200.,
            false,
            "Sword".into(),
            "".into(),
        );
        wait_menu(&window, "Northern Saga").await?;
        choose(&window, "Go to season 2")?;
        wait_until("the second season", || shows_season(&window, 1)).await?;

        window.invoke_back();
        wait_until("the person page again", || {
            window.get_screen() == Screen::Person
        })
        .await?;
        window.invoke_back();
        wait_until("the series page left first", || {
            window.get_screen() == Screen::Series && window.get_series_loaded()
        })
        .await?;
        if !shows_season(&window, 0) {
            return Err(format!(
                "the page below moved to season index {}",
                window.get_series_season()
            ));
        }
        Ok(())
    });
}
