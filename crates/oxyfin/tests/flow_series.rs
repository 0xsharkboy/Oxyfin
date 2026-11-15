//! Opens a series from Shows: the main button, the season pills, the episode rows and stale answers.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    QueryLog, click, exists, fixture, has_param, json, mount_browse_basics, pause, run,
    save_session, start_with, wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::Screen;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";
const SEASON_1: &str = "0407e3d02b7d70abe4a2d37241b02fea";
const SEASON_2: &str = "1507e3d02b7d70abe4a2d37241b02feb";
const PLAYED: &str =
    r#"{"PlaybackPositionTicks":0,"PlayCount":1,"IsFavorite":false,"Played":true}"#;
const FAVORED: &str =
    r#"{"PlaybackPositionTicks":0,"PlayCount":1,"IsFavorite":true,"Played":true}"#;

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

/// Answers `Items/<id>`: the second answer is slow, like a reload started by a toggle.
struct SeriesItem {
    calls: Arc<AtomicUsize>,
}

impl Respond for SeriesItem {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let response = json(fixture("item_series"));
        if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
            response.set_delay(Duration::from_millis(2000))
        } else {
            response
        }
    }
}

/// Answers `Shows/<id>/Episodes`: Season 2 is fast the first time, slow the second
/// time and fails afterwards.
struct Episodes {
    log: QueryLog,
    season_2_calls: Arc<Mutex<usize>>,
}

impl Respond for Episodes {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query = request.url.query().unwrap_or("").to_owned();
        self.log.lock().map(|mut log| log.push(query.clone())).ok();
        if has_param(&query, "seasonId", SEASON_1) {
            return json(fixture("episodes_season_1"));
        }
        let call = self
            .season_2_calls
            .lock()
            .map(|mut calls| {
                *calls += 1;
                *calls
            })
            .unwrap_or(1);
        let response = json(fixture("episodes_season_2"));
        match call {
            1 => response,
            2 => response.set_delay(Duration::from_millis(900)),
            _ => ResponseTemplate::new(500),
        }
    }
}

fn episode_requests(log: &QueryLog) -> Vec<String> {
    log.lock()
        .map(|log| {
            log.iter()
                .filter(|query| query.contains("seasonId="))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Waits for the episode row `title`. Rows that arrive after the list was hidden are only
/// instantiated once the window handles an event, which a small scroll provides.
async fn wait_row(window: &oxyfin::AppWindow, title: &str) -> Result<ElementHandle, String> {
    let start = std::time::Instant::now();
    loop {
        window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(640., 400.),
                delta_x: 0.,
                delta_y: -1.,
            });
        if let Some(row) = ElementHandle::find_by_accessible_label(window, title).find(|row| {
            row.size().height > 0. && row.accessible_role() == Some(AccessibleRole::Button)
        }) {
            return Ok(row);
        }
        if start.elapsed() > Duration::from_secs(10) {
            return Err(format!("timed out waiting for the row {title}"));
        }
        pause(Duration::from_millis(10)).await;
    }
}

fn scroll_down(window: &oxyfin::AppWindow) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: LogicalPosition::new(640., 400.),
            delta_x: 0.,
            delta_y: -600.,
        });
}

fn vertical_centre(element: &ElementHandle) -> f32 {
    element.absolute_position().y + element.size().height / 2.
}

/// The element with `id` that lies inside the row spanning `top..bottom`.
fn inside(window: &oxyfin::AppWindow, id: &str, top: f32, bottom: f32) -> Option<ElementHandle> {
    ElementHandle::find_by_element_id(window, id).find(|element| {
        let y = element.absolute_position().y;
        y >= top && y + element.size().height <= bottom
    })
}

