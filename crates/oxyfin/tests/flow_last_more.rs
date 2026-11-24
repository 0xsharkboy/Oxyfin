//! The next page of a grid is asked for well before its end, also while a new answer waits for
//! the pointer to leave the cards.

mod common;

use common::answers::{Answers, SLOW, open_tab, titles, visit_movies};
use common::{pause, run, save_session, start_with, wait_role, wait_until};
use i_slint_backend_testing::AccessibleRole;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, LogicalSize};
use std::time::Duration;

const NEXT: [&str; 3] = ["/Items?", "includeItemTypes=Movie", "startIndex=60"];

#[test]
fn scrolling_asks_for_the_next_page_while_a_new_answer_waits() {
    let answers = Answers::holding(&(1..=130).collect::<Vec<usize>>());
    let server = answers.clone();
    let flow = start_with(move |mock| server.mount(mock), save_session);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        window.window().set_size(LogicalSize::new(1280., 720.));
        visit_movies(&window).await?;
        open_tab(&window, "Shows").await?;
        wait_until("the shows", || titles(&window).is_empty()).await?;
        answers.held().slow = true;
        answers.held().movies.insert(0, 999);
        open_tab(&window, "Movies").await?;
        let card = wait_role(&window, "Movie 2", AccessibleRole::Button).await?;
        let at = card.absolute_position();
        let over = LogicalPosition::new(at.x + 30., at.y + 30.);
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: over });
        pause(SLOW + Duration::from_millis(400)).await;
        if titles(&window).first().map(String::as_str) != Some("Movie 1") {
            return Err("the new answer did not wait".into());
        }
        if answers.asked(&NEXT) != 0 {
            return Err("the next page is asked for too early".into());
        }
        // Between one and two views are left below.
        window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: over,
                delta_x: 0.,
                delta_y: -1800.,
            });
        wait_until("the next page", || answers.asked(&NEXT) == 1).await?;
        wait_until("its cards", || titles(&window).len() == 120).await?;
        if titles(&window).first().map(String::as_str) != Some("Movie 999") {
            return Err("the new answer is lost".into());
        }
        Ok(())
    });
}
