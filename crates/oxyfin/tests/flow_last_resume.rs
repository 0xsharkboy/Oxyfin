//! An item taken out of Continue Watching stays out: of the home screen showing its last answer
//! with another on its way, and of its later visits.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::answers::{EMPTY, SLOW, open_tab};
use common::menu::{choose, right_click, wait_menu};
use common::{
    fixture, json, mount_me, pause, run, save_session, start_with, wait_role, wait_until,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{AppWindow, HomeTitles, Screen};
use slint::{ComponentHandle, Model};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const EPISODE: &str = "a2000000000000000000000000000005";

fn episode() -> serde_json::Value {
    serde_json::json!({
        "Name": "Sword", "Id": EPISODE, "Type": "Episode", "IsFolder": false,
        "RunTimeTicks": 14_400_000_000_i64, "ParentIndexNumber": 2, "IndexNumber": 5,
        "SeriesName": "Northern Saga", "SeriesId": "9ea196e7ccc3ae17d2c3f75c3ab34406",
        "UserData": {"PlayedPercentage": 50, "PlaybackPositionTicks": 7_200_000_000_i64,
            "PlayCount": 1, "IsFavorite": false, "Played": false}
    })
}

/// The resume list as it was when asked for.
struct Resume {
    hidden: Arc<AtomicBool>,
    slow: Arc<AtomicBool>,
}

impl Respond for Resume {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let answer = if self.hidden.load(Ordering::SeqCst) {
            json(EMPTY)
        } else {
            let list =
                serde_json::json!({"Items": [episode()], "TotalRecordCount": 1, "StartIndex": 0});
            json(list.to_string())
        };
        match self.slow.load(Ordering::SeqCst) {
            true => answer.set_delay(SLOW),
            false => answer,
        }
    }
}

struct Hide(Arc<AtomicBool>);

impl Respond for Hide {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.0.store(true, Ordering::SeqCst);
        json(r#"{"PlaybackPositionTicks":0,"PlayCount":1,"IsFavorite":false,"Played":false}"#)
    }
}

async fn mount(mock: MockServer, hidden: Arc<AtomicBool>, slow: Arc<AtomicBool>) -> MockServer {
    mount_me(&mock).await;
    Mock::given(method("GET"))
        .and(path("/UserItems/Resume"))
        .respond_with(Resume {
            hidden: Arc::clone(&hidden),
            slow,
        })
        .mount(&mock)
        .await;
    for (route, body) in [
        ("/UserViews".to_owned(), fixture("user_views")),
        ("/Shows/NextUp".to_owned(), EMPTY.to_owned()),
        ("/Items/Latest".to_owned(), "[]".to_owned()),
        ("/Items".to_owned(), EMPTY.to_owned()),
        ("/Genres".to_owned(), EMPTY.to_owned()),
        (format!("/Items/{EPISODE}"), episode().to_string()),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body))
            .mount(&mock)
            .await;
    }
    Mock::given(method("POST"))
        .and(path(format!("/UserItems/{EPISODE}/UserData")))
        .respond_with(Hide(hidden))
        .mount(&mock)
        .await;
    mock
}

fn resumes(window: &AppWindow) -> bool {
    let title = window.global::<HomeTitles>().invoke_continue_watching();
    window.get_screen() == Screen::Home
        && (window.get_home_rows().iter().any(|row| row.title == title)
            || window.get_home_banner().has)
}

#[test]
fn an_item_taken_out_of_continue_watching_never_comes_back() {
    let hidden = Arc::new(AtomicBool::new(false));
    let slow = Arc::new(AtomicBool::new(false));
    let waits = Arc::clone(&slow);
    let flow = start_with(move |mock| mount(mock, hidden, waits), save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_home_settled()).await?;
        open_tab(&window, "Movies").await?;
        wait_until("the movies", || window.get_screen() == Screen::Library).await?;
        slow.store(true, Ordering::SeqCst);
        open_tab(&window, "Home").await?;
        if !resumes(&window) {
            return Err("the home screen waits for the server".into());
        }
        let card = wait_role(&window, "Sword", AccessibleRole::Button).await?;
        right_click(&window, &card);
        wait_menu(&window, "Northern Saga").await?;
        choose(&window, "Remove from Continue Watching")?;
        wait_until("the item to go", || !resumes(&window)).await?;
        for visit in ["Home", "Movies", "Home"] {
            open_tab(&window, visit).await?;
            for _ in 0..12 {
                if resumes(&window) {
                    return Err(format!("the item came back on {visit}"));
                }
                pause(SLOW / 10).await;
            }
        }
        pause(Duration::from_millis(100)).await;
        Ok(())
    });
}
