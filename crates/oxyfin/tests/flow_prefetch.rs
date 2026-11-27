//! Once a page has loaded, what is likely to be opened next is asked for once: the libraries from
//! the home screen, the movie whose card is hovered.

mod common;

use common::answers::{Answers, id, open_tab, titles};
use common::{click_at, run, save_session, start_with, wait_role, wait_until};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{HoverCard, Screen};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};

const MOVIES: [&str; 2] = ["/Items?", "includeItemTypes=Movie"];
const SHOWS: [&str; 2] = ["/Items?", "includeItemTypes=Series"];

#[test]
fn what_is_opened_next_was_asked_for_once() {
    let answers = Answers::holding(&[1, 2, 3]);
    answers.held().posters = true;
    let server = answers.clone();
    let flow = start_with(move |mock| server.mount(mock), save_session);
    flow.app.set_prefetch(true);
    let app = std::rc::Rc::clone(&flow.app);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_until("the libraries", || {
            answers.asked(&SHOWS) == 1 && answers.asked(&["/Genres"]) == 2
        })
        .await?;
        wait_until("the background done", || {
            !app.prefetching() && answers.asked(&["/Images/Primary", "tag=tag&"]) >= 3
        })
        .await?;
        let posters = answers.asked(&["/Images/Primary", "tag=tag&"]);
        let listings = answers.asked(&MOVIES) + answers.asked(&SHOWS);
        if (listings, answers.asked(&["/Genres"]), posters) != (2, 2, 3) {
            return Err(format!("too many requests: {:?}", answers.held().asked));
        }

        answers.hold();
        open_tab(&window, "Movies").await?;
        if titles(&window) != ["Movie 1", "Movie 2", "Movie 3"] || window.get_library_loading() {
            return Err("Movies waits for the server".into());
        }
        // The page is asked for anew, and nothing else is: the rest was answered a moment ago.
        wait_until("the page asked again", || answers.asked(&MOVIES) == 2).await?;
        answers.release();
        wait_until("the background done", || !app.prefetching()).await?;
        if (answers.asked(&MOVIES), answers.asked(&SHOWS)) != (2, 1) {
            return Err(format!("too many requests: {:?}", answers.held().asked));
        }

        let card = wait_role(&window, "Movie 2", AccessibleRole::Button).await?;
        let at = card.absolute_position();
        let over = LogicalPosition::new(at.x + 30., at.y + 30.);
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: over });
        wait_until("the hover card", || {
            window.global::<HoverCard>().get_details().id == id(2).as_str()
        })
        .await?;
        click_at(&window, over);
        if window.get_screen() != Screen::Movie || !window.get_movie_loaded() {
            return Err("the movie page waits for the server".into());
        }
        Ok(())
    });
}
