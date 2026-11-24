//! A row chosen from the keyboard in the menu of a card keeps the focus on that card when its
//! row is rebuilt, and gives it back to the window once the card has left its row.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::menu::wait_menu;
use common::{fixture, json, mount_me, pause, run, save_session, start_with, wait_until};
use oxyfin::{AppWindow, ContextMenu, Screen, Shortcuts};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";
const SEASON_2: &str = "1507e3d02b7d70abe4a2d37241b02feb";
const EPISODE: &str = "a2000000000000000000000000000005";
const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

fn episode() -> serde_json::Value {
    serde_json::json!({
        "Name": "Sword", "Id": EPISODE, "Type": "Episode", "IsFolder": false,
        "RunTimeTicks": 14_400_000_000_i64, "ParentIndexNumber": 2, "IndexNumber": 5,
        "SeriesName": "Northern Saga", "SeriesId": SERIES, "SeasonId": SEASON_2,
        "UserData": {"PlayedPercentage": 50, "PlaybackPositionTicks": 7_200_000_000_i64,
            "PlayCount": 1, "IsFavorite": false, "Played": false}
    })
}

/// The resume list: the episode until it is taken out.
struct Resume {
    hidden: Arc<AtomicBool>,
}

impl Respond for Resume {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        if self.hidden.load(Ordering::SeqCst) {
            return json(EMPTY);
        }
        json(
            serde_json::json!({"Items": [episode()], "TotalRecordCount": 1, "StartIndex": 0})
                .to_string(),
        )
    }
}

struct Hide {
    hidden: Arc<AtomicBool>,
}

impl Respond for Hide {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.hidden.store(true, Ordering::SeqCst);
        json(r#"{"PlaybackPositionTicks":0,"PlayCount":1,"IsFavorite":false,"Played":false}"#)
    }
}

async fn mount(mock: MockServer, hidden: Arc<AtomicBool>) -> MockServer {
    mount_me(&mock).await;
    let get = |route: String, body: String| {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(json(body))
    };
    get("/UserViews".into(), fixture("user_views"))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/UserItems/Resume"))
        .respond_with(Resume {
            hidden: Arc::clone(&hidden),
        })
        .mount(&mock)
        .await;
    get("/Shows/NextUp".into(), EMPTY.into()).mount(&mock).await;
    get("/Items/Latest".into(), "[]".into()).mount(&mock).await;
    get(format!("/Items/{EPISODE}"), episode().to_string())
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/UserFavoriteItems/{EPISODE}")))
        .respond_with(json(
            r#"{"PlaybackPositionTicks":7200000000,"PlayCount":1,"IsFavorite":true,"Played":false}"#,
        ))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/UserItems/{EPISODE}/UserData")))
        .respond_with(Hide { hidden })
        .mount(&mock)
        .await;
    mock
}

fn press(window: &AppWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn shift_f10(window: &AppWindow) {
    let shift: slint::SharedString = Key::Shift.into();
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: shift.clone(),
    });
    press(window, Key::F10);
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: shift });
}

/// Whether the focused element opened a keyboard menu on the episode.
fn episode_menu_open(window: &AppWindow) -> bool {
    let menu = window.global::<ContextMenu>();
    menu.get_open() && menu.get_by_keyboard() && menu.get_id() == EPISODE
}

/// Moves the focus with Tab until it rests on the episode card.
async fn focus_the_card(window: &AppWindow) -> Result<(), String> {
    for _ in 0..30 {
        press(window, Key::Tab);
        shift_f10(window);
        if episode_menu_open(window) {
            press(window, Key::Escape);
            return Ok(());
        }
        if window.global::<ContextMenu>().get_open() {
            press(window, Key::Escape);
        }
    }
    Err("Tab never reached the episode card".into())
}

/// Chooses the row `label` of the open menu with the arrows and Return.
async fn choose_with_keys(window: &AppWindow, label: &str) -> Result<(), String> {
    let rows = wait_menu(window, "Northern Saga").await?;
    let index = rows
        .iter()
        .position(|row| row == label)
        .ok_or_else(|| format!("no row {label} in {rows:?}"))?;
    for _ in 0..index {
        press(window, Key::DownArrow);
    }
    press(window, Key::Return);
    Ok(())
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
fn a_keyboard_menu_row_keeps_the_focus_through_a_rebuilt_row() {
    let hidden = Arc::new(AtomicBool::new(false));
    let flow = start_with(move |mock| mount(mock, hidden), save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_until("the Continue Watching card", || resume_cards(&window) == 1).await?;
        focus_the_card(&window).await?;

        shift_f10(&window);
        choose_with_keys(&window, "Add to favorites").await?;
        let menu = window.global::<ContextMenu>();
        if menu.get_changes_pending() != 1 {
            return Err("the window does not wait for the answer of the server".into());
        }
        wait_until("the answer of the server", || {
            menu.get_changes_pending() == 0
        })
        .await?;
        // The change reloads the home rows, which builds the cards again.
        pause(Duration::from_millis(600)).await;
        shift_f10(&window);
        if !episode_menu_open(&window) {
            return Err("the rebuilt card lost the focus".into());
        }

        choose_with_keys(&window, "Remove from Continue Watching").await?;
        wait_until("the card to leave", || resume_cards(&window) == 0).await?;
        pause(Duration::from_millis(600)).await;
        press(&window, "?");
        if !window.global::<Shortcuts>().get_open() {
            return Err("the keys are dead once the focused card left".into());
        }
        Ok(())
    });
}
