//! End-to-end playback with the real mpv player and a mock server.
#![cfg(feature = "mpv")]

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::{bodies, fixture, session, wait_requests};
use oxyfin_api::ItemId;
use oxyfin_core::{
    PlayMode, PlaybackCommand, PlaybackController, PlaybackEvent, PlaybackTimings, player_events,
};
use oxyfin_player::{Player, PlayerOptions, VideoOutput};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const MOVIE_ID: &str = "76282b1c408f374beeee50519e9157a4";
const SOURCE_ID: &str = "5a3b1b5e0c8e4f6e9a1d2c3b4a5f6e7d";
const START: &str = "/Sessions/Playing";
const PROGRESS: &str = "/Sessions/Playing/Progress";
const STOPPED: &str = "/Sessions/Playing/Stopped";
/// mpv needs a few seconds to open and play the 3 s sample.
const LONG: Duration = Duration::from_secs(20);

fn player_fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../oxyfin-player/tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()))
}

/// PlaybackInfo of `sample.mkv` (video ff0, audio eng ff1 and fra ff2, internal
/// subtitle ff3) with `external.srt` listed first, so Jellyfin indexes are ff + 1.
fn playback_info() -> Value {
    json!({
        "MediaSources": [{
            "Id": SOURCE_ID,
            "Container": "mkv",
            "RunTimeTicks": 30_000_000,
            "SupportsDirectPlay": true,
            "MediaStreams": [
                { "Index": 0, "Type": "Subtitle", "Codec": "subrip", "Language": "eng",
                  "DisplayTitle": "English - SUBRIP - External", "IsExternal": true },
                { "Index": 1, "Type": "Video", "Codec": "mpeg4" },
                { "Index": 2, "Type": "Audio", "Codec": "flac", "Language": "eng",
                  "Title": "English", "Channels": 1, "IsDefault": true },
                { "Index": 3, "Type": "Audio", "Codec": "flac", "Language": "fra",
                  "Title": "French", "Channels": 1 },
                { "Index": 4, "Type": "Subtitle", "Codec": "subrip", "Language": "eng" }
            ],
            "DefaultAudioStreamIndex": 3,
            "DefaultSubtitleStreamIndex": 0
        }],
        "PlaySessionId": "e2e-session"
    })
}

/// Serves a file with HTTP range support, as Jellyfin does; mpv reads Matroska
/// files in ranges and stalls near the end when a range is ignored.
struct Ranged(Vec<u8>);

impl Respond for Ranged {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let len = self.0.len();
        let range = request
            .headers
            .get("range")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("bytes="))
            .and_then(|value| value.split_once('-'))
            .and_then(|(start, end)| {
                let start: usize = start.parse().ok()?;
                let end = end.parse::<usize>().map_or(len - 1, |end| end.min(len - 1));
                (start <= end).then_some((start, end))
            });
        let Some((start, end)) = range else {
            return ResponseTemplate::new(200)
                .insert_header("Accept-Ranges", "bytes")
                .insert_header("Content-Type", "video/x-matroska")
                .set_body_bytes(self.0.clone());
        };
        ResponseTemplate::new(206)
            .insert_header("Accept-Ranges", "bytes")
            .insert_header("Content-Type", "video/x-matroska")
            .insert_header(
                "Content-Range",
                format!("bytes {start}-{end}/{len}").as_str(),
            )
            .set_body_bytes(self.0[start..=end].to_vec())
    }
}

async fn mount(mock: &MockServer) {
    Mock::given(method("GET"))
        .and(path(format!("/Items/{MOVIE_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("item_movie")))
        .mount(mock)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/Items/{MOVIE_ID}/PlaybackInfo")))
        .respond_with(ResponseTemplate::new(200).set_body_json(playback_info()))
        .mount(mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/Videos/{MOVIE_ID}/stream")))
        .respond_with(Ranged(player_fixture("sample.mkv")))
        .mount(mock)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/Videos/{MOVIE_ID}/{SOURCE_ID}/Subtitles/0/Stream.srt"
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/plain")
                .set_body_bytes(player_fixture("external.srt")),
        )
        .mount(mock)
        .await;
    for route in [START, PROGRESS, STOPPED] {
        Mock::given(method("POST"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(204))
            .mount(mock)
            .await;
    }
}

/// Receives events until one matches `pred`, waiting up to [`LONG`]; returns them all.
async fn until(
    events: &mut UnboundedReceiver<PlaybackEvent>,
    pred: impl Fn(&PlaybackEvent) -> bool,
) -> Vec<PlaybackEvent> {
    let mut seen = Vec::new();
    let found = tokio::time::timeout(LONG, async {
        while let Some(event) = events.recv().await {
            let done = pred(&event);
            seen.push(event);
            if done {
                return true;
            }
        }
        false
    })
    .await;
    assert!(
        matches!(found, Ok(true)),
        "no matching event, received {seen:?}"
    );
    seen
}

#[tokio::test(flavor = "multi_thread")]
async fn real_player_reports_start() {
    let mock = MockServer::start().await;
    mount(&mock).await;
    let (sink, events) = player_events();
    let player = Player::new(
        PlayerOptions {
            video_output: VideoOutput::None,
        },
        move |event| sink.send(event),
    )
    .unwrap_or_else(|err| panic!("cannot create the player: {err}"));
    let (handle, mut events) = PlaybackController::spawn_with_timings(
        session(&mock),
        Arc::new(player),
        events,
        PlaybackTimings {
            progress_interval: Duration::from_secs(3600),
            ..PlaybackTimings::default()
        },
    );
    handle.send(PlaybackCommand::Play {
        item: ItemId::from(MOVIE_ID),
        mode: PlayMode::FromStart,
    });

    let seen = until(&mut events, |e| matches!(e, PlaybackEvent::Ready { .. })).await;
    assert!(
        !seen.iter().any(|e| matches!(e, PlaybackEvent::Error(_))),
        "{seen:?}"
    );
    let ready = seen.last().cloned();
    let Some(PlaybackEvent::Ready {
        audio, subtitles, ..
    }) = ready
    else {
        unreachable!();
    };
    assert!(
        audio.iter().any(|a| a.index == 3 && a.selected),
        "{audio:?}"
    );
    assert!(
        subtitles.iter().any(|s| s.index == 0 && s.selected),
        "{subtitles:?}"
    );

    let start = wait_requests(&mock, START, 1).await;
    assert_eq!(start[0]["AudioStreamIndex"], json!(3));
    assert_eq!(start[0]["SubtitleStreamIndex"], json!(0));
    assert_eq!(start[0]["PlaySessionId"], json!("e2e-session"));

    // The 3 s sample plays to its end with no user action: the only reports
    // are the start and the stop of the real end.
    let after = until(&mut events, |e| matches!(e, PlaybackEvent::Ended)).await;
    wait_requests(&mock, STOPPED, 1).await;
    let changes = after
        .iter()
        .filter(|e| matches!(e, PlaybackEvent::TracksChanged { .. }))
        .count();
    let progress = bodies(&mock, PROGRESS).await;
    assert!(progress.is_empty(), "{progress:?}");
    assert_eq!(changes, 0, "{after:?}");
    assert_eq!(bodies(&mock, START).await.len(), 1);
    handle.shutdown().await;
}
