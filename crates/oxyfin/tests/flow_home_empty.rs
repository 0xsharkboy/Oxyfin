//! Restores a session on a server with nothing to show and expects the empty home message.

mod common;

use common::{fixture, json, mount_me, run, save_session, start_with, wait_until};
use i_slint_backend_testing::ElementHandle;
use oxyfin::Screen;
use slint::{ComponentHandle, Model};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

async fn mount_empty(mock: MockServer) -> MockServer {
    mount_me(&mock).await;
    Mock::given(method("GET"))
        .and(path("/UserViews"))
        .respond_with(json(fixture("user_views")))
        .mount(&mock)
        .await;
    for route in ["/UserItems/Resume", "/Shows/NextUp"] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(EMPTY))
            .mount(&mock)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/Items/Latest"))
        .respond_with(json("[]"))
        .mount(&mock)
        .await;
    mock
}

#[test]
fn empty_home_says_so() {
    let flow = start_with(mount_empty, save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_until("the empty home message", || {
            ElementHandle::find_by_accessible_label(&window, "Nothing to show yet")
                .next()
                .is_some()
        })
        .await?;
        if window.get_home_rows().row_count() != 0 || window.get_home_banner().has {
            return Err("an empty home still has content".into());
        }
        Ok(())
    });
}
