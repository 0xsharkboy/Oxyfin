//! A home screen asked for again after the player keeps its rows when the server fails, and
//! tells it in the notice.

mod common;

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::{fixture, json, mount_browse_basics, run, save_session, start_with, wait_until};
use oxyfin::state::Msg;
use oxyfin::{AppWindow, HomeRow, Message, Screen};
use slint::{ComponentHandle, Model};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

struct Resume(Arc<AtomicBool>);

impl Respond for Resume {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        if self.0.load(Ordering::SeqCst) {
            return ResponseTemplate::new(500);
        }
        json(fixture("user_items_resume"))
    }
}

async fn mount(mock: MockServer, failing: Arc<AtomicBool>) -> MockServer {
    Mock::given(method("GET"))
        .and(path("/UserItems/Resume"))
        .respond_with(Resume(failing))
        .mount(&mock)
        .await;
    mount_browse_basics(&mock, r#"{"Items":[],"TotalRecordCount":0}"#).await;
    mock
}

fn resume_row(window: &AppWindow) -> Option<HomeRow> {
    window.get_home_rows().iter().find(|row| row.resume)
}

#[test]
fn a_failed_refresh_keeps_the_rows() {
    let failing = Arc::new(AtomicBool::new(false));
    let server = Arc::clone(&failing);
    let flow = start_with(move |mock| mount(mock, server), save_session);
    let window = flow.window.as_weak();
    let app = Rc::clone(&flow.app);

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home rows", || {
            window.get_home_settled()
                && resume_row(&window).is_some_and(|row| row.items.row_count() > 0)
        })
        .await?;
        failing.store(true, Ordering::SeqCst);
        app.dispatch(Msg::OpenPlayer);
        app.dispatch(Msg::ClosePlayer);
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_until("the notice", || window.get_notice().visible).await?;
        if window.get_notice().message != Message::ServerError {
            return Err(format!("{:?}", window.get_notice()));
        }
        let row = resume_row(&window).ok_or("Continue Watching is gone")?;
        if row.items.row_count() == 0 || !row.error.is_empty() {
            return Err(format!("the row gave way to an error: {}", row.error));
        }
        Ok(())
    });
}
