//! Signs in with Quick Connect, from the code to the home screen.

mod common;

use common::{QUICK_CONNECT_CODE, run, server, start, wait_until};
use oxyfin::{Screen, ServerState};
use slint::ComponentHandle;

#[test]
fn quick_connect_reaches_home() {
    let flow = start(|_| server());
    let window = flow.window.as_weak();
    let uri = flow.mock.uri();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;
        window.invoke_server_changed(uri.as_str().into());
        wait_until("the server", || {
            window.get_server_status().state == ServerState::Found
        })
        .await?;

        window.invoke_open_quick_connect();
        wait_until("the code", || {
            window.get_quick_connect_code() == QUICK_CONNECT_CODE
        })
        .await?;
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        Ok(())
    });
}
