//! Taking an item without a position out of Continue Watching offers nothing to undo.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::menu::{choose, right_click, wait_menu};
use common::{exists, json, mount_me, run, save_session, start_with, wait_role, wait_until};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{AppWindow, Screen};
use slint::{ComponentHandle, Model};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";
const SEASON_2: &str = "1507e3d02b7d70abe4a2d37241b02feb";
const EPISODE: &str = "a2000000000000000000000000000005";
const POSITION: i64 = 0;
const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

fn episode(position: i64) -> serde_json::Value {
    serde_json::json!({
        "Name": "Sword", "Id": EPISODE, "Type": "Episode", "IsFolder": false,
        "RunTimeTicks": 14_400_000_000_i64, "ParentIndexNumber": 2, "IndexNumber": 5,
        "SeriesName": "Northern Saga", "SeriesId": SERIES, "SeasonId": SEASON_2,
        "UserData": {"PlaybackPositionTicks": position, "PlayCount": 1, "IsFavorite": false,
            "Played": false}
    })
}

/// The resume list: the episode while it has a position.
struct Resume {
    hidden: Arc<AtomicBool>,
}

impl Respond for Resume {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        if self.hidden.load(Ordering::SeqCst) {
            return json(EMPTY);
        }
        json(
            serde_json::json!({"Items": [episode(POSITION)], "TotalRecordCount": 1, "StartIndex": 0})
                .to_string(),
        )
    }
}

/// Stores the position posted, as the server does.
struct UserData {
    hidden: Arc<AtomicBool>,
}

impl Respond for UserData {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let position = body["PlaybackPositionTicks"].as_i64().unwrap_or(0);
        self.hidden.store(position == 0, Ordering::SeqCst);
        json(episode(position)["UserData"].to_string())
    }
}

fn resume_cards(window: &AppWindow) -> usize {
    let rows = window.get_home_rows();
    (0..rows.row_count())
        .filter_map(|index| rows.row_data(index))
        .filter(|row| row.resume)
        .map(|row| row.items.row_count())
        .sum()
}

#[test]
fn an_item_without_a_position_offers_no_undo() {
    let hidden = Arc::new(AtomicBool::new(false));
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_me(&mock).await;
            let get = |route: &str, body: String| {
                Mock::given(method("GET"))
                    .and(path(route.to_owned()))
                    .respond_with(json(body))
            };
            get("/UserViews", common::fixture("user_views"))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path("/UserItems/Resume"))
                .respond_with(Resume {
                    hidden: Arc::clone(&hidden),
                })
                .mount(&mock)
                .await;
            get("/Shows/NextUp", EMPTY.into()).mount(&mock).await;
            get("/Items/Latest", "[]".into()).mount(&mock).await;
            get(&format!("/Items/{EPISODE}"), episode(POSITION).to_string())
                .mount(&mock)
                .await;
            Mock::given(method("POST"))
                .and(path(format!("/UserItems/{EPISODE}/UserData")))
                .respond_with(UserData { hidden })
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
        let card = wait_role(&window, "Sword", AccessibleRole::Button).await?;
        right_click(&window, &card);
        wait_menu(&window, "Northern Saga").await?;
        choose(&window, "Remove from Continue Watching")?;
        wait_until("the card to leave", || resume_cards(&window) == 0).await?;

        let notice = wait_role(&window, "Notice", AccessibleRole::Groupbox).await?;
        let text = notice
            .query_descendants()
            .match_accessible_role(AccessibleRole::Text)
            .find_first()
            .and_then(|text| text.accessible_label())
            .unwrap_or_default();
        if text != "Removed from Continue Watching" {
            return Err(format!("notice {text:?}"));
        }
        if exists(&window, "Undo") {
            return Err("an undo would put no position back".into());
        }
        Ok(())
    });
}
