//! The context menu of a series in a library: its rows, a favorite change, then Details.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use common::menu::{choose, right_click, wait_menu};
use common::{
    click, fixture, json, mount_browse_basics, run, save_session, start_with, wait_role, wait_until,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SERIES: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";
const TITLE: &str = "Test Show";

/// The series as the server holds it, a favorite once added.
struct Series {
    favorite: Arc<AtomicBool>,
}

impl Respond for Series {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let mut body: serde_json::Value =
            serde_json::from_str(&fixture("item_series")).unwrap_or_default();
        body["UserData"]["IsFavorite"] = self.favorite.load(Ordering::SeqCst).into();
        json(body.to_string())
    }
}

struct AddFavorite {
    favorite: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
}

impl Respond for AddFavorite {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.favorite.store(true, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
        json(r#"{"PlaybackPositionTicks":0,"PlayCount":0,"IsFavorite":true,"Played":false}"#)
    }
}

#[test]
fn a_series_menu_adds_it_to_the_favorites() {
    let favorite = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let posted = Arc::clone(&calls);
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, &fixture("next_up_series")).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(json(fixture("items_shows")))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Items/{SERIES}")))
                .respond_with(Series {
                    favorite: Arc::clone(&favorite),
                })
                .mount(&mock)
                .await;
            Mock::given(method("POST"))
                .and(path(format!("/UserFavoriteItems/{SERIES}")))
                .respond_with(AddFavorite {
                    favorite,
                    calls: posted,
                })
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
        let tab = wait_role(&window, "Shows", AccessibleRole::Tab).await?;
        click(&window, &tab);
        let card = wait_role(&window, TITLE, AccessibleRole::Button).await?;

        right_click(&window, &card);
        let rows = wait_menu(&window, TITLE).await?;
        let expected = [
            "Resume S02E05",
            "Mark as watched",
            "Add to favorites",
            "Details",
        ];
        if rows != expected {
            return Err(format!("rows {rows:?}, expected {expected:?}"));
        }
        choose(&window, "Add to favorites")?;
        wait_until("the favorite request", || calls.load(Ordering::SeqCst) == 1).await?;

        let card = wait_role(&window, TITLE, AccessibleRole::Button).await?;
        right_click(&window, &card);
        let rows = wait_menu(&window, TITLE).await?;
        if !rows.iter().any(|row| row == "Remove from favorites") {
            return Err(format!("the menu does not follow the change: {rows:?}"));
        }
        choose(&window, "Details")?;
        wait_until("the series page", || window.get_screen() == Screen::Series).await?;
        Ok(())
    });
}
