//! Played and favorite request tests against a mock server.

mod common;

use common::{
    TEST_TOKEN, TEST_USER_ID, authorization_is, fixture, session, to_pascal_case, variants,
};
use oxyfin_api::{Error, ItemId, Ticks};
use serde_json::Value;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn version_of(label: &str) -> &str {
    label.split('/').next().unwrap_or(label)
}

async fn mount(mock: &MockServer, verb: &str, route: &str, body: &Value) {
    Mock::given(method(verb))
        .and(path(route))
        .and(header("accept", "application/json"))
        .and(authorization_is(Some(TEST_TOKEN)))
        .and(query_param("userId", TEST_USER_ID))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(mock)
        .await;
}

#[tokio::test]
async fn set_played_true_posts() {
    for (label, body) in variants("user_played_items") {
        let captured = to_pascal_case(fixture(version_of(&label), "user_played_items"));
        let mock = MockServer::start().await;
        mount(&mock, "POST", "/UserPlayedItems/abc", &body).await;
        let data = session(&mock)
            .set_played(&ItemId::from("abc"), true)
            .await
            .unwrap();
        assert!(data.played, "{label}");
        assert_eq!(
            data.play_count,
            captured["PlayCount"].as_u64().unwrap() as u32,
            "{label}"
        );
    }
}

#[tokio::test]
async fn set_played_false_deletes() {
    for (label, body) in variants("user_played_items") {
        let mock = MockServer::start().await;
        mount(&mock, "DELETE", "/UserPlayedItems/abc", &body).await;
        let data = session(&mock)
            .set_played(&ItemId::from("abc"), false)
            .await
            .unwrap();
        assert!(data.played, "{label}: the body is decoded as served");
    }
}

#[tokio::test]
async fn set_favorite_true_posts() {
    for (label, body) in variants("user_favorite_items") {
        let mock = MockServer::start().await;
        mount(&mock, "POST", "/UserFavoriteItems/abc", &body).await;
        let data = session(&mock)
            .set_favorite(&ItemId::from("abc"), true)
            .await
            .unwrap();
        assert!(data.is_favorite, "{label}");
    }
}

#[tokio::test]
async fn set_favorite_false_deletes() {
    for (label, body) in variants("user_favorite_items") {
        let mock = MockServer::start().await;
        mount(&mock, "DELETE", "/UserFavoriteItems/abc", &body).await;
        let data = session(&mock)
            .set_favorite(&ItemId::from("abc"), false)
            .await
            .unwrap();
        assert!(data.is_favorite, "{label}: the body is decoded as served");
    }
}

#[tokio::test]
async fn set_played_reports_status_errors() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/UserPlayedItems/abc"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&mock)
        .await;
    let err = session(&mock)
        .set_played(&ItemId::from("abc"), true)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Status { status: 404, .. }), "{err:?}");
}

#[tokio::test]
async fn hide_from_resume_clears_the_position() {
    for (label, body) in variants("user_played_items") {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/UserItems/abc/UserData"))
            .and(header("accept", "application/json"))
            .and(authorization_is(Some(TEST_TOKEN)))
            .and(query_param("userId", TEST_USER_ID))
            .and(body_json(serde_json::json!({ "PlaybackPositionTicks": 0 })))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let data = session(&mock)
            .hide_from_resume(&ItemId::from("abc"))
            .await
            .unwrap();
        assert!(data.played, "{label}: the body is decoded as served");
    }
}

#[tokio::test]
async fn hide_from_resume_reports_status_errors() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/UserItems/abc/UserData"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&mock)
        .await;
    let err = session(&mock)
        .hide_from_resume(&ItemId::from("abc"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Status { status: 403, .. }), "{err:?}");
}

#[tokio::test]
async fn set_resume_position_sends_the_position() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/UserItems/abc/UserData"))
        .and(authorization_is(Some(TEST_TOKEN)))
        .and(query_param("userId", TEST_USER_ID))
        .and(body_json(
            serde_json::json!({ "PlaybackPositionTicks": 7_200_000_000_i64 }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "Played": false, "IsFavorite": false, "PlaybackPositionTicks": 7_200_000_000_i64,
            "PlayCount": 1
        })))
        .expect(1)
        .mount(&mock)
        .await;
    let data = session(&mock)
        .set_resume_position(&ItemId::from("abc"), Ticks(7_200_000_000))
        .await
        .unwrap();
    assert_eq!(data.playback_position_ticks, Ticks(7_200_000_000));
}
