//! A page whose new answer fails keeps its last one, and the user is told.

mod common;

use common::answers::{Answers, open_tab, titles, visit_movies};
use common::{run, save_session, start_with, wait_until};
use oxyfin::Message;
use slint::ComponentHandle;

#[test]
fn a_failed_answer_keeps_the_last_one_and_shows_a_notice() {
    let answers = Answers::holding(&[1, 2, 3]);
    let server = answers.clone();
    let flow = start_with(move |mock| server.mount(mock), save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        visit_movies(&window).await?;
        open_tab(&window, "Shows").await?;
        wait_until("the shows", || titles(&window).is_empty()).await?;
        answers.held().failing = true;
        open_tab(&window, "Movies").await?;
        wait_until("the notice", || {
            let notice = window.get_notice();
            notice.visible && notice.message == Message::ServerError
        })
        .await?;
        if titles(&window) != ["Movie 1", "Movie 2", "Movie 3"] {
            return Err("the last answer is gone".into());
        }
        if !window.get_library_error().is_empty() {
            return Err("the page shows an error".into());
        }
        Ok(())
    });
}
