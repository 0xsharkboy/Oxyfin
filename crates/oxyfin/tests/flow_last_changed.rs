//! What the user marks is never undone by an answer asked for before: not on the page, and not on
//! a later visit.

mod common;

use common::answers::{Answers, SLOW, open_tab, titles, visit_movies, watched};
use common::menu::{choose, right_click, wait_menu};
use common::{pause, run, save_session, start_with, wait_role, wait_until};
use i_slint_backend_testing::AccessibleRole;
use slint::ComponentHandle;
use std::time::Duration;

#[test]
fn a_movie_marked_watched_stays_so_whatever_was_asked_before() {
    let answers = Answers::holding(&[1, 2, 3]);
    let server = answers.clone();
    let flow = start_with(move |mock| server.mount(mock), save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        visit_movies(&window).await?;
        open_tab(&window, "Shows").await?;
        wait_until("the shows", || titles(&window).is_empty()).await?;
        // The last answer shows, with another on its way that holds the movie unwatched.
        answers.held().slow = true;
        open_tab(&window, "Movies").await?;
        answers.held().slow = false;
        let card = wait_role(&window, "Movie 2", AccessibleRole::Button).await?;
        right_click(&window, &card);
        wait_menu(&window, "Movie 2").await?;
        choose(&window, "Mark as watched")?;
        wait_until("the badge", || watched(&window, 2) == Some(true)).await?;
        for visit in ["Shows", "Movies", "Shows", "Movies"] {
            open_tab(&window, visit).await?;
            for _ in 0..8 {
                if watched(&window, 2) == Some(false) {
                    return Err(format!("the badge went away on {visit}"));
                }
                pause(SLOW / 12).await;
            }
        }
        wait_until("the movies", || watched(&window, 2) == Some(true)).await?;
        pause(Duration::from_millis(100)).await;
        Ok(())
    });
}
