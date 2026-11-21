//! Subtitle search and download, and the permission to use them, against a mock server.

mod common;

use common::{TEST_TOKEN, authorization_is, fixture, session, to_camel_case, variants};
use oxyfin_api::{Error, ItemId, RemoteSubtitleInfo, UserPolicy};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ITEM: &str = "76282b1c408f374beeee50519e9157a4";
const SUBTITLE: &str = "e02a34f56233e3e5e7765144ca8f184f_srt-fre-4437550";

async fn me_with(body: Value) -> oxyfin_api::UserDto {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Users/Me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&mock)
        .await;
    session(&mock)
        .me()
        .await
        .unwrap_or_else(|err| panic!("cannot read the user: {err}"))
}

#[tokio::test]
async fn user_policy_subtitle_management() {
    for (label, body) in variants("users_me") {
        let policy = me_with(body).await.policy;
        assert_eq!(
            policy,
            UserPolicy {
                enable_subtitle_management: false,
                is_administrator: true,
            },
            "{label}"
        );
        assert!(policy.can_manage_subtitles(), "{label}: administrators may");
    }

    let mut allowed = fixture("12.4.0", "users_me");
    allowed["Policy"]["EnableSubtitleManagement"] = json!(true);
    allowed["Policy"]["IsAdministrator"] = json!(false);
    assert!(me_with(allowed).await.policy.can_manage_subtitles());

    let mut denied = fixture("12.4.0", "users_me");
    denied["Policy"]["IsAdministrator"] = json!(false);
    assert!(!me_with(denied).await.policy.can_manage_subtitles());

    let mut without = fixture("12.4.0", "users_me");
    if let Some(user) = without.as_object_mut() {
        user.remove("Policy");
    }
    let policy = me_with(without).await.policy;
    assert_eq!(policy, UserPolicy::default());
    assert!(!policy.can_manage_subtitles());
}

#[tokio::test]
async fn search_subtitles_query() {
    let captured = fixture("12.4.0", "remote_subtitles_search");
    for (label, body) in [
        ("as captured", captured.clone()),
        ("camelCase", to_camel_case(captured.clone())),
    ] {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/Items/{ITEM}/RemoteSearch/Subtitles/fre")))
            .and(header("accept", "application/json"))
            .and(authorization_is(Some(TEST_TOKEN)))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&mock)
            .await;
        let found = session(&mock)
            .search_subtitles(&ItemId::from(ITEM), "fre")
            .await
            .unwrap();
        assert_eq!(found.len(), 16, "{label}");
        assert_eq!(
            found[0],
            RemoteSubtitleInfo {
                id: SUBTITLE.to_owned(),
                name: "night of the living dead (1969)".to_owned(),
                provider_name: Some("Open Subtitles".to_owned()),
                format: Some("srt".to_owned()),
                download_count: Some(3005),
                is_forced: Some(false),
                is_hearing_impaired: Some(false),
                is_perfect_match: Some(false),
            },
            "{label}"
        );
    }
}

#[tokio::test]
async fn search_subtitles_tags_and_missing_fields() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{ITEM}/RemoteSearch/Subtitles/eng")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"Id": "a", "Name": "A", "Forced": true, "HearingImpaired": true, "IsHashMatch": true},
            {"Id": "b", "Name": "B"},
        ])))
        .mount(&mock)
        .await;
    let found = session(&mock)
        .search_subtitles(&ItemId::from(ITEM), "eng")
        .await
        .unwrap();
    assert_eq!(found[0].is_forced, Some(true));
    assert_eq!(found[0].is_hearing_impaired, Some(true));
    assert_eq!(found[0].is_perfect_match, Some(true));
    assert_eq!(
        found[1],
        RemoteSubtitleInfo {
            id: "b".to_owned(),
            name: "B".to_owned(),
            ..RemoteSubtitleInfo::default()
        }
    );
}

#[tokio::test]
async fn download_posts() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/Items/{ITEM}/RemoteSearch/Subtitles/{SUBTITLE}"
        )))
        .and(authorization_is(Some(TEST_TOKEN)))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&mock)
        .await;
    session(&mock)
        .download_subtitle(&ItemId::from(ITEM), SUBTITLE)
        .await
        .unwrap();
}

#[tokio::test]
async fn download_failure_is_a_status_error() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!(
            "/Items/{ITEM}/RemoteSearch/Subtitles/{SUBTITLE}"
        )))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;
    let error = session(&mock)
        .download_subtitle(&ItemId::from(ITEM), SUBTITLE)
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Status { status: 500, .. }),
        "{error}"
    );
}
