//! Playback info, stream source and report tests against a mock server.

mod common;

use common::{TEST_TOKEN, TEST_USER_ID, authorization_is, session, variants};
use oxyfin_api::{ItemId, MediaSourceId, MediaStreamType, PlaySessionId, PlaybackReport, Ticks};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn report() -> PlaybackReport {
    PlaybackReport {
        item_id: "abc".into(),
        media_source_id: "def".into(),
        play_session_id: Some(PlaySessionId::from("ps1")),
        position: Ticks(50_000_000),
        is_paused: false,
        audio_stream_index: Some(1),
        subtitle_stream_index: None,
    }
}

#[tokio::test]
async fn playback_info_decodes() {
    for (label, body) in variants("playback_info") {
        let mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Items/abc/PlaybackInfo"))
            .and(authorization_is(Some(TEST_TOKEN)))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .expect(1)
            .mount(&mock)
            .await;
        let info = session(&mock)
            .playback_info(&ItemId::from("abc"))
            .await
            .unwrap();
        assert!(info.play_session_id.is_some(), "{label}");
        assert_eq!(info.media_sources.len(), 1, "{label}");
        let source = &info.media_sources[0];
        assert!(source.supports_direct_play, "{label}");
        let audio: Vec<_> = source
            .media_streams
            .iter()
            .filter(|s| s.r#type == MediaStreamType::Audio)
            .collect();
        assert_eq!(audio.len(), 2, "{label}");
        assert_eq!(audio[0].language.as_deref(), Some("eng"), "{label}");
        let second = audio[1].language.as_deref();
        assert!(matches!(second, Some("fre" | "fra")), "{label}: {second:?}");
        let subtitles: Vec<_> = source
            .media_streams
            .iter()
            .filter(|s| s.r#type == MediaStreamType::Subtitle)
            .collect();
        assert_eq!(subtitles.len(), 1, "{label}");
        assert!(subtitles[0].is_external, "{label}");
    }
}

#[tokio::test]
async fn playback_info_posts_direct_play_body() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/Items/abc/PlaybackInfo"))
        .and(query_param("userId", TEST_USER_ID))
        .and(header("accept", "application/json"))
        .and(body_json(json!({
            "UserId": TEST_USER_ID,
            "EnableDirectPlay": true,
            "EnableDirectStream": true,
            "EnableTranscoding": false,
            "AutoOpenLiveStream": false,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaSources": []})))
        .expect(1)
        .mount(&mock)
        .await;
    let info = session(&mock)
        .playback_info(&ItemId::from("abc"))
        .await
        .unwrap();
    assert!(info.media_sources.is_empty());
    assert!(info.play_session_id.is_none());
}

#[tokio::test]
async fn stream_source_url_and_header() {
    let mock = MockServer::start().await;
    let source = session(&mock).stream_source(&"abc".into(), &MediaSourceId::from("def"));
    assert_eq!(
        source.url.as_str(),
        format!(
            "{}/Videos/abc/stream?static=true&mediaSourceId=def",
            mock.uri()
        )
    );
    assert!(
        source.authorization.ends_with(r#"Token="test-token""#),
        "{}",
        source.authorization
    );
    assert!(!source.url.as_str().contains(TEST_TOKEN));
}

#[tokio::test]
async fn subtitle_url_format() {
    let mock = MockServer::start().await;
    let url = session(&mock).subtitle_url(&"abc".into(), &MediaSourceId::from("def"), 3, "srt");
    assert_eq!(
        url.as_str(),
        format!("{}/Videos/abc/def/Subtitles/3/Stream.srt", mock.uri())
    );
    assert!(!url.as_str().contains(TEST_TOKEN));
}

#[tokio::test]
async fn reports_post_expected_bodies() {
    let base = json!({
        "ItemId": "abc",
        "MediaSourceId": "def",
        "PlaySessionId": "ps1",
        "PositionTicks": 50_000_000,
        "IsPaused": false,
        "AudioStreamIndex": 1,
    });
    let mut with_method = base.clone();
    with_method["PlayMethod"] = json!("DirectPlay");

    let mock = MockServer::start().await;
    for (route, body) in [
        ("/Sessions/Playing", &with_method),
        ("/Sessions/Playing/Progress", &with_method),
        ("/Sessions/Playing/Stopped", &base),
    ] {
        Mock::given(method("POST"))
            .and(path(route))
            .and(authorization_is(Some(TEST_TOKEN)))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&mock)
            .await;
    }
    let session = session(&mock);
    session.report_playback_start(&report()).await.unwrap();
    session.report_playback_progress(&report()).await.unwrap();
    session.report_playback_stopped(&report()).await.unwrap();
}

#[tokio::test]
async fn reports_omit_unset_fields() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/Sessions/Playing/Stopped"))
        .and(body_json(json!({
            "ItemId": "abc",
            "MediaSourceId": "def",
            "PositionTicks": 0,
            "IsPaused": true,
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&mock)
        .await;
    let bare = PlaybackReport {
        play_session_id: None,
        position: Ticks(0),
        is_paused: true,
        audio_stream_index: None,
        ..report()
    };
    session(&mock).report_playback_stopped(&bare).await.unwrap();
}

#[tokio::test]
async fn stream_source_debug_redacts() {
    let mock = MockServer::start().await;
    let source = session(&mock).stream_source(&"abc".into(), &MediaSourceId::from("def"));
    let debug = format!("{source:?}");
    assert!(!debug.contains(TEST_TOKEN), "{debug}");
    assert!(debug.contains("<redacted>"), "{debug}");
}
