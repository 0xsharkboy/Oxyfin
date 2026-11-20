//! Servers found on the local network: listed on the sign-in card, a click checks one.

mod common;

use common::discovery::{reply, responder};
use common::{SERVER_VERSION, click, run, server, start, wait_role, wait_until};
use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use oxyfin::{Screen, ServerState};
use slint::{ComponentHandle, Model};

fn label_y(window: &impl ComponentHandle, label: &str) -> Result<f32, String> {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|element| element.size().height > 0.)
        .map(|element| element.absolute_position().y)
        .ok_or_else(|| format!("no {label}"))
}

#[test]
fn discovered_server_fills_address() {
    let flow = start(|_| server());
    let uri = flow.mock.uri();
    flow.app
        .set_discovery_targets(vec![responder(Some(reply(&uri)))]);
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;

        let row = wait_role(&window, "Maison", AccessibleRole::Button).await?;
        let servers = window.get_found_servers();
        let found = servers.row_data(0).ok_or("no found server")?;
        if found.detail != format!("{uri} · {SERVER_VERSION}") {
            return Err(format!("unexpected detail: {}", found.detail));
        }
        let header = label_y(&window, "On your network")?;
        let address = label_y(&window, "Server address")?;
        let user = label_y(&window, "Username")?;
        let top = row.absolute_position().y;
        if !(address < header && header <= top && top + row.size().height < user) {
            return Err(format!(
                "list out of place: address {address}, header {header}, row {top}, username {user}"
            ));
        }
        if window.get_server_status().state != ServerState::Idle {
            return Err("a check ran before the click".to_owned());
        }

        click(&window, &row);
        wait_until("the address of the server", || {
            window.get_server_address() == uri.as_str()
        })
        .await?;
        wait_until("the server check", || {
            window.get_server_status().state == ServerState::Found
        })
        .await?;
        Ok(())
    });
}
