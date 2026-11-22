//! A slow home screen shows the bones of its banner until Continue Watching answers.

mod common;

use std::time::Duration;

use common::{
    bones, exists, fixture, json, mount_me, run, save_session, skeleton_shown, start_with,
    wait_until,
};
use oxyfin::Screen;
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

#[test]
fn the_home_screen_shows_bones_until_its_banner_is_known() {
    let flow = start_with(
        |mock: MockServer| async move {
            mount_me(&mock).await;
            Mock::given(method("GET"))
                .and(path("/UserViews"))
                .respond_with(json(fixture("user_views")))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path("/UserItems/Resume"))
                .respond_with(
                    json(fixture("user_items_resume")).set_delay(Duration::from_millis(900)),
                )
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path("/Shows/NextUp"))
                .respond_with(json(fixture("shows_next_up")))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path("/Items/Latest"))
                .respond_with(json(fixture("items_latest")))
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
        wait_until("the banner bones", || skeleton_shown(&window)).await?;
        if exists(&window, "Loading") {
            return Err("the home screen says Loading".into());
        }
        wait_until("the banner", || window.get_home_banner().has).await?;
        wait_until("the bones to leave", || bones(&window).is_empty()).await?;
        Ok(())
    });
}
