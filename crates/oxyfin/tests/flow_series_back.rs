//! A series page left before its episodes or their thumbnails came finishes loading them when
//! Back shows it again.

mod common;

use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use common::person::{press_escape, reveal};
use common::{click, fixture, json, mount_browse_basics, run, save_session, start_with};
use common::{wait_until, wait_visible};
use image::{ImageFormat, Rgba, RgbaImage};
use oxyfin::Screen;
use slint::{ComponentHandle, Model};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";
const OTHER: &str = "9ea196e7ccc3ae17d2c3f75c3ab34407";
const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

fn png() -> Vec<u8> {
    let image = RgbaImage::from_pixel(4, 6, Rgba([200, 100, 50, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap_or_else(|err| panic!("cannot encode a PNG: {err}"));
    bytes.into_inner()
}

/// Answers with `response`, after `delay` while `slow` holds, and counts the calls.
struct Counted {
    calls: Arc<AtomicUsize>,
    response: ResponseTemplate,
    slow: Arc<AtomicBool>,
    delay: Duration,
}

impl Respond for Counted {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let response = self.response.clone();
        if self.slow.load(Ordering::SeqCst) {
            response.set_delay(self.delay)
        } else {
            response
        }
    }
}

#[derive(Clone, Default)]
struct Calls {
    episodes: Arc<AtomicUsize>,
    thumbnails: Arc<AtomicUsize>,
    slow_episodes: Arc<AtomicBool>,
}

async fn mount(mock: &MockServer, calls: Calls) {
    mount_browse_basics(mock, EMPTY).await;
    let similar = format!(
        r#"{{"Items":[{{"Id":"{OTHER}","Name":"Other Show","Type":"Series","ProductionYear":2022}}],
        "TotalRecordCount":1,"StartIndex":0}}"#
    );
    let episodes = fixture("episodes_season_1")
        .replace(r#""ImageTags": {}"#, r#""ImageTags": {"Primary": "t1"}"#);
    let routes = [
        ("/Items".to_owned(), fixture("items_shows")),
        (format!("/Items/{SERIES}"), fixture("item_series")),
        (format!("/Items/{SERIES}/Similar"), similar),
        (format!("/Items/{OTHER}/Similar"), EMPTY.to_owned()),
        (format!("/Shows/{SERIES}/Seasons"), fixture("seasons")),
        (format!("/Shows/{OTHER}/Seasons"), fixture("seasons")),
        (
            format!("/Shows/{OTHER}/Episodes"),
            fixture("episodes_season_1"),
        ),
    ];
    for (route, body) in routes {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body))
            .mount(mock)
            .await;
    }
    // The other series fails: a loaded series would mark the page below stale and reload it.
    Mock::given(method("GET"))
        .and(path(format!("/Items/{OTHER}")))
        .respond_with(ResponseTemplate::new(500))
        .mount(mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Shows/{SERIES}/Episodes")))
        .respond_with(Counted {
            calls: calls.episodes,
            response: json(episodes),
            slow: calls.slow_episodes,
            delay: Duration::from_millis(4000),
        })
        .mount(mock)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/Items/a10+[12]/Images/Primary$"))
        .respond_with(Counted {
            calls: calls.thumbnails,
            response: ResponseTemplate::new(200)
                .set_body_raw(png(), "image/png")
                .set_delay(Duration::from_millis(800)),
            slow: Arc::new(AtomicBool::new(false)),
            delay: Duration::ZERO,
        })
        .mount(mock)
        .await;
}

fn count(calls: &Arc<AtomicUsize>) -> usize {
    calls.load(Ordering::SeqCst)
}

#[test]
fn series_restored_on_back_finishes_loading() {
    let calls = Calls::default();
    calls.slow_episodes.store(true, Ordering::SeqCst);
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
        let rows = || window.get_series_episodes();
        let thumbnails = || {
            rows()
                .iter()
                .filter(|row| row.image.size().width > 0)
                .count()
        };
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        let shows = common::wait_role(
            &window,
            "Shows",
            i_slint_backend_testing::AccessibleRole::Tab,
        )
        .await?;
        click(&window, &shows);
        let card = wait_visible(&window, "Test Show").await?;
        click(&window, &card);
        wait_until("the series", || window.get_series_loaded()).await?;

        // Left before its episodes come: Back asks for them again and shows them.
        let other = reveal(&window, "Other Show").await?;
        if rows().row_count() != 0 {
            return Err(format!(
                "the episodes came before the page was left: {} calls, {} rows",
                count(&calls.episodes),
                rows().row_count()
            ));
        }
        click(&window, &other);
        wait_until("the other series", || !window.get_series_error().is_empty()).await?;
        calls.slow_episodes.store(false, Ordering::SeqCst);
        let asked = count(&calls.episodes);
        press_escape(&window);
        wait_until("the episodes asked again and shown", || {
            window.get_series_data().title == "Test Show"
                && count(&calls.episodes) > asked
                && rows().row_count() == 2
        })
        .await?;

        let asked = count(&calls.episodes);
        // Left while the thumbnails load: Back loads them again.
        wait_until("the thumbnails asked", || count(&calls.thumbnails) == 2).await?;
        let other = reveal(&window, "Other Show").await?;
        click(&window, &other);
        wait_until("the other series", || !window.get_series_error().is_empty()).await?;
        press_escape(&window);
        wait_until("the series again", || {
            window.get_series_data().title == "Test Show" && rows().row_count() == 2
        })
        .await?;
        if count(&calls.episodes) != asked {
            return Err("the episodes were asked again although they had come".into());
        }
        wait_until("the thumbnails shown", || thumbnails() == 2).await?;
        Ok(())
    });
}
