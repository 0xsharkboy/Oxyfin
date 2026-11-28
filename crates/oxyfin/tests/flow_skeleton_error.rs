//! A slow page that fails trades its bones for the error and its Retry button.

mod common;

use std::time::Duration;

use common::{
    bones, click, fixture, json, mount_browse_basics, run, save_session, skeleton_shown,
    start_with, wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";

#[test]
fn a_failed_page_replaces_its_bones_with_the_error() {
    let flow = start_with(
        |mock: MockServer| async move {
            mount_browse_basics(&mock, r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(json(fixture("items_movies")))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Items/{MOVIE}")))
                .respond_with(ResponseTemplate::new(500).set_delay(Duration::from_millis(1500)))
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
        let movies = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &movies);
        let card = wait_visible(&window, "Test Movie (2020)").await?;
        click(&window, &card);

        wait_until("the movie", || window.get_screen() == Screen::Movie).await?;
        wait_until("the header bones", || skeleton_shown(&window)).await?;
        let retry = wait_role(&window, "Retry", AccessibleRole::Button).await?;
        if !bones(&window).is_empty() {
            return Err("bones stay beside the error".into());
        }

        click(&window, &retry);
        wait_until("the header bones again", || skeleton_shown(&window)).await?;
        wait_role(&window, "Retry", AccessibleRole::Button).await?;
        if !bones(&window).is_empty() {
            return Err("bones stay beside the second error".into());
        }
        Ok(())
    });
}
