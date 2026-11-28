//! The banner offers "More info" only when its item has a detail page to open.

mod common;

use common::{
    exists, fixture, json, mount_me, run, save_session, start_with, wait_role, wait_until,
    wait_visible,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

// An episode that does not say which series it belongs to has no page to open.
const ORPHAN: &str = r#"{"Items":[{"Name":"Loose Episode","Id":"0407e3d02b7d70abe4a2d37241b02fea",
    "Type":"Episode","IsFolder":false,"RunTimeTicks":100230000,"ParentIndexNumber":1,"IndexNumber":2,
    "UserData":{"PlaybackPositionTicks":30000000,"PlayCount":1,"IsFavorite":false,"Played":false}}],
    "TotalRecordCount":1,"StartIndex":0}"#;

async fn mount_home(mock: MockServer) -> MockServer {
    mount_me(&mock).await;
    Mock::given(method("GET"))
        .and(path("/UserViews"))
        .respond_with(json(fixture("user_views")))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/UserItems/Resume"))
        .respond_with(json(ORPHAN))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Shows/NextUp"))
        .respond_with(json(EMPTY))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Items/Latest"))
        .respond_with(json("[]"))
        .mount(&mock)
        .await;
    mock
}

#[test]
fn the_banner_hides_more_info_without_a_detail_page() {
    let flow = start_with(mount_home, save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_visible(&window, "Loose Episode").await?;
        wait_role(&window, "Resume S01E02", AccessibleRole::Button).await?;
        if exists(&window, "More info") {
            return Err("More info is offered for an item without a detail page".into());
        }
        Ok(())
    });
}
