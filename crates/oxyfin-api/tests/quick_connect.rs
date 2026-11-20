//! Quick Connect tests against a mock server.

mod common;

use common::{TEST_TOKEN, authorization_is, fixture, server, session, to_pascal_case, variants};
use oxyfin_api::{Error, QuickConnectSecret};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn version_of(label: &str) -> &str {
    label.split('/').next().unwrap_or(label)
}

#[tokio::test]
async fn enabled_decodes_bool() {
    for (label, body) in variants("quick_connect_enabled") {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/QuickConnect/Enabled"))
            .and(header("accept", "application/json"))
            .and(authorization_is(None))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let expected = fixture(version_of(&label), "quick_connect_enabled")
            .as_bool()
            .unwrap();
        assert_eq!(
            server(&mock).quick_connect_enabled().await.unwrap(),
            expected,
            "{label}"
        );
    }
}

#[tokio::test]
async fn initiate_decodes_code_and_secret() {
    for (label, body) in variants("quick_connect_initiate") {
        let captured = to_pascal_case(fixture(version_of(&label), "quick_connect_initiate"));
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/QuickConnect/Initiate"))
            .and(authorization_is(None))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let state = server(&mock).quick_connect_initiate().await.unwrap();
        assert_eq!(state.code, captured["Code"].as_str().unwrap(), "{label}");
        assert_eq!(
            state.secret.expose(),
            captured["Secret"].as_str().unwrap(),
            "{label}"
        );
        assert_eq!(
            state.authenticated,
            captured["Authenticated"].as_bool().unwrap(),
            "{label}"
        );
    }
}

#[tokio::test]
async fn poll_sends_secret_query() {
    for (label, body) in variants("quick_connect_connect") {
        let captured = to_pascal_case(fixture(version_of(&label), "quick_connect_connect"));
        let secret = QuickConnectSecret::from("a b&c".to_owned());
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/QuickConnect/Connect"))
            .and(query_param("secret", "a b&c"))
            .and(authorization_is(None))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let state = server(&mock).quick_connect_poll(&secret).await.unwrap();
        assert_eq!(state.code, captured["Code"].as_str().unwrap(), "{label}");
        assert_eq!(
            state.authenticated,
            captured["Authenticated"].as_bool().unwrap(),
            "{label}"
        );
    }
}

#[tokio::test]
async fn authenticate_posts_secret_and_returns_session() {
    for (label, body) in variants("authenticate_by_name") {
        let captured = to_pascal_case(fixture(version_of(&label), "authenticate_by_name"));
        let secret = QuickConnectSecret::from("s3cr3t".to_owned());
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Users/AuthenticateWithQuickConnect"))
            .and(authorization_is(None))
            .and(body_json(json!({"Secret": "s3cr3t"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let session = server(&mock)
            .quick_connect_authenticate(&secret)
            .await
            .unwrap();
        assert_eq!(
            session.token(),
            captured["AccessToken"].as_str().unwrap(),
            "{label}"
        );
        assert_eq!(
            session.user_id().as_str(),
            captured["User"]["Id"].as_str().unwrap(),
            "{label}"
        );
        assert_eq!(
            session.server_id(),
            captured["ServerId"].as_str(),
            "{label}"
        );
    }
}

#[tokio::test]
async fn secret_not_in_debug() {
    let captured: Value = to_pascal_case(fixture("12.4.0", "quick_connect_initiate"));
    let raw = captured["Secret"].as_str().unwrap().to_owned();
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/QuickConnect/Initiate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&captured))
        .mount(&mock)
        .await;
    let state = server(&mock).quick_connect_initiate().await.unwrap();
    let shown = format!("{state:?} {:?}", state.secret);
    assert!(!shown.contains(&raw), "{shown}");
    assert!(shown.contains("QuickConnectSecret(..)"), "{shown}");
}

#[tokio::test]
async fn secret_not_in_errors() {
    let secret = QuickConnectSecret::from("s3cr3t".to_owned());
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/QuickConnect/Connect"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Unknown secret"))
        .mount(&mock)
        .await;
    let err = server(&mock).quick_connect_poll(&secret).await.unwrap_err();
    assert!(matches!(err, Error::Status { status: 404, .. }));
    let shown = format!("{err} {err:?}");
    assert!(!shown.contains("s3cr3t"), "{shown}");

    let dead = {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        oxyfin_api::Server::new(&format!("http://127.0.0.1:{port}"), common::client_info()).unwrap()
    };
    let err = dead.quick_connect_poll(&secret).await.unwrap_err();
    let shown = format!("{err} {err:?}");
    assert!(!shown.contains("s3cr3t"), "{shown}");
}

#[tokio::test]
async fn authorize_posts_code() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/QuickConnect/Authorize"))
        .and(query_param("code", "482913"))
        .and(authorization_is(Some(TEST_TOKEN)))
        .respond_with(ResponseTemplate::new(200).set_body_json(true))
        .expect(1)
        .mount(&mock)
        .await;
    assert!(
        session(&mock)
            .quick_connect_authorize("482913")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn authorize_wrong_code_false() {
    for status in [400, 404] {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/QuickConnect/Authorize"))
            .respond_with(
                ResponseTemplate::new(status).set_body_string("Error processing request."),
            )
            .expect(1)
            .mount(&mock)
            .await;
        assert!(
            !session(&mock)
                .quick_connect_authorize("000000")
                .await
                .unwrap(),
            "HTTP {status}"
        );
    }
}

#[tokio::test]
async fn authorize_when_disabled_is_a_distinct_error() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/QuickConnect/Authorize"))
        .respond_with(ResponseTemplate::new(401).set_body_json("Quick connect is disabled"))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/QuickConnect/Enabled"))
        .respond_with(ResponseTemplate::new(200).set_body_json(false))
        .expect(1)
        .mount(&mock)
        .await;
    let err = session(&mock)
        .quick_connect_authorize("482913")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::QuickConnectDisabled), "{err:?}");
    assert!(!err.is_unauthorized());
}

#[tokio::test]
async fn authorize_with_a_rejected_token_stays_unauthorized() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/QuickConnect/Authorize"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/QuickConnect/Enabled"))
        .respond_with(ResponseTemplate::new(200).set_body_json(true))
        .expect(1)
        .mount(&mock)
        .await;
    let err = session(&mock)
        .quick_connect_authorize("482913")
        .await
        .unwrap_err();
    assert!(err.is_unauthorized(), "{err:?}");
}

#[tokio::test]
async fn authorize_other_failures_are_errors() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/QuickConnect/Authorize"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;
    let err = session(&mock)
        .quick_connect_authorize("482913")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Status { status: 500, .. }), "{err:?}");
}
