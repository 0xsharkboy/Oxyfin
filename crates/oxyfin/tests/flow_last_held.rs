//! A new answer behind the last one waits while the pointer rests on a card it would move.

mod common;

use common::answers::{Answers, SLOW, open_tab, titles, visit_movies, watched};
use common::{pause, run, save_session, start_with, wait_role, wait_until};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::Screen;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};
use std::time::Duration;

#[test]
fn a_new_answer_waits_for_the_pointer_to_leave_the_cards() {
    let answers = Answers::holding(&[1, 2, 3]);
    let server = answers.clone();
    let flow = start_with(move |mock| server.mount(mock), save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        visit_movies(&window).await?;
        open_tab(&window, "Shows").await?;
        wait_until("the shows", || {
            window.get_screen() == Screen::Library && titles(&window).is_empty()
        })
        .await?;
        answers.held().slow = true;
        answers.held().movies = vec![9, 1, 2, 3];
        open_tab(&window, "Movies").await?;
        let card = wait_role(&window, "Movie 2", AccessibleRole::Button).await?;
        let at = card.absolute_position();
        let over = LogicalPosition::new(at.x + 30., at.y + 30.);
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: over });
        let asked = answers.asked(&["/Items?", "includeItemTypes=Movie"]);
        if asked != 2 {
            return Err(format!("the movies were asked {asked} times"));
        }
        pause(SLOW + Duration::from_millis(500)).await;
        if titles(&window) != ["Movie 1", "Movie 2", "Movie 3"] {
            return Err("the card under the pointer moved".into());
        }
        if watched(&window, 2) != Some(false) {
            return Err("Movie 2 is gone".into());
        }
        window.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(2., 2.),
        });
        wait_until("the new answer", || {
            titles(&window) == ["Movie 9", "Movie 1", "Movie 2", "Movie 3"]
        })
        .await
    });
}
