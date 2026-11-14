//! A refused password shows the error under the form and keeps the sign-in screen.

mod common;

use common::{mount_public_info, run, start, wait_until};
use oxyfin::{Message, Messages, Screen, ServerState};
use slint::ComponentHandle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

#[test]
fn refused_password_shows_an_error() {
    let flow = start(|mock| async {
        mount_public_info(&mock).await;
        Mock::given(method("POST"))
            .and(path("/Users/AuthenticateByName"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock)
            .await;
        mock
    });
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

        window.invoke_sign_in("user".into(), "wrong".into());
        let expected = window
            .global::<Messages>()
            .invoke_message_text(Message::InvalidCredentials, "".into());
        wait_until("the credentials error", || {
            window.get_form_error() == expected
        })
        .await?;
        if window.get_screen() != Screen::SignIn {
            return Err("the sign-in screen was left".into());
        }
        if window.get_busy() {
            return Err("the form stayed busy".into());
        }
        Ok(())
    });
}
