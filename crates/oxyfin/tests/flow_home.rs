//! Restores a session and shows the home screen, with one library failing to load.

mod common;

use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{fixture, json, mount_me, run, save_session, start_with, wait_until};
use image::{ImageFormat, Rgba, RgbaImage};
use oxyfin::state::Msg;
use oxyfin::{AppWindow, HomeRow, HomeRowKind, Screen};
use slint::{ComponentHandle, Model};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const MOVIES: &str = "f137a2dd21bbc1b99aa5c0f6bf02a805";
const SHOWS: &str = "a656b907eb3a73532e40e44b968d0225";
const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";

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

fn png() -> Vec<u8> {
    let image = RgbaImage::from_pixel(4, 6, Rgba([200, 100, 50, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap_or_else(|err| panic!("cannot encode a PNG: {err}"));
    bytes.into_inner()
}

async fn mount_home(mock: MockServer, resume_calls: Arc<AtomicUsize>) -> MockServer {
    mount_me(&mock).await;
    Mock::given(method("GET"))
        .and(path("/UserViews"))
        .respond_with(json(fixture("user_views")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/UserItems/Resume"))
        .respond_with(Counted {
            calls: resume_calls,
            response: json(fixture("user_items_resume")),
        })
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Shows/NextUp"))
        .respond_with(json(r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items/Latest"))
        .and(query_param("parentId", MOVIES))
        .respond_with(json(fixture("items_latest")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items/Latest"))
        .and(query_param("parentId", SHOWS))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{MOVIE}/Images/Primary")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(png(), "image/png"))
        .mount(&mock)
        .await;
    mock
}

fn rows(window: &AppWindow) -> Vec<HomeRow> {
    window.get_home_rows().iter().collect()
}

fn titles(window: &AppWindow) -> Vec<String> {
    rows(window)
        .iter()
        .map(|row| row.title.to_string())
        .collect()
}

fn row(window: &AppWindow, title: &str) -> Option<HomeRow> {
    rows(window).into_iter().find(|row| row.title == title)
}

#[test]
fn home_shows_every_row_that_loads() {
    let resume_calls = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&resume_calls);
    let flow = start_with(move |mock| mount_home(mock, calls), save_session);
    let window = flow.window.as_weak();
    let app = flow.app.clone();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_until("every row", || {
            titles(&window)
                == [
                    "Continue Watching",
                    "Recently Added: Movies",
                    "Recently Added: Shows",
                ]
        })
        .await
        .map_err(|error| format!("{error}: {:?}", titles(&window)))?;

        let resume = row(&window, "Continue Watching").ok_or("no Continue Watching")?;
        let resumed: Vec<_> = resume.items.iter().map(|card| card.title).collect();
        if resume.kind != HomeRowKind::Landscape || resumed != ["Test Movie (2020)", "Test Show"] {
            return Err(format!("unexpected Continue Watching row: {resumed:?}"));
        }

        let shows = row(&window, "Recently Added: Shows").ok_or("no Shows row")?;
        if shows.error.is_empty() || shows.items.row_count() != 0 {
            return Err("the failed library row shows no error".into());
        }

        wait_until("the poster image", || {
            row(&window, "Recently Added: Movies")
                .and_then(|row| row.items.row_data(0))
                .is_some_and(|card| card.has_image)
        })
        .await?;
        let movies = row(&window, "Recently Added: Movies").ok_or("no Movies row")?;
        if movies.kind != HomeRowKind::Poster || !movies.error.is_empty() {
            return Err("unexpected Movies row".into());
        }

        wait_until("the banner", || window.get_home_banner().has).await?;
        let banner = window.get_home_banner();
        if banner.title != "Test Movie (2020)" || banner.year != 2020 {
            return Err(format!("unexpected banner: {}", banner.title));
        }
        window.invoke_play(banner.card.id.clone());

        app.dispatch(Msg::OpenPlayer);
        app.dispatch(Msg::ClosePlayer);
        wait_until("the reload", || resume_calls.load(Ordering::SeqCst) == 2).await?;
        wait_until("the rows after the reload", || titles(&window).len() == 3).await?;
        Ok(())
    });
}
