//! A slow series shows the bones of its header, then those of its episodes until they arrive.

mod common;

use std::time::Duration;

use common::{
    bones, click, exists, fixture, json, mount_browse_basics, run, save_session, skeleton_shown,
    start_with, wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::Screen;
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";

#[test]
fn a_slow_series_shows_bones_for_its_header_then_its_episodes() {
    let flow = start_with(
        |mock: MockServer| async move {
            mount_browse_basics(&mock, &fixture("next_up_series")).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(json(fixture("items_shows")))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Items/{SERIES}")))
                .respond_with(json(fixture("item_series")).set_delay(Duration::from_millis(600)))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Shows/{SERIES}/Seasons")))
                .respond_with(json(fixture("seasons")))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Shows/{SERIES}/Episodes")))
                .respond_with(
                    json(fixture("episodes_season_2")).set_delay(Duration::from_millis(900)),
                )
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
        wait_until("the header bones", || skeleton_shown(&window)).await?;
        if ElementHandle::find_by_element_type_name(&window, "DetailHeaderBones")
            .next()
            .is_none()
        {
            return Err("the bones are not those of a header".into());
        }

        wait_until("the series details", || window.get_series_loaded()).await?;
        wait_until("the episode bones", || {
            skeleton_shown(&window)
                && ElementHandle::find_by_element_type_name(&window, "DetailHeaderBones")
                    .next()
                    .is_none()
                && ElementHandle::find_by_element_type_name(&window, "EpisodeRowBones")
                    .next()
                    .is_some()
        })
        .await?;
        if exists(&window, "Loading") {
            return Err("the series still says Loading".into());
        }
        wait_until("the episodes", || {
            !window.get_series_episodes_loading() && bones(&window).is_empty()
        })
        .await?;
        Ok(())
    });
}
