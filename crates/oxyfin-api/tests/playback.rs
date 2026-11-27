//! Playback info, stream source and report tests against a mock server.

mod common;

use common::{TEST_TOKEN, TEST_USER_ID, authorization_is, session, variants};
use oxyfin_api::Server;
use oxyfin_api::{
    ItemId, MediaSourceId, MediaStreamType, PlayMethod, PlaySessionId, PlaybackOptions,
    PlaybackReport, SubtitleDeliveryMethod, Ticks,
};
use serde_json::json;
use wiremock::matchers::{body_json, body_partial_json, header, method, path, query_param};
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
        play_method: PlayMethod::DirectPlay,
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
        assert!(
            source.bitrate.is_some_and(|bitrate| bitrate > 100_000),
            "{label}"
        );
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
        assert_eq!(
            subtitles[0].path.as_deref(),
            Some("/media/movies/Test Movie (2020)/Test Movie (2020).en.srt"),
            "{label}"
        );
    }
}

/// The `PlaybackInfo` body of a request with a bitrate or height limit, before the options.
fn limited_info_body() -> serde_json::Value {
    json!({
        "UserId": TEST_USER_ID,
        "EnableDirectPlay": true,
        "EnableDirectStream": true,
        "EnableTranscoding": true,
        "AllowVideoStreamCopy": true,
        "AllowAudioStreamCopy": true,
        "AutoOpenLiveStream": false,
        "DeviceProfile": {
            "Name": "Oxyfin",
            "DirectPlayProfiles": [{ "Type": "Video" }, { "Type": "Audio" }],
            "TranscodingProfiles": [{
                "Type": "Video",
                "Container": "ts",
                "Protocol": "hls",
                "Context": "Streaming",
                "VideoCodec": "h264",
                "AudioCodec": "aac,mp3,ac3",
                "MaxAudioChannels": "6",
            }],
            "SubtitleProfiles": [
                { "Format": "srt", "Method": "External" },
                { "Format": "subrip", "Method": "External" },
                { "Format": "ass", "Method": "External" },
                { "Format": "ssa", "Method": "External" },
                { "Format": "vtt", "Method": "External" },
                { "Format": "webvtt", "Method": "External" },
            ],
        },
    })
}

/// The `PlaybackInfo` body of a request without any limit.
fn playback_info_body() -> serde_json::Value {
    let mut body = limited_info_body();
    body["MaxStreamingBitrate"] = json!(1_000_000_000);
    body["DeviceProfile"]["MaxStreamingBitrate"] = json!(1_000_000_000);
    if let Some(profiles) = body["DeviceProfile"]["SubtitleProfiles"].as_array_mut() {
        for format in [
            "srt",
            "subrip",
            "ass",
            "ssa",
            "vtt",
            "webvtt",
            "mov_text",
            "text",
            "pgssub",
            "pgs",
            "hdmv_pgs_subtitle",
            "dvdsub",
            "dvd_subtitle",
            "dvbsub",
            "dvb_subtitle",
            "xsub",
        ] {
            profiles.push(json!({ "Format": format, "Method": "Embed" }));
        }
    }
    body
}

#[tokio::test]
async fn playback_info_posts_the_device_profile() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/Items/abc/PlaybackInfo"))
        .and(query_param("userId", TEST_USER_ID))
        .and(header("accept", "application/json"))
        .and(body_json(playback_info_body()))
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
async fn playback_info_posts_the_bitrate_and_the_tracks() {
    let mock = MockServer::start().await;
    let mut body = limited_info_body();
    body["MaxStreamingBitrate"] = json!(4_000_000);
    body["MediaSourceId"] = json!("def");
    body["AudioStreamIndex"] = json!(3);
    body["SubtitleStreamIndex"] = json!(-1);
    Mock::given(method("POST"))
        .and(path("/Items/abc/PlaybackInfo"))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaSources": []})))
        .expect(1)
        .mount(&mock)
        .await;
    let options = PlaybackOptions {
        max_streaming_bitrate: Some(4_000_000),
        media_source_id: Some(MediaSourceId::from("def")),
        audio_stream_index: Some(3),
        subtitle_stream_index: Some(-1),
        max_height: None,
        ..PlaybackOptions::default()
    };
    session(&mock)
        .playback_info_with(&ItemId::from("abc"), &options)
        .await
        .unwrap();
}

