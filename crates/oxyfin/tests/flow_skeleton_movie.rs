//! A slow library and a slow movie show bones after a short wait, never a "Loading" text, and
//! the bones leave when the content arrives.

mod common;

use std::time::{Duration, Instant};

use common::{
    bones, click, exists, fixture, json, mount_browse_basics, run, save_session, skeleton_shown,
    start_with, wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
const SLOW: Duration = Duration::from_millis(700);
const DELAY: Duration = Duration::from_millis(150);

#[test]
fn slow_pages_show_bones_until_they_load() {
    let flow = start_with(
        |mock: MockServer| async move {
            mount_browse_basics(&mock, r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(json(fixture("items_movies")).set_delay(SLOW))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Items/{MOVIE}")))
                .respond_with(json(fixture("item_movie")).set_delay(SLOW))
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
        wait_until("the home screen to settle", || window.get_home_settled()).await?;
        if !bones(&window).is_empty() {
            return Err("bones are left on a loaded home screen".into());
        }

        let movies = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &movies);
        let asked = Instant::now();
        if skeleton_shown(&window) {
            return Err("the grid bones show at once".into());
        }
        wait_until("the grid bones", || skeleton_shown(&window)).await?;
        if asked.elapsed() < DELAY - Duration::from_millis(10) {
            return Err(format!("the grid bones showed after {:?}", asked.elapsed()));
        }
        if exists(&window, "Loading") {
            return Err("the library still says Loading".into());
        }
        let card = wait_visible(&window, "Test Movie (2020)").await?;
        wait_until("the grid bones to leave", || bones(&window).is_empty()).await?;

        click(&window, &card);
        let asked = Instant::now();
        wait_until("the movie", || window.get_screen() == Screen::Movie).await?;
        wait_until("the header bones", || skeleton_shown(&window)).await?;
        if asked.elapsed() < DELAY - Duration::from_millis(10) {
            return Err(format!(
                "the header bones showed after {:?}",
                asked.elapsed()
            ));
        }
        if exists(&window, "Loading") {
            return Err("the movie still says Loading".into());
        }
        wait_role(&window, "Back", AccessibleRole::Button).await?;
        wait_visible(&window, "Drama").await?;
        wait_until("the header bones to leave", || bones(&window).is_empty()).await?;

        let back = wait_role(&window, "Back", AccessibleRole::Button).await?;
        click(&window, &back);
        wait_until("the library", || window.get_screen() == Screen::Library).await?;
        wait_visible(&window, "Test Movie (2020)").await?;
        if !bones(&window).is_empty() {
            return Err("a page restored from the back cache shows bones".into());
        }
        Ok(())
    });
}
