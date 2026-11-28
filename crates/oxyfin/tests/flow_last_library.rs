//! A library page opened again shows its last answer at once, then what changed, in place.

mod common;

use common::answers::{Answers, id, open_tab, titles, visit_movies};
use common::{pause, run, save_session, skeleton_shown, start_with, wait_until};
use oxyfin::Screen;
use slint::ComponentHandle;
use std::time::Duration;

#[test]
fn a_library_opened_again_shows_its_last_answer_then_the_new_one() {
    let answers = Answers::holding(&[1, 2, 3]);
    answers.held().posters = true;
    let server = answers.clone();
    let flow = start_with(move |mock| server.mount(mock), save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        visit_movies(&window).await?;
        wait_until("the genres", || answers.asked(&["/Genres"]) == 1).await?;
        let asked = answers.asked(&["/Items?", "includeItemTypes=Movie"]);
        if asked != 1 {
            return Err(format!("the movies were asked {asked} times"));
        }
        open_tab(&window, "Shows").await?;
        wait_until("the shows", || {
            window.get_screen() == Screen::Library && !window.get_library_loading()
        })
        .await?;
        answers.held().slow = true;
        answers.held().movies = vec![9, 1, 3];
        open_tab(&window, "Movies").await?;
        if titles(&window) != ["Movie 1", "Movie 2", "Movie 3"] {
            return Err(format!("not the last answer: {:?}", titles(&window)));
        }
        if window.get_library_loading() || skeleton_shown(&window) {
            return Err("a skeleton shows over the last answer".into());
        }
        if window.get_library_total() != 3 {
            return Err("the total is not the last one".into());
        }
        wait_until("the new answer", || {
            titles(&window) == ["Movie 9", "Movie 1", "Movie 3"]
        })
        .await?;
        pause(Duration::from_millis(200)).await;
        let asked = answers.asked(&["/Items?", "includeItemTypes=Movie"]);
        if asked != 2 {
            return Err(format!("the movies were asked {asked} times"));
        }
        // Images that answer 404 are asked again by the next visit only.
        let posters = answers.asked(&[&format!("/Items/{}/Images", id(1))]);
        if posters > 3 {
            return Err(format!("the poster of Movie 1 was asked {posters} times"));
        }
        Ok(())
    });
}
