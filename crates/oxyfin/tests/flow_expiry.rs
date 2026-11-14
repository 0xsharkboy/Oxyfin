//! A restored session the server rejects everywhere leads back to sign-in once.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::{USER_ID, fixture, json, pause, run, save_session, start_with, wait_until};
use oxyfin::{Message, Screen, ServerState};
use oxyfin_core::account_key;
use slint::ComponentHandle;
use wiremock::matchers::{any, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

struct Counted {
    calls: Arc<AtomicUsize>,
    response: ResponseTemplate,
}

impl Respond for Counted {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.response.clone()
    }
}

async fn mount_rejecting(mock: MockServer, checks: Arc<AtomicUsize>) -> MockServer {
    Mock::given(method("GET"))
        .and(path("/System/Info/Public"))
        .respond_with(Counted {
            calls: checks,
            response: json(fixture("system_info_public")),
        })
        .mount(&mock)
        .await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(401))
        .with_priority(10)
        .mount(&mock)
        .await;
    mock
}

#[test]
fn rejected_session_expires_once() {
    let checks = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&checks);
    let flow = start_with(move |mock| mount_rejecting(mock, calls), save_session);
    let window = flow.window.as_weak();
    let uri = flow.mock.uri();
    let account = account_key(&uri, USER_ID);

    run(&flow, async move {
        let window = window.upgrade().ok_or("window dropped")?;
        wait_until("the sign-in screen", || {
            window.get_screen() == Screen::SignIn
        })
        .await?;
        wait_until("the server check", || {
            window.get_server_status().state == ServerState::Found
        })
        .await?;
        pause(Duration::from_millis(300)).await;

        if window.get_screen() != Screen::SignIn {
            return Err("the sign-in screen did not stay".into());
        }
        if window.get_server_address() != uri.as_str() {
            return Err(format!(
                "address not prefilled: {:?}",
                window.get_server_address()
            ));
        }
        let notice = window.get_notice();
        if !notice.visible || notice.message != Message::SessionExpired || !notice.dismissible {
            return Err(format!("unexpected notice: {notice:?}"));
        }
        let checks = checks.load(Ordering::SeqCst);
        if checks != 1 {
            return Err(format!("{checks} sign-in screens shown instead of one"));
        }
        Ok(())
    });

    let token = flow
        .vault
        .load(&account)
        .unwrap_or_else(|err| panic!("cannot read the vault: {err}"));
    assert!(token.is_none(), "the token is still saved");
}
