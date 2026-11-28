//! A grid asked for again after playback keeps what it shows when the server fails, and tells
//! it in the notice.
#![cfg(feature = "mpv")]

mod common;

use std::rc::Rc;

use common::answers::{Answers, titles, visit_movies};
use common::{run, save_session, start_with, wait_until};
use oxyfin::Message;
use oxyfin_core::PlaybackEvent;
use slint::ComponentHandle;

#[test]
fn a_failed_refresh_keeps_the_grid() {
    let answers = Answers::holding(&[1, 2, 3]);
    let server = answers.clone();
    let flow = start_with(move |mock| server.mount(mock), save_session);
    let window = flow.window.as_weak();
    let app = Rc::clone(&flow.app);

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        visit_movies(&window).await?;
        answers.held().failing = true;
        app.show_playback(PlaybackEvent::StopReported);
        wait_until("the notice", || window.get_notice().visible).await?;
        if window.get_notice().message != Message::ServerError {
            return Err(format!("{:?}", window.get_notice()));
        }
        if titles(&window) != ["Movie 1", "Movie 2", "Movie 3"] {
            return Err(format!("the grid changed: {:?}", titles(&window)));
        }
        if !window.get_library_error().is_empty() {
            return Err("the grid gave way to an error".into());
        }
        Ok(())
    });
}
