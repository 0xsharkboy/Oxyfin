//! A click on "Sign in" while the server is being checked signs in once the server is found.

mod common;

use std::time::Duration;

use common::{click, fixture, json, mount_sign_in, pause, run, start, wait_role, wait_until};
use i_slint_backend_testing::AccessibleRole;
use oxyfin::{Screen, ServerState};
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CHECK_DELAY: Duration = Duration::from_millis(500);

#[test]
fn sign_in_waits_for_the_server_check() {
    let flow = start(|mock| async {
        Mock::given(method("GET"))
            .and(path("/System/Info/Public"))
            .respond_with(json(fixture("system_info_public")).set_delay(CHECK_DELAY))
            .mount(&mock)
            .await;
        mount_sign_in(&mock).await;
        mock
    });
    let failing = flow.runtime.block_on(async {
        let failing = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/System/Info/Public"))
            .respond_with(ResponseTemplate::new(500).set_delay(CHECK_DELAY))
            .mount(&failing)
            .await;
        failing
    });
    let failing_uri = failing.uri();
    let uri = flow.mock.uri();
    let window = flow.window.as_weak();

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;
        window.set_username("user".into());

        window.invoke_server_confirmed(failing_uri.as_str().into());
        if window.get_server_status().state != ServerState::Checking {
            return Err("the failing server is not being checked".into());
        }
        let button = wait_role(&window, "Sign in", AccessibleRole::Button).await?;
        click(&window, &button);
        wait_until("the check error", || {
            window.get_server_status().state == ServerState::Error
        })
        .await?;
        if window.get_busy() {
            return Err("the form stayed busy after the check failed".into());
        }

        window.invoke_server_confirmed(uri.as_str().into());
        wait_until("the server", || {
            window.get_server_status().state == ServerState::Found
        })
        .await?;
        pause(Duration::from_millis(300)).await;
        if window.get_screen() != Screen::SignIn {
            return Err("a click before a failed check signed in later".into());
        }

        window.invoke_server_changed(uri.as_str().into());
        if window.get_server_status().state != ServerState::Checking {
            return Err("the server is not being checked again".into());
        }
        let button = wait_role(&window, "Sign in", AccessibleRole::Button).await?;
        click(&window, &button);
        wait_until("the home screen", || window.get_screen() == Screen::Home).await?;
        Ok(())
    });
}