#[test]
fn a_series_opens_with_seasons_and_episodes() {
    let log: QueryLog = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::clone(&log);
    let calls = Arc::new(Mutex::new(0));
    let posted = Arc::new(AtomicUsize::new(0));
    let played_calls = Arc::clone(&posted);
    let favored = Arc::new(AtomicUsize::new(0));
    let favored_calls = Arc::clone(&favored);
    let series_calls = Arc::new(AtomicUsize::new(0));
    let series_served = Arc::clone(&series_calls);
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, &fixture("next_up_series")).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(json(fixture("items_shows")))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Items/{SERIES}")))
                .respond_with(SeriesItem {
                    calls: series_served,
                })
                .mount(&mock)
                .await;
            Mock::given(method("POST"))
                .and(path(format!("/UserFavoriteItems/{SERIES}")))
                .respond_with(Counted {
                    calls: favored_calls,
                    response: json(FAVORED),
                })
                .mount(&mock)
                .await;
            Mock::given(method("POST"))
                .and(path(format!("/UserPlayedItems/{SERIES}")))
                .respond_with(Counted {
                    calls: played_calls,
                    response: json(PLAYED),
                })
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Shows/{SERIES}/Seasons")))
                .respond_with(json(fixture("seasons")))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Shows/{SERIES}/Episodes")))
                .respond_with(Episodes {
                    log: served,
                    season_2_calls: calls,
                })
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
        let shows = wait_role(&window, "Shows", AccessibleRole::Tab).await?;
        click(&window, &shows);
        let card = wait_visible(&window, "Test Show").await?;
        click(&window, &card);

        wait_until("the series", || window.get_screen() == Screen::Series).await?;
        for label in ["Mystery", "Thriller", "2021", "2 seasons", "7.9", "TV-MA"] {
            wait_visible(&window, label).await?;
        }
        wait_visible(&window, "A generated test show.").await?;
        wait_visible(&window, "Starring Gia Gate, Hal Hash").await?;
        wait_role(&window, "Resume S02E05", AccessibleRole::Button).await?;

        scroll_down(&window);
        let season_2 = wait_role(&window, "Season 2", AccessibleRole::Button).await?;
        if season_2.accessible_checked() != Some(true) {
            return Err("Season 2 is not the chosen pill".into());
        }
        let season_1 = wait_role(&window, "Season 1", AccessibleRole::Button).await?;
        if season_1.accessible_checked() != Some(false) {
            return Err("Season 1 is chosen too".into());
        }
        let lighthouse = wait_role(&window, "The Lighthouse", AccessibleRole::Button).await?;
        let pact = wait_role(&window, "The Pact", AccessibleRole::Button).await?;
        if exists(&window, "Pilot Light") {
            return Err("an episode of another season is shown".into());
        }
        for row in [&lighthouse, &pact] {
            let (top, bottom) = (
                row.absolute_position().y,
                row.absolute_position().y + row.size().height,
            );
            if row.size().height <= 0. {
                return Err("an episode row has no height".into());
            }
            let thumb = inside(&window, "EpisodeRow::thumb", top, bottom)
                .ok_or("an episode row has no thumbnail inside it")?;
            let title = inside(&window, "EpisodeRow::title-text", top, bottom)
                .ok_or("an episode row has no title inside it")?;
            let (middle, row_middle) = (vertical_centre(&thumb), (top + bottom) / 2.);
            if (middle - row_middle).abs() > 2. {
                return Err(format!(
                    "the thumbnail is centred at {middle} in a row centred at {row_middle}"
                ));
            }
            let title_middle = vertical_centre(&title);
            let thumb_top = thumb.absolute_position().y;
            if title_middle < thumb_top || title_middle > thumb_top + thumb.size().height {
                return Err("the title is not level with the thumbnail".into());
            }
        }

        click(&window, &season_1);
        wait_row(&window, "Pilot Light").await?;
        wait_row(&window, "Second Watch").await?;
        if exists(&window, "The Pact") {
            return Err("the rows of Season 2 are still shown".into());
        }
        let requests = episode_requests(&log);
        if !requests
            .last()
            .is_some_and(|query| has_param(query, "seasonId", SEASON_1))
        {
            return Err(format!("no request for Season 1: {requests:?}"));
        }
        let season_1 = wait_role(&window, "Season 1", AccessibleRole::Button).await?;
        if season_1.accessible_checked() != Some(true) {
            return Err("Season 1 is not the chosen pill after the click".into());
        }

        // A slow answer for a season that was left must not replace the chosen one.
        let season_2 = wait_role(&window, "Season 2", AccessibleRole::Button).await?;
        click(&window, &season_2);
        wait_until("the slow request", || {
            episode_requests(&log)
                .last()
                .is_some_and(|query| has_param(query, "seasonId", SEASON_2))
        })
        .await?;
        wait_visible(&window, "Loading").await?;
        if exists(&window, "Pilot Light") || exists(&window, "Second Watch") {
            return Err("the rows of Season 1 stay under the Season 2 pill".into());
        }
        let season_1 = wait_role(&window, "Season 1", AccessibleRole::Button).await?;
        click(&window, &season_1);
        wait_row(&window, "Pilot Light").await?;
        pause(Duration::from_millis(1300)).await;
        if exists(&window, "The Pact") {
            return Err("an answer for a season that was left replaced the rows".into());
        }
        wait_row(&window, "Second Watch").await?;

        // Marking the series watched reloads the episodes.
        let before = episode_requests(&log).len();
        window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(640., 400.),
                delta_x: 0.,
                delta_y: 2000.,
            });
        let watched = wait_role(&window, "Mark as watched", AccessibleRole::Button).await?;
        click(&window, &watched);
        wait_until("the watched request", || posted.load(Ordering::SeqCst) == 1).await?;
        // A favorite change while the series reload is in flight must not drop that reload.
        wait_until("the series reload", || {
            series_calls.load(Ordering::SeqCst) == 2
        })
        .await?;
        let favorite = wait_role(&window, "Add to favorites", AccessibleRole::Button).await?;
        click(&window, &favorite);
        wait_until("the favorite request", || {
            favored.load(Ordering::SeqCst) == 1
        })
        .await?;
        wait_until("the series reloaded again", || {
            series_calls.load(Ordering::SeqCst) == 3
        })
        .await?;
        wait_until("the episodes reload", || {
            episode_requests(&log).len() > before
        })
        .await?;

        // A season whose episodes fail does not show the rows of another season.
        scroll_down(&window);
        let season_2 = wait_role(&window, "Season 2", AccessibleRole::Button).await?;
        click(&window, &season_2);
        wait_role(&window, "Retry", AccessibleRole::Button).await?;
        if exists(&window, "Pilot Light") || exists(&window, "Second Watch") {
            return Err("the rows of Season 1 are shown after Season 2 failed".into());
        }
        let season_1 = wait_role(&window, "Season 1", AccessibleRole::Button).await?;
        click(&window, &season_1);
        wait_row(&window, "Second Watch").await?;
        scroll_down(&window);

        #[cfg(not(feature = "mpv"))]
        {
            let row = wait_role(&window, "Second Watch", AccessibleRole::Button).await?;
            click(&window, &row);
            wait_until("the playback notice", || {
                window.get_notice().visible
                    && window.get_notice().message == oxyfin::Message::PlaybackUnavailable
            })
            .await?;
        }
        Ok(())
    });
}
