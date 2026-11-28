//! The answer to a stop report reloads the screen shown once; the screens below reload when
//! they show again, and the similar titles keep their order.
#![cfg(feature = "mpv")]

mod common;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::answers::{Answers, id, titles, visit_movies};
use common::{click, json, run, save_session, start_with, wait_role, wait_until, wait_visible};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{AppWindow, Screen};
use oxyfin_core::PlaybackEvent;
use slint::{ComponentHandle, Model};
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, Request, Respond, ResponseTemplate};

/// Lists movies 2 and 3, the other way round and with 3 played from the second answer on.
struct Similar(Arc<AtomicUsize>);

impl Respond for Similar {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let again = self.0.fetch_add(1, Ordering::SeqCst) > 0;
        let movie = |number: usize| {
            serde_json::json!({
                "Name": format!("Movie {number}"), "Id": id(number), "Type": "Movie",
                "IsFolder": false, "ProductionYear": 2020,
                "UserData": {"PlaybackPositionTicks": 0, "PlayCount": 0, "IsFavorite": false,
                    "Played": again && number == 3}
            })
        };
        let items = if again {
            [movie(3), movie(2)]
        } else {
            [movie(2), movie(3)]
        };
        json(serde_json::json!({"Items": items, "TotalRecordCount": 2}).to_string())
    }
}

fn similar(window: &AppWindow) -> Vec<(String, bool)> {
    window
        .get_movie_similar()
        .iter()
        .map(|card| (card.title.to_string(), card.watched))
        .collect()
}

#[test]
fn a_stop_report_reloads_only_the_screen_shown() {
    let answers = Answers::holding(&[1, 2, 3]);
    let server = answers.clone();
    let similar_asked = Arc::new(AtomicUsize::new(0));
    let responder = Similar(Arc::clone(&similar_asked));
    let flow = start_with(
        move |mock| async move {
            Mock::given(method("GET"))
                .and(path_regex("^/Items/[0-9a-f]{32}/Similar$"))
                .respond_with(responder)
                .mount(&mock)
                .await;
            server.mount(mock).await
        },
        save_session,
    );
    let window = flow.window.as_weak();
    let app = Rc::clone(&flow.app);
    let counted = answers.clone();
    let before = Rc::new(Cell::new((0, 0)));
    let baseline = Rc::clone(&before);
    let movie = format!("/Items/{}?", id(1));
    let grid = ["/Items?", "includeItemTypes=Movie"];
    let (movie_path, grid_parts) = (movie.clone(), grid);
    let asked = Arc::clone(&similar_asked);

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        visit_movies(&window).await?;
        let card = wait_visible(&window, "Movie 1").await?;
        click(&window, &card);
        wait_until("the movie", || window.get_screen() == Screen::Movie).await?;
        wait_role(&window, "Mark as watched", AccessibleRole::Button).await?;
        wait_until("the similar titles", || similar(&window).len() == 2).await?;
        let (pages, grids) = (answers.asked(&[&movie]), answers.asked(&grid));
        baseline.set((pages, grids));

        app.show_playback(PlaybackEvent::StopReported);
        wait_until("the movie asked again", || {
            answers.asked(&[&movie]) == pages + 1
        })
        .await?;
        if answers.asked(&grid) != grids {
            return Err("the grid below was asked for while hidden".into());
        }
        wait_until("the similar titles asked again", || {
            asked.load(Ordering::SeqCst) == 2 && similar(&window).iter().any(|(_, seen)| *seen)
        })
        .await?;
        let shown = similar(&window);
        let expected = [("Movie 2".to_owned(), false), ("Movie 3".to_owned(), true)];
        if shown != expected {
            return Err(format!("the similar titles moved: {shown:?}"));
        }

        let back = wait_role(&window, "Back", AccessibleRole::Button).await?;
        click(&window, &back);
        wait_until("the library", || window.get_screen() == Screen::Library).await?;
        wait_until("the grid asked again", || answers.asked(&grid) == grids + 1).await?;
        if titles(&window) != ["Movie 1", "Movie 2", "Movie 3"] {
            return Err(format!("{:?}", titles(&window)));
        }
        Ok(())
    });
    let (pages, grids) = before.get();
    assert_eq!(
        counted.asked(&[&movie_path]),
        pages + 1,
        "the movie was asked again"
    );
    assert_eq!(
        counted.asked(&grid_parts),
        grids + 1,
        "the grid was asked again"
    );
    assert_eq!(
        similar_asked.load(Ordering::SeqCst),
        2,
        "the similar titles"
    );
}
