//! Nothing answers on the local network: the sign-in card shows no server list.

mod common;

use std::time::Duration;

use common::discovery::responder;
use common::{exists, pause, run, server, start, wait_until};
use oxyfin::Screen;
use slint::{ComponentHandle, Model};

#[test]
fn no_section_when_nothing_found() {
    let flow = start(|_| server());
    flow.app.set_discovery_targets(vec![responder(None)]);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;
        pause(Duration::from_millis(2500)).await;
        if exists(&window, "On your network") || window.get_found_servers().row_count() != 0 {
            return Err("the section shows without any server".to_owned());
        }
        Ok(())
    });
}