#[tokio::test]
async fn limited_quality_sends_a_height_condition() {
    let mock = MockServer::start().await;
    let mut body = limited_info_body();
    body["MaxStreamingBitrate"] = json!(1_500_000);
    body["DeviceProfile"]["CodecProfiles"] = json!([{
        "Type": "Video",
        "Conditions": [{
            "Condition": "LessThanEqual",
            "Property": "Height",
            "Value": "480",
            "IsRequired": false,
        }, {
            "Condition": "LessThanEqual",
            "Property": "Width",
            "Value": "854",
            "IsRequired": false,
        }],
    }]);
    Mock::given(method("POST"))
        .and(path("/Items/abc/PlaybackInfo"))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaSources": []})))
        .expect(1)
        .mount(&mock)
        .await;
    let options = PlaybackOptions {
        max_streaming_bitrate: Some(1_500_000),
        max_height: Some(480),
        ..PlaybackOptions::default()
    };
    session(&mock)
        .playback_info_with(&ItemId::from("abc"), &options)
        .await
        .unwrap();
}

#[tokio::test]
async fn original_quality_sends_no_height_limit() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/Items/abc/PlaybackInfo"))
        .and(body_json(playback_info_body()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaSources": []})))
        .expect(1)
        .mount(&mock)
        .await;
    let options = PlaybackOptions {
        max_height: None,
        ..PlaybackOptions::default()
    };
    session(&mock)
        .playback_info_with(&ItemId::from("abc"), &options)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_file_to_convert_is_asked_without_direct_play() {
    let mock = MockServer::start().await;
    let mut body = playback_info_body();
    body["EnableDirectPlay"] = json!(false);
    body["EnableDirectStream"] = json!(false);
    Mock::given(method("POST"))
        .and(path("/Items/abc/PlaybackInfo"))
        .and(body_json(body))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"MediaSources": []})))
        .expect(1)
        .mount(&mock)
        .await;
    let options = PlaybackOptions {
        transcode_only: true,
        ..PlaybackOptions::default()
    };
    session(&mock)
        .playback_info_with(&ItemId::from("abc"), &options)
        .await
        .unwrap();
}

/// A transcode answer, shaped like the one of Jellyfin 12.4 (camelCase).
fn transcode_answer() -> serde_json::Value {
    json!({
        "mediaSources": [{
            "id": "def",
            "supportsDirectPlay": false,
            "supportsDirectStream": false,
            "supportsTranscoding": true,
            "transcodingUrl": "/videos/abc/master.m3u8?DeviceId=test-device-id&MediaSourceId=def&AudioStreamIndex=2&PlaySessionId=ps1&ApiKey=test-token&TranscodeReasons=ContainerBitrateExceedsLimit",
            "transcodingSubProtocol": "hls",
            "mediaStreams": [{
                "index": 0,
                "type": "Subtitle",
                "codec": "subrip",
                "isExternal": true,
                "isTextSubtitleStream": true,
                "deliveryMethod": "External",
                "deliveryUrl": "/Videos/abc/def/Subtitles/0/0/Stream.subrip?ApiKey=test-token",
            }],
        }],
        "playSessionId": "ps1",
    })
}

