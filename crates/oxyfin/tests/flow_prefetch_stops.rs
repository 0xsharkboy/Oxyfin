//! Nothing is asked for in the background once another page shows, until a page has loaded again.

mod common;

use std::time::Duration;

use common::answers::Answers;
use common::{run, save_session, start_with, wait_until};
use oxyfin::Screen;
use slint::ComponentHandle;

#[test]
fn opening_another_page_stops_what_runs_in_the_background() {
    let answers = Answers::holding(&[1, 2, 3]);
    let server = answers.clone();
    let flow = start_with(move |mock| server.mount(mock), save_session);
    flow.app.set_prefetch(true);
    flow.app.set_prefetch_delay(Duration::from_secs(3600));
    let window = flow.window.as_weak();
    let app = std::rc::Rc::clone(&flow.app);

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the home screen", || window.get_home_settled()).await?;
        if !app.prefetching() {
            return Err("nothing waits to be asked for in the background".into());
        }
        window.invoke_open_settings();
        wait_until("the background stopped", || !app.prefetching()).await?;
        if answers.asked(&["/Items?"]) != 0 {
            return Err(format!("asked for: {:?}", answers.held().asked));
        }
        app.set_prefetch_delay(Duration::ZERO);
        window.invoke_close_settings();
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        wait_until("the libraries", || answers.asked(&["/Items?"]) == 2).await
    });
}
