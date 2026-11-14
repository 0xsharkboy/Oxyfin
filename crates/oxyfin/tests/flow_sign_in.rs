//! Signs in with a password, from an empty session store to the home screen.

mod common;

use std::time::Duration;

use common::{SERVER_NAME, SERVER_VERSION, fixture, json, pause, run, server, start, wait_until};
use oxyfin::{Screen, ServerState};
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

#[test]
fn password_sign_in_reaches_home() {
    let flow = start(|_| server());
    let window = flow.window.as_weak();
    let uri = flow.mock.uri();
    let slow = flow.runtime.block_on(async {
        let slow = MockServer::start().await;
        let info = fixture("system_info_public").replace(SERVER_NAME, "slow-server");
        Mock::given(method("GET"))
            .and(path("/System/Info/Public"))
            .respond_with(json(info).set_delay(Duration::from_millis(400)))
            .mount(&slow)
            .await;
        slow
    });
    let slow_uri = slow.uri();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;

        window.invoke_server_changed("not a url".into());
        wait_until("an invalid address", || {
            window.get_server_status().state == ServerState::Error
        })
        .await?;

        window.invoke_server_changed("not a url".into());
        window.invoke_server_changed(uri.as_str().into());
        wait_until("the server", || {
            window.get_server_status().state == ServerState::Found
        })
        .await?;
        pause(Duration::from_millis(800)).await;
        let status = window.get_server_status();
        if status.state != ServerState::Found {
            return Err(format!(
                "a stale check replaced the server: {:?}",
                status.state
            ));
        }
        if !status.text.contains(SERVER_NAME) || !status.text.contains(SERVER_VERSION) {
            return Err(format!(
                "server status lacks name or version: {}",
                status.text
            ));
        }

        window.invoke_server_confirmed(slow_uri.as_str().into());
        window.invoke_server_confirmed(uri.as_str().into());
        wait_until("the fast server", || {
            window.get_server_status().state == ServerState::Found
        })
        .await?;
        pause(Duration::from_millis(800)).await;
        let status = window.get_server_status();
        if status.state != ServerState::Found || status.text.contains("slow-server") {
            return Err(format!(
                "a stale answer replaced the server: {}",
                status.text
            ));
        }

        window.invoke_sign_in("user".into(), "pass".into());
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        Ok(())
    });
}
