//! Opens a movie from the library: details, resume and play buttons, watched and favorite toggles.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{
    click, exists, fixture, json, mount_browse_basics, run, save_session, start_with, wait_role,
    wait_until, wait_visible,
};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{Message, Screen};
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const MOVIE: &str = "76282b1c408f374beeee50519e9157a4";
const SECOND: &str = "b4c9a0c1d2e34f5a8b6c7d8e9f0a1b2c";
const PLAYED: &str = r#"{"PlaybackPositionTicks":0,"PlayCount":2,"IsFavorite":true,"Played":true}"#;

struct Counted {
    calls: Arc<AtomicUsize>,
    response: ResponseTemplate,
}

impl Respond for Counted {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.response.clone()
    }
}

fn replaced(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "{from} is not in the fixture");
    text.replace(from, to)
}

#[test]
fn a_movie_opens_with_its_details_and_actions() {
    let played = Arc::new(AtomicUsize::new(0));
    let unfavored = Arc::new(AtomicUsize::new(0));
    let (played_calls, unfavored_calls) = (Arc::clone(&played), Arc::clone(&unfavored));
    let flow = start_with(
        move |mock: MockServer| async move {
            mount_browse_basics(&mock, r#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#).await;
            Mock::given(method("GET"))
                .and(path("/Items"))
                .respond_with(json(fixture("items_movies")))
                .mount(&mock)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/Items/{MOVIE}")))
                .respond_with(json(fixture("item_movie")))
                .mount(&mock)
                .await;
            let unstarted = replaced(
                &replaced(&fixture("item_movie"), MOVIE, SECOND),
                "\"PlaybackPositionTicks\": 30000000",
                "\"PlaybackPositionTicks\": 0",
            );
            Mock::given(method("GET"))
                .and(path(format!("/Items/{SECOND}")))
                .respond_with(json(replaced(
                    &unstarted,
                    "Test Movie (2020)",
                    "Second Feature (2021)",
                )))
                .mount(&mock)
                .await;
            Mock::given(method("POST"))
                .and(path(format!("/UserPlayedItems/{MOVIE}")))
                .respond_with(Counted {
                    calls: played_calls,
                    response: json(PLAYED),
                })
                .mount(&mock)
                .await;
            Mock::given(method("DELETE"))
                .and(path(format!("/UserFavoriteItems/{MOVIE}")))
                .respond_with(Counted {
                    calls: unfavored_calls,
                    response: ResponseTemplate::new(500),
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
        let movies = wait_role(&window, "Movies", AccessibleRole::Tab).await?;
        click(&window, &movies);
        let card = wait_visible(&window, "Test Movie (2020)").await?;
        click(&window, &card);

        wait_until("the movie", || window.get_screen() == Screen::Movie).await?;
        for label in ["Drama", "Comedy", "2020", "1 h 40 min", "8.4", "TV-14"] {
            wait_visible(&window, label).await?;
        }
        wait_visible(&window, "A generated test movie.").await?;
        wait_role(&window, "Cast and crew", AccessibleRole::List).await?;
        wait_role(&window, "Ada Actor", AccessibleRole::Button).await?;
        wait_role(&window, "Resume from 0:03", AccessibleRole::Button).await?;
        wait_role(&window, "Play", AccessibleRole::Button).await?;

        let favorite = wait_role(&window, "Remove from favorites", AccessibleRole::Button).await?;
        click(&window, &favorite);
        wait_until("the failed request", || {
            unfavored.load(Ordering::SeqCst) == 1
        })
        .await?;
        wait_until("the notice", || window.get_notice().visible).await?;
        if window.get_notice().message != Message::ServerError {
            return Err(format!("unexpected notice: {:?}", window.get_notice()));
        }
        wait_role(&window, "Remove from favorites", AccessibleRole::Button).await?;

        let watched = wait_role(&window, "Mark as watched", AccessibleRole::Button).await?;
        click(&window, &watched);
        wait_visible(&window, "Mark as unwatched").await?;
        if played.load(Ordering::SeqCst) != 1 {
            return Err("Mark as watched did not post once".into());
        }
        if exists(&window, "Resume from 0:03") {
            return Err("a watched movie still offers to resume".into());
        }

        #[cfg(not(feature = "mpv"))]
        {
            let play = wait_role(&window, "Play", AccessibleRole::Button).await?;
            click(&window, &play);
            wait_until("the playback notice", || {
                window.get_notice().visible
                    && window.get_notice().message == Message::PlaybackUnavailable
            })
            .await?;
        }

        let back = wait_role(&window, "Back", AccessibleRole::Button).await?;
        click(&window, &back);
        wait_until("the library", || window.get_screen() == Screen::Library).await?;
        let card = wait_visible(&window, "Second Feature (2021)").await?;
        click(&window, &card);
        wait_until("the second movie", || window.get_screen() == Screen::Movie).await?;
        wait_visible(&window, "Drama").await?;
        wait_role(&window, "Play", AccessibleRole::Button).await?;
        if exists(&window, "Resume from 0:03") || exists(&window, "Resume from 0:00") {
            return Err("a movie without a position offers to resume".into());
        }
        Ok(())
    });
}
