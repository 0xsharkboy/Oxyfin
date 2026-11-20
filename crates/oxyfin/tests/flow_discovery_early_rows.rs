//! A found server is listed as soon as discovery ends, before its version arrives.

mod common;

use std::time::Duration;

use common::discovery::{reply, responder};
use common::{SERVER_VERSION, fixture, json, run, start, wait_until};
use oxyfin::Screen;
use slint::{ComponentHandle, Model};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

#[test]
fn rows_show_before_their_version() {
    let flow = start(|mock: MockServer| async move {
        Mock::given(method("GET"))
            .and(path("/System/Info/Public"))
            .respond_with(json(fixture("system_info_public")).set_delay(Duration::from_millis(800)))
            .mount(&mock)
            .await;
        mock
    });
    let uri = flow.mock.uri();
    flow.app
        .set_discovery_targets(vec![responder(Some(reply(&uri)))]);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;
        wait_until("the found server", || {
            window.get_found_servers().row_count() == 1
        })
        .await?;
        let detail = |window: &oxyfin::AppWindow| {
            window
                .get_found_servers()
                .row_data(0)
                .map(|row| row.detail.to_string())
                .unwrap_or_default()
        };
        if detail(&window) != uri {
            return Err(format!(
                "the row waited for its version: {}",
                detail(&window)
            ));
        }
        let versioned = format!("{uri} · {SERVER_VERSION}");
        wait_until("the version", || detail(&window) == versioned).await?;
        Ok(())
    });
}
