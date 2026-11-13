//! Transport, server connection and session tests against a mock server.

mod common;

use common::{
    TEST_TOKEN, TEST_USER_ID, authorization_is, client_info, fixture, server, session,
    to_pascal_case, variants,
};
use oxyfin_api::{Error, MINIMUM_SERVER_VERSION, Server, ServerVersion, Session};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn public_info_decodes_all_fixtures() {
    for (label, body) in variants("system_info_public") {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/System/Info/Public"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&mock)
            .await;
        let info = server(&mock).public_info().await.unwrap();
        let expected = label.split('/').next().unwrap();
        assert_eq!(info.version, expected, "{label}");
        assert!(!info.id.is_empty(), "{label}");
        assert_eq!(info.startup_wizard_completed, Some(true), "{label}");
    }
}

#[tokio::test]
async fn requests_send_accept_and_authorization() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/System/Info/Public"))
        .and(header("accept", "application/json"))
        .and(authorization_is(None))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(fixture("12.4.0", "system_info_public")),
        )
        .expect(1)
        .mount(&mock)
        .await;
    server(&mock).public_info().await.unwrap();
}

#[tokio::test]
async fn check_version_rejects_10_9() {
    let mock = MockServer::start().await;
    let mut body = fixture("10.10.7", "system_info_public");
    body["Version"] = json!("10.9.11");
    Mock::given(method("GET"))
        .and(path("/System/Info/Public"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&mock)
        .await;
    match server(&mock).check_version().await {
        Err(Error::UnsupportedServer { found, minimum }) => {
            assert_eq!(found, "10.9.11".parse::<ServerVersion>().unwrap());
            assert_eq!(minimum, MINIMUM_SERVER_VERSION);
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[tokio::test]
async fn check_version_accepts_supported_servers() {
    for (label, body) in variants("system_info_public") {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/System/Info/Public"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&mock)
            .await;
        let info = server(&mock).check_version().await.unwrap();
        assert!(
            info.server_version().unwrap() >= MINIMUM_SERVER_VERSION,
            "{label}"
        );
    }
}

#[tokio::test]
async fn authenticate_by_name_posts_credentials() {
    for (label, body) in variants("authenticate_by_name") {
        let version = label.split('/').next().unwrap();
        let captured = to_pascal_case(fixture(version, "authenticate_by_name"));
        let token = captured["AccessToken"].as_str().unwrap();
        let user_id = captured["User"]["Id"].as_str().unwrap();

        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Users/AuthenticateByName"))
            .and(header("accept", "application/json"))
            .and(authorization_is(None))
            .and(body_json(json!({"Username": "u", "Pw": "p"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let session = server(&mock).authenticate_by_name("u", "p").await.unwrap();
        assert_eq!(session.token(), token, "{label}");
        assert_eq!(session.user_id().as_str(), user_id, "{label}");
        assert_eq!(
            session.server_id(),
            captured["ServerId"].as_str(),
            "{label}"
        );
        assert_eq!(
            session.server().base_url().as_str(),
            format!("{}/", mock.uri())
        );
    }
}

#[tokio::test]
async fn session_requests_carry_token() {
    for (label, body) in variants("users_me") {
        let version = label.split('/').next().unwrap();
        let captured = to_pascal_case(fixture(version, "users_me"));
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users/Me"))
            .and(header("accept", "application/json"))
            .and(authorization_is(Some(TEST_TOKEN)))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let user = session(&mock).me().await.unwrap();
        assert_eq!(
            user.id.as_str(),
            captured["Id"].as_str().unwrap(),
            "{label}"
        );
        assert_eq!(user.name, "oxyfin", "{label}");
        assert_eq!(
            user.server_id.as_deref(),
            captured["ServerId"].as_str(),
            "{label}"
        );
    }
}

#[tokio::test]
async fn logout_posts_sessions_logout() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/Sessions/Logout"))
        .and(authorization_is(Some(TEST_TOKEN)))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&mock)
        .await;
    session(&mock).logout().await.unwrap();
}

#[test]
fn restore_session_keeps_token_and_user() {
    let server = Server::new("http://127.0.0.1:8096", client_info()).unwrap();
    let session = server.restore_session(TEST_TOKEN, TEST_USER_ID.into(), Some("srv".into()));
    assert_eq!(session.token(), TEST_TOKEN);
    assert_eq!(session.user_id().as_str(), TEST_USER_ID);
    assert_eq!(session.server_id(), Some("srv"));
    assert_eq!(session.clone().token(), TEST_TOKEN);
}

#[tokio::test]
async fn subpath_base_url_is_kept() {
    for suffix in ["/jellyfin", "/jellyfin/"] {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/jellyfin/System/Info/Public"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("12.4.0", "system_info_public")),
            )
            .expect(1)
            .mount(&mock)
            .await;
        let server = Server::new(&format!("{}{suffix}", mock.uri()), client_info()).unwrap();
        assert_eq!(
            server.base_url().as_str(),
            format!("{}/jellyfin/", mock.uri())
        );
        server.public_info().await.unwrap();
    }
}

#[test]
fn base_url_is_trimmed() {
    let server = Server::new("  https://media.example.org  ", client_info()).unwrap();
    assert_eq!(server.base_url().as_str(), "https://media.example.org/");
}

#[test]
fn invalid_base_urls_are_rejected() {
    for input in ["", "jellyfin.local:8096", "ftp://h"] {
        assert!(
            matches!(Server::new(input, client_info()), Err(Error::InvalidUrl(_))),
            "{input:?}"
        );
    }
}

#[test]
fn base_urls_with_credentials_are_rejected() {
    for input in [
        "http://alice:hunter2@host",
        "https://alice@host/jellyfin",
        "http://:hunter2@host",
    ] {
        match Server::new(input, client_info()) {
            Err(err @ Error::InvalidUrl(_)) => {
                let shown = format!("{err} {err:?}");
                assert!(!shown.contains("hunter2"), "{shown}");
                assert!(!shown.contains("alice"), "{shown}");
            }
            other => panic!("{input:?}: unexpected result {other:?}"),
        }
    }
}

#[tokio::test]
async fn html_body_is_a_decode_error() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/System/Info/Public"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw("<!DOCTYPE html><html></html>", "text/html"),
        )
        .mount(&mock)
        .await;
    match server(&mock).public_info().await {
        Err(Error::Decode { route, .. }) => assert_eq!(route, "System/Info/Public"),
        other => panic!("unexpected result: {other:?}"),
    }

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/System/Info/Public"))
        .respond_with(
            ResponseTemplate::new(502).set_body_raw("<!DOCTYPE html><html></html>", "text/html"),
        )
        .mount(&mock)
        .await;
    match server(&mock).public_info().await {
        Err(Error::Status { route, status, .. }) => {
            assert_eq!(route, "System/Info/Public");
            assert_eq!(status, 502);
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[tokio::test]
async fn status_errors() {
    for code in [401u16, 404, 500] {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users/Me"))
            .respond_with(ResponseTemplate::new(code).set_body_string("nope"))
            .mount(&mock)
            .await;
        let err = session(&mock).me().await.unwrap_err();
        match &err {
            Error::Status {
                route,
                status,
                body,
            } => {
                assert_eq!(route, "Users/Me");
                assert_eq!(*status, code);
                assert_eq!(body, "nope");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert_eq!(err.is_unauthorized(), code == 401);
    }
}

#[tokio::test]
async fn secrets_never_in_debug() {
    let mock = MockServer::start().await;
    let session = session(&mock);
    let shown = format!("{session:?} {:?}", session.server());
    assert!(!shown.contains(TEST_TOKEN), "{shown}");

    Mock::given(method("POST"))
        .and(path("/Users/AuthenticateByName"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Invalid username or password"))
        .mount(&mock)
        .await;
    let err = server(&mock)
        .authenticate_by_name("u", "hunter2")
        .await
        .unwrap_err();
    assert!(err.is_unauthorized());
    let shown = format!("{err:?} {err}");
    assert!(!shown.contains("hunter2"), "{shown}");
}

#[test]
fn server_and_session_are_send_sync_clone() {
    fn assert_traits<T: Send + Sync + Clone + 'static>() {}
    assert_traits::<Server>();
    assert_traits::<Session>();
}
