//! An image the server rejects with 401 ends the session like any other rejected request.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{
    fixture, json, mount_me, mount_public_info, run, save_session, start_with, wait_until,
};
use oxyfin::{Message, Screen};
use slint::ComponentHandle;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

struct Counted(Arc<AtomicUsize>);

impl Respond for Counted {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.0.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(401)
    }
}

async fn mount_home(mock: MockServer, images: Arc<AtomicUsize>) -> MockServer {
    mount_public_info(&mock).await;
    mount_me(&mock).await;
    Mock::given(method("GET"))
        .and(path("/UserViews"))
        .respond_with(json(r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#))
        .mount(&mock)
        .await;
    let resume = fixture("user_items_resume");
    let with_thumb = resume.replacen(
        "\"ImageTags\": {",
        "\"ImageTags\": {\"Thumb\": \"0f1e2d3c\",",
        1,
    );
    assert_ne!(with_thumb, resume, "the fixture has no ImageTags to patch");
    Mock::given(method("GET"))
        .and(path("/UserItems/Resume"))
        .respond_with(json(with_thumb))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Shows/NextUp"))
        .respond_with(json(r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/Items/[^/]+/Images/"))
        .respond_with(Counted(images))
        .mount(&mock)
        .await;
    mock
}

#[test]
fn rejected_image_expires_the_session() {
    let images = Arc::new(AtomicUsize::new(0));
    let requests = Arc::clone(&images);
    let flow = start_with(move |mock| mount_home(mock, requests), save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the image request", || images.load(Ordering::SeqCst) > 0).await?;
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;
        let notice = window.get_notice();
        if !notice.visible || notice.message != Message::SessionExpired {
            return Err(format!("unexpected notice: {notice:?}"));
        }
        Ok(())
    });
}
