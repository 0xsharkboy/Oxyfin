//! Without libmpv, playing a Continue Watching card explains that playback is unavailable.
#![cfg(not(feature = "mpv"))]

mod common;

use common::{fixture, json, mount_me, run, save_session, start_with, wait_until};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{Message, Screen};
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

const RESUMED: &str = "Test Movie (2020)";

async fn mount_home(mock: MockServer) -> MockServer {
    mount_me(&mock).await;
    Mock::given(method("GET"))
        .and(path("/UserViews"))
        .respond_with(json(r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/UserItems/Resume"))
        .respond_with(json(fixture("user_items_resume")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Shows/NextUp"))
        .respond_with(json(r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#))
        .mount(&mock)
        .await;
    mock
}

#[test]
fn playing_a_card_without_mpv_shows_a_notice() {
    let flow = start_with(mount_home, save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        let card = || {
            ElementHandle::find_by_accessible_label(&window, RESUMED)
                .find(|element| element.accessible_role() == Some(AccessibleRole::Button))
        };
        wait_until("the Continue Watching card", || card().is_some()).await?;
        card()
            .ok_or("no Continue Watching card")?
            .invoke_accessible_default_action();

        wait_until("the notice", || window.get_notice().visible).await?;
        let notice = window.get_notice();
        if notice.message != Message::PlaybackUnavailable || notice.dismissible {
            return Err(format!("unexpected notice: {notice:?}"));
        }
        if window.get_screen() != Screen::Home {
            return Err(format!("left the home screen: {:?}", window.get_screen()));
        }
        Ok(())
    });
}
