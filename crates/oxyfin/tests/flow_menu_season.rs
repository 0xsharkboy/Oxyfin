//! The context menu of a season pill and of an episode row on the series page.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use common::menu::{choose, is_open, right_click, wait_menu};
use common::{
    click, fixture, json, mount_browse_basics, pause, run, save_session, start_with, wait_role,
    wait_until,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::Screen;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";
const SEASON_1: &str = "0407e3d02b7d70abe4a2d37241b02fea";
const SEASON_2: &str = "1507e3d02b7d70abe4a2d37241b02feb";
const EPISODE: &str = "a2000000000000000000000000000005";

/// Season 2 as the server holds it, played once marked.
struct Season {
    played: Arc<AtomicBool>,
}

impl Respond for Season {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        json(
            serde_json::json!({
                "Id": SEASON_2, "Name": "Season 2", "Type": "Season", "IndexNumber": 2,
                "SeriesId": SERIES, "SeriesName": "Test Show",
                "UserData": {"Played": self.played.load(Ordering::SeqCst), "IsFavorite": false,
                    "PlaybackPositionTicks": 0, "PlayCount": 0}
            })
            .to_string(),
        )
    }
}

struct MarkPlayed {
    played: Arc<AtomicBool>,
}

impl Respond for MarkPlayed {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.played.store(true, Ordering::SeqCst);
        json(r#"{"PlaybackPositionTicks":0,"PlayCount":0,"IsFavorite":false,"Played":true}"#)
    }
}

/// Counts the episode lists asked for season 2.
struct Episodes {
    calls: Arc<AtomicUsize>,
}

impl Respond for Episodes {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.calls.fetch_add(1, Ordering::SeqCst);
        json(fixture("episodes_season_2"))
    }
}

async fn mount(mock: MockServer, played: Arc<AtomicBool>, calls: Arc<AtomicUsize>) -> MockServer {
    mount_browse_basics(&mock, &fixture("next_up_series")).await;
    let get = |route: String, body: String| {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body))
    };
    get("/Items".into(), fixture("items_shows"))
        .mount(&mock)
        .await;
    get(format!("/Items/{SERIES}"), fixture("item_series"))
        .mount(&mock)
        .await;
    get(format!("/Shows/{SERIES}/Seasons"), fixture("seasons"))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Shows/{SERIES}/Episodes")))
        .and(query_param("seasonId", SEASON_1))
        .respond_with(json(fixture("episodes_season_1")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Shows/{SERIES}/Episodes")))
        .respond_with(Episodes { calls })
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{SEASON_2}")))
        .respond_with(Season {
            played: Arc::clone(&played),
        })
        .mount(&mock)
        .await;
    let mut episode: serde_json::Value =
        serde_json::from_str(&fixture("episodes_season_2")).unwrap_or_default();
    let episode = episode["Items"]
        .as_array_mut()
        .and_then(|items| items.iter().find(|item| item["Id"] == EPISODE).cloned())
        .unwrap_or_default();
    get(format!("/Items/{EPISODE}"), episode.to_string())
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/UserPlayedItems/{SEASON_2}")))
        .respond_with(MarkPlayed { played })
        .mount(&mock)
        .await;
    mock
}

/// Waits for the episode row `title`, scrolling the page a little so rows below the window
/// are laid out.
async fn wait_row(window: &oxyfin::AppWindow, title: &str) -> Result<ElementHandle, String> {
    let start = std::time::Instant::now();
    let bottom = window.window().size().height as f32 / window.window().scale_factor();
    loop {
        window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(640., 400.),
                delta_x: 0.,
                delta_y: -40.,
            });
        if let Some(row) = ElementHandle::find_by_accessible_label(window, title).find(|row| {
            row.size().height > 0.
                && row.absolute_position().y + row.size().height < bottom
                && row.accessible_role() == Some(AccessibleRole::Button)
        }) {
            return Ok(row);
        }
        if start.elapsed() > Duration::from_secs(5) {
            return Err(format!("timed out waiting for the row {title}"));
        }
        pause(Duration::from_millis(10)).await;
    }
}

#[test]
fn a_season_menu_marks_the_season_watched() {
    let played = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let listed = Arc::clone(&calls);
    let flow = start_with(move |mock| mount(mock, played, listed), save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let tab = wait_role(&window, "Shows", AccessibleRole::Tab).await?;
        click(&window, &tab);
        let card = wait_role(&window, "Test Show", AccessibleRole::Button).await?;
        click(&window, &card);
        wait_until("the series page", || window.get_screen() == Screen::Series).await?;
        let row = wait_row(&window, "The Pact").await?;

        right_click(&window, &row);
        let menu = window.global::<oxyfin::ContextMenu>();
        let rows = wait_menu(&window, "Test Show").await.map_err(|error| {
            format!(
                "{error}: open {}, id {}, ready {}, row at {:?}",
                menu.get_open(),
                menu.get_id(),
                menu.get_ready_serial(),
                row.absolute_position()
            )
        })?;
        let expected = [
            "Resume from 20:00",
            "Play from the beginning",
            "Mark as watched",
            "Add to favorites",
        ];
        if rows != expected {
            return Err(format!("episode rows {rows:?}, expected {expected:?}"));
        }
        common::person::press_escape(&window);
        if is_open(&window) {
            return Err("Escape leaves the menu open".into());
        }
        if window.get_screen() != Screen::Series {
            return Err("Escape left the series page".into());
        }

        let pill = wait_role(&window, "Season 2", AccessibleRole::Button).await?;
        right_click(&window, &pill);
        let rows = wait_menu(&window, "Test Show").await?;
        if rows != ["Play season", "Mark season as watched"] {
            return Err(format!("season rows {rows:?}"));
        }
        let before = calls.load(Ordering::SeqCst);
        choose(&window, "Mark season as watched")?;
        if is_open(&window) {
            return Err("the menu stays open after a row".into());
        }
        wait_until("the episodes to reload", || {
            calls.load(Ordering::SeqCst) > before
        })
        .await?;

        let pill = wait_role(&window, "Season 2", AccessibleRole::Button).await?;
        right_click(&window, &pill);
        let rows = wait_menu(&window, "Test Show").await?;
        if rows != ["Play season", "Mark season as unwatched"] {
            return Err(format!("the menu does not follow the change: {rows:?}"));
        }
        #[cfg(not(feature = "mpv"))]
        {
            choose(&window, "Play season")?;
            wait_until("the playback notice", || {
                window.get_notice().visible
                    && window.get_notice().message == oxyfin::Message::PlaybackUnavailable
            })
            .await?;
        }
        Ok(())
    });
}