#[tokio::test]
async fn transcode_answer_decodes_without_the_token() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/Items/abc/PlaybackInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(transcode_answer()))
        .mount(&mock)
        .await;
    let session = session(&mock);
    let info = session.playback_info(&ItemId::from("abc")).await.unwrap();
    let source = &info.media_sources[0];
    assert!(!source.supports_direct_play && source.supports_transcoding);
    assert_eq!(source.transcoding_sub_protocol.as_deref(), Some("hls"));
    assert_eq!(source.transcode_reasons(), ["ContainerBitrateExceedsLimit"]);
    assert_eq!(source.transcoding_audio_stream_index(), Some(2));
    assert_eq!(source.transcoding_subtitle_stream_index(), None);
    let url = source.transcoding_url.as_deref().unwrap();
    assert!(!url.contains(TEST_TOKEN), "{url}");
    let subtitle = &source.media_streams[0];
    assert_eq!(
        subtitle.delivery_method,
        Some(SubtitleDeliveryMethod::External)
    );
    assert!(subtitle.is_text_subtitle_stream);
    assert_eq!(
        subtitle.delivery_url.as_deref(),
        Some("/Videos/abc/def/Subtitles/0/0/Stream.subrip")
    );
    assert!(!format!("{info:?}").contains(TEST_TOKEN));

    let stream = session.transcode_source(url).unwrap();
    assert_eq!(
        stream.url.as_str(),
        format!(
            "{}/videos/abc/master.m3u8?DeviceId=test-device-id&MediaSourceId=def&AudioStreamIndex=2&PlaySessionId=ps1&TranscodeReasons=ContainerBitrateExceedsLimit",
            mock.uri()
        )
    );
    assert!(stream.authorization.ends_with(r#"Token="test-token""#));
}

#[test]
fn server_urls_keep_the_base_path() {
    let session = Server::new("https://h.example/jellyfin", common::client_info())
        .unwrap()
        .restore_session(TEST_TOKEN, TEST_USER_ID.into(), None);
    assert_eq!(
        session
            .server_url("/Videos/a/b/Subtitles/0/0/Stream.subrip")
            .unwrap()
            .as_str(),
        "https://h.example/jellyfin/Videos/a/b/Subtitles/0/0/Stream.subrip"
    );
    assert_eq!(
        session
            .server_url("videos/a/master.m3u8?x=1")
            .unwrap()
            .as_str(),
        "https://h.example/jellyfin/videos/a/master.m3u8?x=1"
    );
}

#[tokio::test]
async fn stop_transcoding_deletes_the_session_encoding() {
    let mock = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/Videos/ActiveEncodings"))
        .and(query_param("deviceId", "test-device-id"))
        .and(query_param("playSessionId", "ps1"))
        .and(authorization_is(Some(TEST_TOKEN)))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&mock)
        .await;
    session(&mock)
        .stop_transcoding(&PlaySessionId::from("ps1"))
        .await
        .unwrap();
}

#[tokio::test]
async fn reports_carry_the_transcode_method() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/Sessions/Playing/Progress"))
        .and(body_partial_json(
            json!({ "PlayMethod": "Transcode", "PlaySessionId": "ps1" }),
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&mock)
        .await;
    let transcoded = PlaybackReport {
        play_method: PlayMethod::Transcode,
        ..report()
    };
    session(&mock)
        .report_playback_progress(&transcoded)
        .await
        .unwrap();
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
async fn a_stream_is_probed_with_one_byte() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/Videos/abc/stream"))
        .and(query_param("static", "true"))
        .and(query_param("mediaSourceId", "def"))
        .and(header("Range", "bytes=0-0"))
        .and(authorization_is(Some(TEST_TOKEN)))
        .respond_with(ResponseTemplate::new(206).set_body_bytes(vec![0]))
        .expect(1)
        .mount(&mock)
        .await;
    session(&mock)
        .probe_stream(&"abc".into(), &MediaSourceId::from("def"))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_stream_the_server_cannot_serve_fails_its_probe() {
    for status in [401, 404, 410, 500] {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Videos/abc/stream"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&mock)
            .await;
        let error = session(&mock)
            .probe_stream(&"abc".into(), &MediaSourceId::from("def"))
            .await
            .unwrap_err();
        assert!(
            matches!(error, oxyfin_api::Error::Status { status: got, .. } if got == status),
            "{error:?}"
        );
    }
}

#[tokio::test]
async fn stream_source_debug_redacts() {
    let mock = MockServer::start().await;
    let source = session(&mock).stream_source(&"abc".into(), &MediaSourceId::from("def"));
    let debug = format!("{source:?}");
    assert!(!debug.contains(TEST_TOKEN), "{debug}");
    assert!(debug.contains("<redacted>"), "{debug}");
}

#[tokio::test]
async fn a_subtitle_burned_into_a_transcode_is_known() {
    let mut answer = transcode_answer();
    answer["mediaSources"][0]["transcodingUrl"] = json!(
        "/videos/abc/master.m3u8?MediaSourceId=def&AudioStreamIndex=1&SubtitleStreamIndex=3&SubtitleMethod=Encode&ApiKey=test-token"
    );
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/Items/abc/PlaybackInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(answer))
        .mount(&mock)
        .await;
    let info = session(&mock)
        .playback_info(&ItemId::from("abc"))
        .await
        .unwrap();
    assert_eq!(
        info.media_sources[0].transcoding_subtitle_stream_index(),
        Some(3)
    );
}
