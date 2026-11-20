//! A malformed discovery answer shows no server and no error, and sign-in still works.

mod common;

use std::time::Duration;

use common::discovery::responder;
use common::{exists, pause, run, server, start, wait_until};
use oxyfin::{Screen, ServerState};
use slint::ComponentHandle;

#[test]
fn malformed_reply_keeps_sign_in_usable() {
    let flow = start(|_| server());
    flow.app
        .set_discovery_targets(vec![responder(Some(b"{\"Id\": 3".to_vec()))]);
    let window = flow.window.as_weak();
    let uri = flow.mock.uri();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;
        pause(Duration::from_millis(2500)).await;
        if exists(&window, "On your network") {
            return Err("a malformed reply shows a server".to_owned());
        }
        if window.get_server_status().state != ServerState::Idle
            || !window.get_form_error().is_empty()
        {
            return Err("a malformed reply shows an error".to_owned());
        }
        window.invoke_server_changed(uri.as_str().into());
        wait_until("the server", || {
            window.get_server_status().state == ServerState::Found
        })
        .await?;
        Ok(())
    });
}
