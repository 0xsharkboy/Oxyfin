//! A favorite removed from a movie page is gone from the Favorites grid when going back.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use common::{
    QueryLog, click, fixture, has_param, json, mount_browse_basics, run, save_session, start_with,
    wait_role, wait_until, wait_visible,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
const UNFAVORED: &str =
    r#"{"PlaybackPositionTicks":0,"PlayCount":0,"IsFavorite":false,"Played":false}"#;
const EMPTY: &str = r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;

struct Favorites {
    removed: Arc<AtomicBool>,
    log: QueryLog,
}

impl Respond for Favorites {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query = request.url.query().unwrap_or("").to_owned();
        let favorites = has_param(&query, "isFavorite", "true");
        if favorites {
            self.log.lock().map(|mut log| log.push(query)).ok();
        }
        if favorites && self.removed.load(Ordering::SeqCst) {
            json(EMPTY)
        } else {
            json(fixture("items_movies"))
        }
    }
}

fn requests(log: &QueryLog) -> usize {
    log.lock().map(|log| log.len()).unwrap_or(0)
}

#[test]
fn going_back_after_removing_a_favorite_reloads_the_grid() {
    let removed = Arc::new(AtomicBool::new(false));
    let log: QueryLog = Arc::new(Mutex::new(Vec::new()));
    let (flag, served) = (Arc::clone(&removed), Arc::clone(&log));
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, EMPTY).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(Favorites {
                    removed: Arc::clone(&flag),
                    log: served,
                })
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Items/{MOVIE}")))
                .respond_with(json(fixture("item_movie")))
                .mount(&mock)
                .await;
            Mock::given(method("DELETE"))
                .and(path(format!("/UserFavoriteItems/{MOVIE}")))
                .respond_with(move |_: &Request| {
                    flag.store(true, Ordering::SeqCst);
                    json(UNFAVORED)
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
        let favorites = wait_role(&window, "Favorites", AccessibleRole::Tab).await?;
        click(&window, &favorites);
        let card = wait_visible(&window, "Test Movie (2020)").await?;
        click(&window, &card);
        wait_until("the movie", || window.get_screen() == Screen::Movie).await?;

        let remove = wait_role(&window, "Remove from favorites", AccessibleRole::Button).await?;
        click(&window, &remove);
        wait_role(&window, "Add to favorites", AccessibleRole::Button).await?;
        if requests(&log) != 1 {
            return Err(format!(
                "{} favorites requests before going back",
                requests(&log)
            ));
        }

        let back = wait_role(&window, "Back", AccessibleRole::Button).await?;
        click(&window, &back);
        wait_until("the library", || window.get_screen() == Screen::Library).await?;
        wait_visible(&window, "Nothing here yet").await?;
        if requests(&log) != 2 {
            return Err(format!(
                "{} favorites requests after going back",
                requests(&log)
            ));
        }
        Ok(())
    });
}
