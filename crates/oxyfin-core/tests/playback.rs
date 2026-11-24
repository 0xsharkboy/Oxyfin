//! Integration tests for the playback controller, with a fake player.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    Call, FakePlayer, TEST_TOKEN, bodies, events_until, fixture, session, wait_event, wait_requests,
};
use oxyfin_api::{ImageType, ItemId, MediaSourceId};
use oxyfin_core::{
    AudioPick, Chapter, ImageRef, PROGRESS_INTERVAL, Paths, PlayMethod, PlayMode, PlaybackCommand,
    PlaybackController, PlaybackError, PlaybackEvent, PlaybackHandle, PlaybackPrefs, PlaybackStats,
    PlaybackTimings, PlayerEventSink, QUALITIES, Quality, SHUTDOWN_TIMEOUT, SegmentKind,
    SkippableSegment, SubtitleBackground, SubtitlePick, SubtitleSize, SubtitleStyle, TrackChoice,
    TrackMemory, Trickplay, UNLOAD_GRACE, chapter_at, next_chapter, player_events,
    previous_chapter, start_at,
};
use oxyfin_player::{PlayerEvent, SeekTarget, Track, TrackId, TrackKind, UnloadReason};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::mpsc::UnboundedReceiver;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MOVIE_ID: &str = "76282b1c408f374beeee50519e9157a4";
/// A second item, served with the same fixtures.
const OTHER_ID: &str = "0000000000000000000000000000000b";
/// Series of `shows_episodes.json`.
const SERIES_ID: &str = "9ea196e7ccc3ae17d2c3f75c3ab34406";
/// Episodes of `shows_episodes.json`, in server order: S01E01, S01E02, S02E01, S02E02.
const EPISODES: [&str; 4] = [
    "0366bed6b6c97d8af5be151ffc258992",
    "50d3822a34079d70485f1bf36c274c35",
    "6cd987e31b59eee077807423aecbcd8a",
    "6aeeb019134a3bf62f0f69ab03349ab3",
];
const PLAY_SESSION_ID: &str = "8050f69acbe34027b28b38f406d4fe49";
/// Account whose track choices the tests remember.
const ACCOUNT: &str = "user@http://jellyfin.test";
const START: &str = "/Sessions/Playing";
const PROGRESS: &str = "/Sessions/Playing/Progress";
const STOPPED: &str = "/Sessions/Playing/Stopped";
/// `UserData.PlaybackPositionTicks` of `item_movie.json`.
const RESUME: Duration = Duration::from_secs(3);

struct Harness {
    mock: MockServer,
    player: Arc<FakePlayer>,
    sink: PlayerEventSink,
    handle: PlaybackHandle,
    events: UnboundedReceiver<PlaybackEvent>,
    /// Holds the track memory file.
    data: TempDir,
}

/// Timings with no periodic report, so only explicit triggers send one.
fn quiet() -> PlaybackTimings {
    PlaybackTimings {
        progress_interval: Duration::from_secs(3600),
        ..PlaybackTimings::default()
    }
}

async fn mount_status(mock: &MockServer, verb: &str, route: &str, status: u16) {
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(ResponseTemplate::new(status))
        .with_priority(1)
        .mount(mock)
        .await;
}

async fn mount_playback_info(mock: &MockServer, body: Value) {
    Mock::given(method("POST"))
        .and(path(format!("/Items/{MOVIE_ID}/PlaybackInfo")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .with_priority(1)
        .mount(mock)
        .await;
}

/// Spawns a controller on `mock`, after mounting the default routes (lowest priority).
async fn harness_on(mock: MockServer, timings: PlaybackTimings) -> Harness {
    let data = tempfile::tempdir().unwrap_or_else(|err| panic!("no temporary directory: {err}"));
    harness_in(mock, timings, data).await
}

/// Like [`harness_on`], with the track memory stored in `data`.
async fn harness_in(mock: MockServer, timings: PlaybackTimings, data: TempDir) -> Harness {
    for id in [MOVIE_ID, OTHER_ID] {
        Mock::given(method("GET"))
            .and(path(format!("/Items/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("item_movie")))
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/Items/{id}/PlaybackInfo")))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("playback_info")))
            .mount(&mock)
            .await;
    }
    for id in EPISODES {
        Mock::given(method("GET"))
            .and(path(format!("/Items/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("item_episode")))
            .mount(&mock)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/Items/{id}/PlaybackInfo")))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("playback_info")))
            .mount(&mock)
            .await;
    }
    Mock::given(method("GET"))
        .and(path(format!("/Shows/{SERIES_ID}/Episodes")))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("shows_episodes")))
        .mount(&mock)
        .await;
    for route in [START, PROGRESS, STOPPED] {
        Mock::given(method("POST"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(204))
            .mount(&mock)
            .await;
    }
    let player = Arc::new(FakePlayer::default());
    let (sink, player_events) = player_events();
    let (handle, events) = PlaybackController::spawn_with_timings(
        session(&mock),
        player.clone(),
        player_events,
        timings,
        TrackMemory::load(&Paths::under(data.path()), ACCOUNT),
    );
    Harness {
        mock,
        player,
        sink,
        handle,
        events,
        data,
    }
}

async fn harness(timings: PlaybackTimings) -> Harness {
    harness_on(MockServer::start().await, timings).await
}

fn track(id: i64, kind: TrackKind, ff: i64, selected: bool) -> Track {
    Track {
        id: TrackId(id),
        kind,
        language: None,
        title: None,
        codec: None,
        channels: None,
        is_default: false,
        is_forced: false,
        is_external: false,
        is_selected: selected,
        stream_index: Some(ff),
        external_url: None,
    }
}

/// The tracks mpv reports for the fixture file: video 0, audio 1 (eng) and 2 (fra).
fn file_tracks() -> Vec<Track> {
    vec![
        track(1, TrackKind::Video, 0, true),
        track(1, TrackKind::Audio, 1, true),
        track(2, TrackKind::Audio, 2, false),
    ]
}

impl Harness {
    async fn play(&mut self, mode: PlayMode) {
        self.play_item(MOVIE_ID, mode).await;
    }

    /// Plays `id` and waits until the player receives its `load`.
    async fn play_item(&mut self, id: &str, mode: PlayMode) {
        let loads = self.loads().len();
        self.handle.send(PlaybackCommand::Play {
            item: ItemId::from(id),
            mode,
        });
        wait_event(&mut self.events, |e| {
            matches!(e, PlaybackEvent::Loading { .. })
        })
        .await;
        let start = tokio::time::Instant::now();
        while self.loads().len() <= loads {
            assert!(start.elapsed() < common::DEADLINE, "no load for {id}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn loads(&self) -> Vec<oxyfin_player::MediaSource> {
        self.player
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Load(source) => Some(source),
                _ => None,
            })
            .collect()
    }

    /// Sends `FileLoaded` with the fixture tracks and waits for `Ready`.
    async fn file_loaded(&mut self) -> PlaybackEvent {
        let mut seen = self.file_loaded_events().await;
        seen.pop().unwrap_or_else(|| panic!("no Ready event"))
    }

    /// Sends `FileLoaded` and returns every event received up to `Ready`.
    async fn file_loaded_events(&mut self) -> Vec<PlaybackEvent> {
        self.sink.send(PlayerEvent::FileLoaded {
            duration: Some(Duration::from_millis(10_023)),
            tracks: file_tracks(),
        });
        events_until(&mut self.events, |e| {
            matches!(e, PlaybackEvent::Ready { .. })
        })
        .await
    }

    /// Plays the movie in auto mode and loads it; returns the `Ready` event.
    async fn start(&mut self) -> PlaybackEvent {
        self.play(PlayMode::Auto).await;
        let ready = self.file_loaded().await;
        wait_requests(&self.mock, START, 1).await;
        ready
    }

    /// Like [`Harness::stop`] for an item that already reported its stop (real
    /// end): no new stop report is awaited.
    async fn stop_after_end(&mut self) -> Vec<PlaybackEvent> {
        let marker = PlaybackEvent::Position(Duration::from_millis(999));
        self.sink
            .send(PlayerEvent::Position(Duration::from_millis(999)));
        let mut seen = events_until(&mut self.events, |e| *e == marker).await;
        self.handle.send(PlaybackCommand::Stop);
        seen.extend(events_until(&mut self.events, |e| matches!(e, PlaybackEvent::Stopped)).await);
        seen
    }

    /// Lets the controller handle every player event sent so far, then stops
    /// playback; returns every event received up to `Stopped`.
    ///
    /// Player events and commands travel on separate channels, hence the
    /// marker event before the command. Reports are sent in order, so once the
    /// stop report arrives every earlier report has arrived too.
    async fn stop(&mut self) -> Vec<PlaybackEvent> {
        let stopped = bodies(&self.mock, STOPPED).await.len();
        let marker = PlaybackEvent::Position(Duration::from_millis(999));
        self.sink
            .send(PlayerEvent::Position(Duration::from_millis(999)));
        let mut seen = events_until(&mut self.events, |e| *e == marker).await;
        self.handle.send(PlaybackCommand::Stop);
        seen.extend(events_until(&mut self.events, |e| matches!(e, PlaybackEvent::Stopped)).await);
        wait_requests(&self.mock, STOPPED, stopped + 1).await;
        seen
    }
}

fn load_call(player: &FakePlayer) -> oxyfin_player::MediaSource {
    player
        .calls()
        .into_iter()
        .find_map(|call| match call {
            Call::Load(source) => Some(source),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no load call"))
}

fn position_of(seen: &[Call], wanted: &Call) -> usize {
    seen.iter()
        .position(|call| call == wanted)
        .unwrap_or_else(|| panic!("{wanted:?} not in {seen:?}"))
}

#[test]
fn default_timings_follow_the_spec() {
    let timings = PlaybackTimings::default();
    assert_eq!(timings.progress_interval, PROGRESS_INTERVAL);
    assert_eq!(timings.progress_interval, Duration::from_secs(10));
    assert_eq!(timings.next_up_countdown_tick, Duration::from_secs(1));
    assert_eq!(timings.shutdown_timeout, SHUTDOWN_TIMEOUT);
    assert_eq!(timings.shutdown_timeout, Duration::from_secs(2));
    assert_eq!(timings.unload_grace, UNLOAD_GRACE);
    assert_eq!(timings.unload_grace, Duration::from_millis(500));
}

#[tokio::test]
async fn play_auto_resumes_at_saved_position() {
    let mut h = harness(quiet()).await;
    h.play(PlayMode::Auto).await;
    let source = load_call(&h.player);
    assert_eq!(source.start, Some(RESUME));
    assert!(
        source
            .location
            .contains(&format!("/Videos/{MOVIE_ID}/stream")),
        "{}",
        source.location
    );
    let auth = source
        .headers
        .iter()
        .find(|(name, _)| name == "Authorization")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    assert!(auth.contains(TEST_TOKEN));
}

#[tokio::test]
async fn play_from_start_has_no_start() {
    let mut h = harness(quiet()).await;
    h.play(PlayMode::FromStart).await;
    assert_eq!(load_call(&h.player).start, None);
}

#[tokio::test]
async fn loading_event_carries_the_title() {
    let mut h = harness(quiet()).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(MOVIE_ID),
        mode: PlayMode::Auto,
    });
    let event = wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::Loading { .. })
    })
    .await;
    let PlaybackEvent::Loading { item, title } = event else {
        unreachable!();
    };
    assert_eq!(item, ItemId::from(MOVIE_ID));
    assert_eq!(title.title, "Test Movie (2020)");
    assert_eq!(title.series_name, None);
    assert_eq!(title.episode_label, None);
}

#[tokio::test]
async fn loaded_file_applies_default_tracks() {
    let mut h = harness(quiet()).await;
    let ready = h.start().await;
    let calls = h.player.calls();
    let add = calls
        .iter()
        .position(|call| matches!(call, Call::AddExternal { .. }))
        .unwrap_or_else(|| panic!("no external subtitle added: {calls:?}"));
    let Call::AddExternal { url, language, .. } = &calls[add] else {
        unreachable!();
    };
    assert!(
        url.ends_with(&format!(
            "/Videos/{MOVIE_ID}/{MOVIE_ID}/Subtitles/0/Stream.srt"
        )),
        "{url}"
    );
    assert_eq!(language.as_deref(), Some("eng"));
    let audio = position_of(&calls, &Call::SelectAudio(Some(TrackId(1))));
    let subtitle = position_of(&calls, &Call::SelectSubtitle(Some(TrackId(1))));
    assert!(add < subtitle && add < audio);
    position_of(&calls, &Call::SetSpeed(1.0));

    let PlaybackEvent::Ready {
        duration,
        audio,
        subtitles,
        has_previous,
        has_next,
    } = ready
    else {
        unreachable!();
    };
    assert_eq!(duration, Some(Duration::from_millis(10_023)));
    let selected_audio: Vec<i32> = audio
        .iter()
        .filter(|a| a.selected)
        .map(|a| a.index)
        .collect();
    assert_eq!(selected_audio, vec![2]);
    assert_eq!(audio.len(), 2);
    assert_eq!(subtitles.len(), 1);
    assert!(subtitles[0].selected && subtitles[0].external);
    assert!(has_previous && !has_next);
}

#[tokio::test]
async fn missing_default_subtitle_turns_subtitles_off() {
    let mock = MockServer::start().await;
    let mut info = fixture("playback_info");
    let source = &mut info["MediaSources"][0];
    let Some(source) = source.as_object_mut() else {
        panic!("bad fixture");
    };
    source.remove("DefaultSubtitleStreamIndex");
    source.remove("DefaultAudioStreamIndex");
    mount_playback_info(&mock, info).await;
    let mut h = harness_on(mock, quiet()).await;
    h.start().await;
    let calls = h.player.calls();
    position_of(&calls, &Call::SelectSubtitle(None));
    assert!(
        !calls.iter().any(|c| matches!(c, Call::SelectAudio(_))),
        "{calls:?}"
    );
    let start = wait_requests(&h.mock, START, 1).await;
    assert_eq!(start[0]["AudioStreamIndex"], json!(2));
    assert_eq!(start[0].get("SubtitleStreamIndex"), None);
}

#[tokio::test]
async fn start_report_has_jellyfin_indexes() {
    let mut h = harness(quiet()).await;
    h.start().await;
    let start = wait_requests(&h.mock, START, 1).await;
    let body = &start[0];
    assert_eq!(body["ItemId"], json!(MOVIE_ID));
    assert_eq!(body["MediaSourceId"], json!(MOVIE_ID));
    assert_eq!(body["PlaySessionId"], json!(PLAY_SESSION_ID));
    assert_eq!(body["AudioStreamIndex"], json!(2));
    assert_eq!(body["SubtitleStreamIndex"], json!(0));
    assert_eq!(body["PositionTicks"], json!(30_000_000));
    assert_eq!(body["IsPaused"], json!(false));
}

#[tokio::test]
async fn progress_reports_on_each_interval_until_paused() {
    let mut h = harness(PlaybackTimings {
        progress_interval: Duration::from_millis(30),
        ..PlaybackTimings::default()
    })
    .await;
    h.start().await;
    h.sink.send(PlayerEvent::Position(Duration::from_secs(4)));
    let periodic = wait_requests(&h.mock, PROGRESS, 3).await;
    assert!(periodic.iter().all(|body| body["IsPaused"] == json!(false)));

    h.sink.send(PlayerEvent::Paused(true));
    let start = tokio::time::Instant::now();
    let after_pause = loop {
        let found = bodies(&h.mock, PROGRESS).await;
        if found
            .last()
            .is_some_and(|body| body["IsPaused"] == json!(true))
        {
            break found.len();
        }
        assert!(start.elapsed() < common::DEADLINE, "no paused report");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(bodies(&h.mock, PROGRESS).await.len(), after_pause);
}

#[tokio::test]
async fn pause_reports_immediately() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.sink.send(PlayerEvent::Position(Duration::from_secs(5)));
    h.sink.send(PlayerEvent::Paused(true));
    wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Paused(true))).await;
    let progress = wait_requests(&h.mock, PROGRESS, 1).await;
    assert_eq!(progress[0]["IsPaused"], json!(true));
    assert_eq!(progress[0]["PositionTicks"], json!(50_000_000));
}

#[tokio::test]
async fn renderer_seek_pair_is_silent() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.sink.send(PlayerEvent::Seeking);
    h.sink.send(PlayerEvent::Seeked);
    let seen = h.stop().await;
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, PlaybackEvent::Seeking | PlaybackEvent::Seeked)),
        "{seen:?}"
    );
    assert!(bodies(&h.mock, PROGRESS).await.is_empty());
}

#[tokio::test]
async fn user_seek_reports_on_seeked() {
    let mut h = harness(quiet()).await;
    h.start().await;
    let target = SeekTarget::Forward(Duration::from_secs(10));
    h.handle.send(PlaybackCommand::Seek(target));
    h.player.wait_call(|c| *c == Call::Seek(target)).await;
    h.sink.send(PlayerEvent::Seeking);
    h.sink.send(PlayerEvent::Position(Duration::from_secs(13)));
    h.sink.send(PlayerEvent::Seeked);
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;
    assert!(seen.iter().any(|e| matches!(e, PlaybackEvent::Seeking)));
    let progress = wait_requests(&h.mock, PROGRESS, 1).await;
    assert_eq!(progress[0]["PositionTicks"], json!(130_000_000));

    h.sink.send(PlayerEvent::Seeking);
    h.sink.send(PlayerEvent::Seeked);
    let seen = h.stop().await;
    assert!(
        !seen.iter().any(|e| matches!(e, PlaybackEvent::Seeked)),
        "{seen:?}"
    );
    assert_eq!(bodies(&h.mock, PROGRESS).await.len(), 1);
}

#[tokio::test]
async fn command_while_loading_is_ignored() {
    let mut h = harness(quiet()).await;
    h.player.set_no_media(true);
    h.play(PlayMode::Auto).await;
    let target = SeekTarget::Absolute(Duration::from_secs(1));
    h.handle.send(PlaybackCommand::SetPaused(true));
    h.handle.send(PlaybackCommand::Seek(target));
    h.player.wait_call(|c| *c == Call::Seek(target)).await;
    h.player.set_no_media(false);

    h.sink.send(PlayerEvent::FileLoaded {
        duration: Some(Duration::from_millis(10_023)),
        tracks: file_tracks(),
    });
    h.sink.send(PlayerEvent::Seeking);
    h.sink.send(PlayerEvent::Seeked);
    let mut seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Ready { .. })).await;
    seen.extend(h.stop().await);
    assert!(
        !seen.iter().any(|e| matches!(
            e,
            PlaybackEvent::Error(_) | PlaybackEvent::Seeking | PlaybackEvent::Seeked
        )),
        "{seen:?}"
    );
    assert!(bodies(&h.mock, PROGRESS).await.is_empty());
    wait_requests(&h.mock, STOPPED, 1).await;
}

#[tokio::test]
async fn failed_progress_report_keeps_playing() {
    let mock = MockServer::start().await;
    mount_status(&mock, "POST", PROGRESS, 500).await;
    let mut h = harness_on(mock, quiet()).await;
    h.start().await;
    h.sink.send(PlayerEvent::Paused(true));
    wait_requests(&h.mock, PROGRESS, 1).await;
    h.sink.send(PlayerEvent::Position(Duration::from_secs(5)));
    let seen = events_until(&mut h.events, |e| {
        *e == PlaybackEvent::Position(Duration::from_secs(5))
    })
    .await;
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, PlaybackEvent::Error(_) | PlaybackEvent::SessionExpired)),
        "{seen:?}"
    );
    assert!(!h.player.calls().contains(&Call::Stop));
    h.stop().await;
    wait_requests(&h.mock, STOPPED, 1).await;
}

#[tokio::test]
async fn progress_401_is_session_expired() {
    let mock = MockServer::start().await;
    mount_status(&mock, "POST", PROGRESS, 401).await;
    let mut h = harness_on(mock, quiet()).await;
    h.start().await;
    h.sink.send(PlayerEvent::Paused(true));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SessionExpired)
    })
    .await;
    assert!(h.player.calls().contains(&Call::Stop));
    assert!(bodies(&h.mock, STOPPED).await.is_empty());
}

#[tokio::test]
async fn item_401_is_session_expired() {
    let mock = MockServer::start().await;
    mount_status(&mock, "GET", &format!("/Items/{MOVIE_ID}"), 401).await;
    let mut h = harness_on(mock, quiet()).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(MOVIE_ID),
        mode: PlayMode::Auto,
    });
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SessionExpired)
    })
    .await;
    assert!(!h.player.calls().iter().any(|c| matches!(c, Call::Load(_))));
}

#[tokio::test]
async fn no_playable_source_is_an_error() {
    let mock = MockServer::start().await;
    let mut info = fixture("playback_info");
    info["MediaSources"][0]["SupportsDirectPlay"] = json!(false);
    mount_playback_info(&mock, info).await;
    let mut h = harness_on(mock, quiet()).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(MOVIE_ID),
        mode: PlayMode::Auto,
    });
    wait_event(&mut h.events, |e| {
        *e == PlaybackEvent::Error(PlaybackError::NoPlayableSource)
    })
    .await;
    assert!(!h.player.calls().iter().any(|c| matches!(c, Call::Load(_))));

    let mock = MockServer::start().await;
    mount_playback_info(&mock, json!({ "MediaSources": [] })).await;
    let mut h = harness_on(mock, quiet()).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(MOVIE_ID),
        mode: PlayMode::Auto,
    });
    wait_event(&mut h.events, |e| {
        *e == PlaybackEvent::Error(PlaybackError::NoPlayableSource)
    })
    .await;
    assert!(h.player.calls().is_empty());
}

#[tokio::test]
async fn load_error_is_reported() {
    let mut h = harness(quiet()).await;
    h.player.set_fail_load(true);
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(MOVIE_ID),
        mode: PlayMode::Auto,
    });
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::Error(PlaybackError::LoadFailed { .. }))
    })
    .await;
    assert!(bodies(&h.mock, START).await.is_empty());
}

#[tokio::test]
async fn playback_error_is_reported_with_a_stop_report() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.sink.send(PlayerEvent::Unloaded(UnloadReason::Error(
        "loading failed".into(),
    )));
    wait_event(&mut h.events, |e| {
        *e == PlaybackEvent::Error(PlaybackError::LoadFailed {
            detail: "loading failed".into(),
        })
    })
    .await;
    wait_requests(&h.mock, STOPPED, 1).await;
}

#[tokio::test]
async fn stop_reports_then_emits_stopped() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.sink.send(PlayerEvent::Position(Duration::from_secs(7)));
    wait_event(&mut h.events, |e| {
        *e == PlaybackEvent::Position(Duration::from_secs(7))
    })
    .await;
    h.handle.send(PlaybackCommand::Stop);
    wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Stopped)).await;
    let stopped = wait_requests(&h.mock, STOPPED, 1).await;
    assert_eq!(stopped[0]["PositionTicks"], json!(70_000_000));
    assert!(h.player.calls().contains(&Call::Stop));
}

#[tokio::test]
async fn select_commands_use_mpv_ids() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.handle.send(PlaybackCommand::SelectAudio(3));
    h.player
        .wait_call(|c| *c == Call::SelectAudio(Some(TrackId(2))))
        .await;
    h.handle.send(PlaybackCommand::SelectSubtitle(None));
    h.player
        .wait_call(|c| *c == Call::SelectSubtitle(None))
        .await;
}

#[tokio::test]
async fn track_change_reports_once() {
    let mut h = harness(quiet()).await;
    h.start().await;
    let mut external = track(1, TrackKind::Subtitle, 0, true);
    external.is_external = true;
    external.stream_index = None;
    let tracks = vec![
        track(1, TrackKind::Video, 0, true),
        track(1, TrackKind::Audio, 1, false),
        track(2, TrackKind::Audio, 2, true),
        external,
    ];
    let mut confirmed = tracks.clone();
    confirmed[1].is_selected = true;
    confirmed[2].is_selected = false;
    h.sink.send(PlayerEvent::Tracks(confirmed));
    h.sink.send(PlayerEvent::Tracks(tracks.clone()));
    let event = wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::TracksChanged { .. })
    })
    .await;
    let PlaybackEvent::TracksChanged { audio, .. } = event else {
        unreachable!();
    };
    assert!(audio.iter().any(|a| a.index == 3 && a.selected));
    let progress = wait_requests(&h.mock, PROGRESS, 1).await;
    assert_eq!(progress[0]["AudioStreamIndex"], json!(3));
    assert_eq!(progress[0]["SubtitleStreamIndex"], json!(0));

    h.sink.send(PlayerEvent::Tracks(tracks));
    let seen = h.stop().await;
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, PlaybackEvent::TracksChanged { .. })),
        "{seen:?}"
    );
    assert_eq!(bodies(&h.mock, PROGRESS).await.len(), 1);
}

#[tokio::test]
async fn player_state_events_are_forwarded() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.sink.send(PlayerEvent::Volume(42.0));
    h.sink.send(PlayerEvent::Muted(true));
    h.sink.send(PlayerEvent::Speed(1.5));
    let seen = events_until(&mut h.events, |e| *e == PlaybackEvent::Speed(1.5)).await;
    assert!(seen.contains(&PlaybackEvent::Volume(42.0)));
    assert!(seen.contains(&PlaybackEvent::Muted(true)));
    h.handle.send(PlaybackCommand::SetSpeed(2.0));
    h.handle.send(PlaybackCommand::SetVolume(50.0));
    h.handle.send(PlaybackCommand::SetMuted(false));
    h.handle.send(PlaybackCommand::SetPaused(true));
    h.player.wait_call(|c| *c == Call::SetPaused(true)).await;
    let calls = h.player.calls();
    assert!(calls.contains(&Call::SetSpeed(2.0)));
    assert!(calls.contains(&Call::SetVolume(50.0)));
    assert!(calls.contains(&Call::SetMuted(false)));
}

/// Asserts the `Ready` event of item B, started from the beginning, and its start report.
async fn assert_other_started(h: &mut Harness, seen: &[PlaybackEvent]) {
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, PlaybackEvent::Error(_) | PlaybackEvent::SessionExpired)),
        "{seen:?}"
    );
    let starts = wait_requests(&h.mock, START, 2).await;
    let body = &starts[1];
    assert_eq!(body["ItemId"], json!(OTHER_ID));
    assert_eq!(body["PositionTicks"], json!(0));
    assert_eq!(body["AudioStreamIndex"], json!(2));
    assert_eq!(body["SubtitleStreamIndex"], json!(0));
    let added: Vec<String> = h
        .player
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::AddExternal { url, .. } => Some(url),
            _ => None,
        })
        .collect();
    assert_eq!(added.len(), 2, "{added:?}");
    assert!(
        added[1].contains(&format!("/Videos/{OTHER_ID}/")),
        "{added:?}"
    );
}

#[tokio::test]
async fn switching_items_drops_stale_events() {
    let mut h = harness(PlaybackTimings {
        unload_grace: Duration::from_secs(5),
        ..quiet()
    })
    .await;
    h.start().await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(OTHER_ID),
        mode: PlayMode::FromStart,
    });
    h.player.wait_call(|c| *c == Call::Stop).await;
    h.sink.send(PlayerEvent::Paused(true));
    h.sink.send(PlayerEvent::Position(Duration::from_secs(50)));
    let mut stale = file_tracks();
    stale[1].is_selected = false;
    stale[2].is_selected = true;
    h.sink.send(PlayerEvent::Tracks(stale));
    h.sink.send(PlayerEvent::Unloaded(UnloadReason::Stopped));
    let start = tokio::time::Instant::now();
    while h.loads().len() < 2 {
        assert!(start.elapsed() < common::DEADLINE, "B never loaded");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(h.loads()[1].start, None);
    let mut seen = events_until(&mut h.events, |e| {
        matches!(e, PlaybackEvent::Loading { .. })
    })
    .await;
    seen.extend(h.file_loaded_events().await);
    assert!(
        !seen.iter().any(
            |e| matches!(e, PlaybackEvent::Position(p) if *p == Duration::from_secs(50))
                || *e == PlaybackEvent::Paused(true)
        ),
        "{seen:?}"
    );
    assert_other_started(&mut h, &seen).await;
    let stopped = wait_requests(&h.mock, STOPPED, 1).await;
    assert_eq!(stopped[0]["ItemId"], json!(MOVIE_ID));
    assert!(bodies(&h.mock, PROGRESS).await.is_empty());
}

#[tokio::test]
async fn next_item_does_not_inherit_the_pause() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.handle.send(PlaybackCommand::SetPaused(true));
    h.player.wait_call(|c| *c == Call::SetPaused(true)).await;
    assert!(h.player.state().paused);
    h.sink.send(PlayerEvent::Paused(true));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::Paused(true)).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(OTHER_ID),
        mode: PlayMode::FromStart,
    });
    h.player.wait_call(|c| *c == Call::Stop).await;
    h.sink.send(PlayerEvent::Unloaded(UnloadReason::Stopped));
    wait_loads(&h, 2).await;
    h.file_loaded().await;
    let starts = wait_requests(&h.mock, START, 2).await;
    assert_eq!(starts[1]["ItemId"], json!(OTHER_ID));
    assert_eq!(starts[1]["IsPaused"], json!(false));
    let calls = h.player.calls();
    let second_load = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| matches!(call, Call::Load(_)))
        .nth(1)
        .map(|(index, _)| index)
        .unwrap_or_else(|| panic!("no second load in {calls:?}"));
    assert!(
        calls[second_load..].contains(&Call::SetPaused(false)),
        "{calls:?}"
    );
    assert!(!h.player.state().paused);
}

#[tokio::test]
async fn switching_items_loads_after_the_grace_without_unload() {
    let mut h = harness(PlaybackTimings {
        unload_grace: Duration::from_millis(50),
        ..quiet()
    })
    .await;
    h.play(PlayMode::Auto).await;
    h.play_item(OTHER_ID, PlayMode::FromStart).await;
    assert!(h.player.calls().contains(&Call::Stop));
    let ready = h.file_loaded().await;
    assert_other_started_single(&h, &ready).await;
}

/// Like [`assert_other_started`] when item A never loaded (one start report only).
async fn assert_other_started_single(h: &Harness, ready: &PlaybackEvent) {
    assert!(matches!(ready, PlaybackEvent::Ready { .. }));
    let starts = wait_requests(&h.mock, START, 1).await;
    assert_eq!(starts[0]["ItemId"], json!(OTHER_ID));
    assert_eq!(starts[0]["PositionTicks"], json!(0));
}

#[tokio::test]
async fn stale_unload_error_does_not_stop_the_next_item() {
    let mut h = harness(PlaybackTimings {
        unload_grace: Duration::from_secs(5),
        ..quiet()
    })
    .await;
    h.start().await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(OTHER_ID),
        mode: PlayMode::FromStart,
    });
    h.player.wait_call(|c| *c == Call::Stop).await;
    h.sink
        .send(PlayerEvent::Unloaded(UnloadReason::Error("stale".into())));
    let mut seen = events_until(&mut h.events, |e| {
        matches!(e, PlaybackEvent::Loading { .. })
    })
    .await;
    let start = tokio::time::Instant::now();
    while h.loads().len() < 2 {
        assert!(start.elapsed() < common::DEADLINE, "B never loaded");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    seen.extend(h.file_loaded_events().await);
    seen.extend(h.stop().await);
    assert_other_started(&mut h, &seen).await;
}

#[tokio::test]
async fn failed_play_stops_the_previous_media() {
    let mock = MockServer::start().await;
    mount_status(
        &mock,
        "POST",
        &format!("/Items/{OTHER_ID}/PlaybackInfo"),
        500,
    )
    .await;
    let mut h = harness_on(mock, quiet()).await;
    h.start().await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(OTHER_ID),
        mode: PlayMode::Auto,
    });
    wait_event(&mut h.events, |e| {
        *e == PlaybackEvent::Error(PlaybackError::Server { status: 500 })
    })
    .await;
    assert!(h.player.calls().contains(&Call::Stop));
    assert_eq!(h.loads().len(), 1);
    wait_requests(&h.mock, STOPPED, 1).await;
}

#[tokio::test]
async fn slow_report_does_not_block_commands() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(PROGRESS))
        .respond_with(ResponseTemplate::new(204).set_delay(Duration::from_secs(5)))
        .with_priority(1)
        .mount(&mock)
        .await;
    let mut h = harness_on(mock, quiet()).await;
    h.start().await;
    h.sink.send(PlayerEvent::Paused(true));
    wait_requests(&h.mock, PROGRESS, 1).await;
    let sent = tokio::time::Instant::now();
    h.handle.send(PlaybackCommand::SetPaused(false));
    h.player.wait_call(|c| *c == Call::SetPaused(false)).await;
    assert!(
        sent.elapsed() < Duration::from_millis(500),
        "{:?}",
        sent.elapsed()
    );
}

#[tokio::test]
async fn no_stop_report_before_the_start_report() {
    let mut h = harness(PlaybackTimings {
        unload_grace: Duration::from_millis(50),
        ..quiet()
    })
    .await;
    h.play(PlayMode::Auto).await;
    h.handle.send(PlaybackCommand::Stop);
    wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Stopped)).await;

    h.play(PlayMode::Auto).await;
    h.sink
        .send(PlayerEvent::Unloaded(UnloadReason::Error("broken".into())));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::Error(PlaybackError::LoadFailed { .. }))
    })
    .await;

    // Reports are sent in order: once this start report arrives, any earlier
    // stop report would have arrived too.
    h.start().await;
    assert!(bodies(&h.mock, STOPPED).await.is_empty());
}

#[tokio::test]
async fn start_report_401_is_session_expired() {
    let mock = MockServer::start().await;
    mount_status(&mock, "POST", START, 401).await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::Auto).await;
    h.sink.send(PlayerEvent::FileLoaded {
        duration: None,
        tracks: file_tracks(),
    });
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SessionExpired)
    })
    .await;
    assert!(h.player.calls().contains(&Call::Stop));
    assert!(bodies(&h.mock, STOPPED).await.is_empty());
}

#[tokio::test]
async fn global_player_state_passes_the_stale_gate() {
    let mut h = harness(PlaybackTimings {
        unload_grace: Duration::from_secs(5),
        ..quiet()
    })
    .await;
    h.start().await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(OTHER_ID),
        mode: PlayMode::FromStart,
    });
    h.player.wait_call(|c| *c == Call::Stop).await;
    h.sink.send(PlayerEvent::Volume(30.0));
    h.sink.send(PlayerEvent::Muted(true));
    h.sink.send(PlayerEvent::Speed(1.25));
    h.sink.send(PlayerEvent::Unloaded(UnloadReason::Stopped));
    let seen = events_until(&mut h.events, |e| *e == PlaybackEvent::Speed(1.25)).await;
    assert!(seen.contains(&PlaybackEvent::Volume(30.0)), "{seen:?}");
    assert!(seen.contains(&PlaybackEvent::Muted(true)), "{seen:?}");
}

#[tokio::test]
async fn session_expired_is_emitted_once() {
    let mock = MockServer::start().await;
    mount_status(&mock, "POST", PROGRESS, 401).await;
    mount_status(&mock, "GET", &format!("/Items/{OTHER_ID}"), 401).await;
    let mut h = harness_on(mock, quiet()).await;
    h.start().await;
    h.sink.send(PlayerEvent::Paused(true));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SessionExpired)
    })
    .await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(OTHER_ID),
        mode: PlayMode::Auto,
    });
    wait_requests(&h.mock, &format!("/Items/{OTHER_ID}"), 1).await;
    // Long enough for the 401 to come back and be handled.
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(Duration::from_millis(500), async {
        while let Some(event) = h.events.recv().await {
            seen.push(event);
        }
    })
    .await;
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, PlaybackEvent::SessionExpired)),
        "{seen:?}"
    );
    let item_requests = h
        .mock
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path() == format!("/Items/{OTHER_ID}"))
        .count();
    assert_eq!(item_requests, 1);
}

/// Timings for the episode tests: a countdown step of `tick`, a short unload grace.
fn episode_timings(tick: Duration) -> PlaybackTimings {
    PlaybackTimings {
        next_up_countdown_tick: tick,
        unload_grace: Duration::from_millis(30),
        ..quiet()
    }
}

const TEN_MINUTES: Duration = Duration::from_secs(600);

impl Harness {
    /// Plays `id`, loads it with `duration` and waits for its start report; returns `Ready`.
    async fn start_item(&mut self, id: &str, duration: Duration) -> PlaybackEvent {
        let starts = bodies(&self.mock, START).await.len();
        self.play_item(id, PlayMode::Auto).await;
        self.sink.send(PlayerEvent::FileLoaded {
            duration: Some(duration),
            tracks: file_tracks(),
        });
        let ready = wait_event(&mut self.events, |e| {
            matches!(e, PlaybackEvent::Ready { .. })
        })
        .await;
        wait_requests(&self.mock, START, starts + 1).await;
        ready
    }

    /// Reports `position`, then the end of the media.
    fn end_at(&self, position: Duration) {
        self.sink.send(PlayerEvent::Position(position));
        self.sink.send(PlayerEvent::Ended);
    }

    fn loaded_ids(&self) -> Vec<String> {
        self.loads()
            .into_iter()
            .map(|source| source.location)
            .collect()
    }
}

fn neighbours_of(ready: &PlaybackEvent) -> (bool, bool) {
    match ready {
        PlaybackEvent::Ready {
            has_previous,
            has_next,
            ..
        } => (*has_previous, *has_next),
        other => panic!("not a Ready event: {other:?}"),
    }
}

fn countdowns(seen: &[PlaybackEvent]) -> Vec<(String, u32)> {
    seen.iter()
        .filter_map(|e| match e {
            PlaybackEvent::NextUpCountdown {
                episode, remaining, ..
            } => Some((episode.id.to_string(), *remaining)),
            _ => None,
        })
        .collect()
}

async fn wait_loads(h: &Harness, count: usize) {
    let start = tokio::time::Instant::now();
    while h.loads().len() < count {
        assert!(
            start.elapsed() < common::DEADLINE,
            "{} loads",
            h.loads().len()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn ended_twice_counts_once() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.end_at(Duration::from_secs(595));
    h.sink.send(PlayerEvent::Ended);
    let seen = h.stop_after_end().await;
    assert_eq!(
        seen.iter()
            .filter(|e| matches!(e, PlaybackEvent::Ended))
            .count(),
        1,
        "{seen:?}"
    );
    assert_eq!(countdowns(&seen), vec![(EPISODES[2].to_owned(), 10)]);
    assert!(seen.contains(&PlaybackEvent::NextUpCancelled));
    // The stop at the real end is the only one: a later start report is the barrier.
    h.start_item(EPISODES[0], TEN_MINUTES).await;
    let stopped = bodies(&h.mock, STOPPED).await;
    assert_eq!(stopped.len(), 1, "{stopped:?}");
    assert_eq!(stopped[0]["PositionTicks"], json!(5_950_000_000_i64));
}

#[tokio::test]
async fn early_end_is_a_stop() {
    let mut h = harness(episode_timings(Duration::from_millis(10))).await;
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.end_at(Duration::from_secs(30));
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Stopped)).await;
    assert!(
        !seen.iter().any(|e| matches!(
            e,
            PlaybackEvent::Ended | PlaybackEvent::NextUpCountdown { .. }
        )),
        "{seen:?}"
    );
    let stopped = wait_requests(&h.mock, STOPPED, 1).await;
    assert_eq!(stopped[0]["PositionTicks"], json!(300_000_000));
    assert!(h.player.calls().contains(&Call::Stop));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.loads().len(), 1);
}

#[tokio::test]
async fn countdown_then_next_episode() {
    let mut h = harness(episode_timings(Duration::from_millis(20))).await;
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.end_at(Duration::from_secs(599));
    let seen = events_until(
        &mut h.events,
        |e| matches!(e, PlaybackEvent::Loading { item, .. } if *item == ItemId::from(EPISODES[2])),
    )
    .await;
    assert!(seen.contains(&PlaybackEvent::Ended));
    let expected: Vec<(String, u32)> = (1..=10)
        .rev()
        .map(|remaining| (EPISODES[2].to_owned(), remaining))
        .collect();
    assert_eq!(countdowns(&seen), expected);
    assert!(!seen.contains(&PlaybackEvent::NextUpCancelled));
    wait_loads(&h, 2).await;
    let next = &h.loads()[1];
    assert!(next.location.contains(EPISODES[2]), "{}", next.location);
    assert_eq!(next.start, None);
}

/// Every event received so far.
fn drain(events: &mut UnboundedReceiver<PlaybackEvent>) -> Vec<PlaybackEvent> {
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    seen
}

/// Lets more than ten steps of a 20 ms countdown pass, then checks that it
/// moved no further and started nothing.
async fn assert_held(h: &mut Harness) {
    tokio::time::sleep(Duration::from_millis(250)).await;
    let later = drain(&mut h.events);
    assert!(countdowns(&later).is_empty(), "{later:?}");
    assert_eq!(h.loads().len(), 1);
}

/// Resumes playback and checks that the countdown goes on from `remaining`
/// down to 1, then starts the next episode.
async fn assert_resumes_from(h: &mut Harness, remaining: u32) {
    h.sink.send(PlayerEvent::Paused(false));
    let seen = events_until(
        &mut h.events,
        |e| matches!(e, PlaybackEvent::Loading { item, .. } if *item == ItemId::from(EPISODES[2])),
    )
    .await;
    let expected: Vec<(String, u32)> = (1..remaining)
        .rev()
        .map(|left| (EPISODES[2].to_owned(), left))
        .collect();
    assert_eq!(countdowns(&seen), expected);
}

#[tokio::test]
async fn pausing_holds_the_countdown() {
    let mut h = harness(episode_timings(Duration::from_millis(20))).await;
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.end_at(Duration::from_secs(599));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::NextUpCountdown { .. })
    })
    .await;
    h.sink.send(PlayerEvent::Paused(true));
    let seen = events_until(&mut h.events, |e| *e == PlaybackEvent::Paused(true)).await;
    let remaining = countdowns(&seen).last().map_or(10, |(_, left)| *left);
    assert_held(&mut h).await;
    assert_resumes_from(&mut h, remaining).await;
}

#[tokio::test]
async fn an_episode_ending_paused_holds_the_countdown() {
    let mut h = harness(episode_timings(Duration::from_millis(20))).await;
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.sink.send(PlayerEvent::Paused(true));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::Paused(true)).await;
    h.end_at(Duration::from_secs(599));
    let first = wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::NextUpCountdown { .. })
    })
    .await;
    assert_eq!(countdowns(&[first]), [(EPISODES[2].to_owned(), 10)]);
    assert_held(&mut h).await;
    assert_resumes_from(&mut h, 10).await;
}

#[tokio::test]
async fn cancel_next_up() {
    let mut h = harness(episode_timings(Duration::from_millis(20))).await;
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.end_at(Duration::from_secs(599));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::NextUpCountdown { .. })
    })
    .await;
    h.handle.send(PlaybackCommand::CancelNextUp);
    wait_event(&mut h.events, |e| *e == PlaybackEvent::NextUpCancelled).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(h.loads().len(), 1);
    let mut later = Vec::new();
    while let Ok(event) = h.events.try_recv() {
        later.push(event);
    }
    assert!(countdowns(&later).is_empty(), "{later:?}");
}

#[tokio::test]
async fn backward_seek_cancels_next_up() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.end_at(Duration::from_secs(599));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::NextUpCountdown { .. })
    })
    .await;
    h.handle.send(PlaybackCommand::Seek(SeekTarget::Backward(
        Duration::from_secs(10),
    )));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::NextUpCancelled).await;
    h.handle.send(PlaybackCommand::PlayNextNow);
    let seen = h.stop_after_end().await;
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, PlaybackEvent::Loading { .. }))
    );
    assert_eq!(h.loads().len(), 1);
}

#[tokio::test]
async fn play_next_now() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.end_at(Duration::from_secs(599));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::NextUpCountdown { .. })
    })
    .await;
    h.handle.send(PlaybackCommand::PlayNextNow);
    let seen = events_until(&mut h.events, |e| {
        matches!(e, PlaybackEvent::Loading { .. })
    })
    .await;
    assert!(!seen.contains(&PlaybackEvent::NextUpCancelled), "{seen:?}");
    wait_loads(&h, 2).await;
    assert!(h.loads()[1].location.contains(EPISODES[2]));
}

#[tokio::test]
async fn previous_and_next_episode() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    let ready = h.start_item(EPISODES[1], TEN_MINUTES).await;
    assert_eq!(neighbours_of(&ready), (true, true));

    h.handle.send(PlaybackCommand::Next);
    wait_loads(&h, 2).await;
    assert!(h.loaded_ids()[1].contains(EPISODES[2]));
    let stopped = wait_requests(&h.mock, STOPPED, 1).await;
    assert_eq!(stopped[0]["ItemId"], json!(EPISODES[1]));
    h.sink.send(PlayerEvent::FileLoaded {
        duration: Some(TEN_MINUTES),
        tracks: file_tracks(),
    });
    let ready = wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Ready { .. })).await;
    assert_eq!(neighbours_of(&ready), (true, true));
    wait_requests(&h.mock, START, 2).await;

    h.handle.send(PlaybackCommand::Previous);
    wait_loads(&h, 3).await;
    assert!(h.loaded_ids()[2].contains(EPISODES[1]));
    let stopped = wait_requests(&h.mock, STOPPED, 2).await;
    assert_eq!(stopped[1]["ItemId"], json!(EPISODES[2]));
}

#[tokio::test]
async fn previous_on_first_episode_restarts() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    let ready = h.start_item(EPISODES[0], TEN_MINUTES).await;
    assert_eq!(neighbours_of(&ready), (true, true));
    h.handle.send(PlaybackCommand::Previous);
    h.player
        .wait_call(|c| *c == Call::Seek(SeekTarget::Absolute(Duration::ZERO)))
        .await;
    assert_eq!(h.loads().len(), 1);
}

#[tokio::test]
async fn previous_on_movie_restarts() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    let ready = h.start().await;
    assert_eq!(neighbours_of(&ready), (true, false));
    h.handle.send(PlaybackCommand::Previous);
    h.player
        .wait_call(|c| *c == Call::Seek(SeekTarget::Absolute(Duration::ZERO)))
        .await;
    h.handle.send(PlaybackCommand::Next);
    h.stop().await;
    assert_eq!(h.loads().len(), 1);
    assert!(
        !h.mock
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .any(|r| r.url.path().starts_with("/Shows/"))
    );
}

#[tokio::test]
async fn last_episode_has_no_next() {
    let mut h = harness(episode_timings(Duration::from_millis(10))).await;
    let ready = h.start_item(EPISODES[3], TEN_MINUTES).await;
    assert_eq!(neighbours_of(&ready), (true, false));
    h.handle.send(PlaybackCommand::Next);
    h.end_at(Duration::from_secs(590));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::Ended).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    h.handle.send(PlaybackCommand::Stop);
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Stopped)).await;
    assert!(countdowns(&seen).is_empty(), "{seen:?}");
    assert!(!seen.contains(&PlaybackEvent::NextUpCancelled), "{seen:?}");
    assert_eq!(h.loads().len(), 1);
}

#[tokio::test]
async fn real_end_stops_the_progress_timer() {
    let mut h = harness(PlaybackTimings {
        progress_interval: Duration::from_millis(20),
        ..episode_timings(Duration::from_secs(3600))
    })
    .await;
    h.start_item(EPISODES[3], TEN_MINUTES).await;
    wait_requests(&h.mock, PROGRESS, 1).await;
    h.end_at(Duration::from_secs(599));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::Ended).await;
    wait_requests(&h.mock, STOPPED, 1).await;
    let after_end = bodies(&h.mock, PROGRESS).await.len();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(bodies(&h.mock, PROGRESS).await.len(), after_end);
}

#[tokio::test]
async fn switching_items_reports_stop_first() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.sink.send(PlayerEvent::Position(Duration::from_secs(42)));
    wait_event(&mut h.events, |e| {
        *e == PlaybackEvent::Position(Duration::from_secs(42))
    })
    .await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(EPISODES[2]),
        mode: PlayMode::Auto,
    });
    wait_loads(&h, 2).await;
    h.sink.send(PlayerEvent::FileLoaded {
        duration: Some(TEN_MINUTES),
        tracks: file_tracks(),
    });
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Ready { .. })).await;
    assert!(
        !seen.iter().any(|e| matches!(
            e,
            PlaybackEvent::Ended | PlaybackEvent::NextUpCountdown { .. }
        )),
        "{seen:?}"
    );
    wait_requests(&h.mock, START, 2).await;
    let reports: Vec<(String, Value)> = h
        .mock
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path().starts_with("/Sessions/Playing"))
        .map(|r| {
            (
                r.url.path().to_owned(),
                serde_json::from_slice(&r.body).unwrap_or(Value::Null),
            )
        })
        .collect();
    let routes: Vec<&str> = reports.iter().map(|(route, _)| route.as_str()).collect();
    assert_eq!(routes, vec![START, STOPPED, START]);
    assert_eq!(reports[1].1["ItemId"], json!(EPISODES[1]));
    assert_eq!(reports[1].1["PositionTicks"], json!(420_000_000));
    assert_eq!(reports[2].1["ItemId"], json!(EPISODES[2]));
}

/// Ends E2 for real and waits for the countdown and the stop report.
async fn end_episode_two(h: &mut Harness) {
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.end_at(Duration::from_secs(599));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::NextUpCountdown { .. })
    })
    .await;
    wait_requests(&h.mock, STOPPED, 1).await;
}

/// Seeks back 30 s from the end as the user would, through the real event order.
async fn seek_back_from_end(h: &mut Harness) {
    h.handle.send(PlaybackCommand::Seek(SeekTarget::Backward(
        Duration::from_secs(30),
    )));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::NextUpCancelled).await;
    h.sink.send(PlayerEvent::Seeking);
    h.sink.send(PlayerEvent::Position(Duration::from_secs(569)));
    h.sink.send(PlayerEvent::Seeked);
    wait_event(&mut h.events, |e| *e == PlaybackEvent::Seeked).await;
}

fn session_reports(requests: Vec<wiremock::Request>) -> Vec<(String, Value)> {
    requests
        .into_iter()
        .filter(|r| r.url.path() == START || r.url.path() == STOPPED)
        .map(|r| {
            (
                r.url.path().to_owned(),
                serde_json::from_slice(&r.body).unwrap_or(Value::Null),
            )
        })
        .collect()
}

#[tokio::test]
async fn seek_back_from_the_end_reports_again() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    end_episode_two(&mut h).await;
    seek_back_from_end(&mut h).await;
    wait_requests(&h.mock, START, 2).await;
    h.sink.send(PlayerEvent::Position(Duration::from_secs(580)));
    wait_event(&mut h.events, |e| {
        *e == PlaybackEvent::Position(Duration::from_secs(580))
    })
    .await;
    h.handle.send(PlaybackCommand::Stop);
    wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Stopped)).await;
    wait_requests(&h.mock, STOPPED, 2).await;
    let reports = session_reports(h.mock.received_requests().await.unwrap_or_default());
    let routes: Vec<&str> = reports.iter().map(|(route, _)| route.as_str()).collect();
    assert_eq!(routes, vec![START, STOPPED, START, STOPPED]);
    assert_eq!(reports[1].1["PositionTicks"], json!(5_990_000_000_i64));
    assert_eq!(reports[2].1["PlaySessionId"], json!(PLAY_SESSION_ID));
    assert_eq!(reports[3].1["PositionTicks"], json!(5_800_000_000_i64));
}

#[tokio::test]
async fn seek_back_from_the_end_resumes_progress_reports() {
    let mut h = harness(PlaybackTimings {
        progress_interval: Duration::from_millis(20),
        ..episode_timings(Duration::from_secs(3600))
    })
    .await;
    end_episode_two(&mut h).await;
    let before = bodies(&h.mock, PROGRESS).await.len();
    seek_back_from_end(&mut h).await;
    wait_requests(&h.mock, PROGRESS, before + 3).await;
}

#[tokio::test]
async fn second_real_end_starts_the_countdown_again() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    end_episode_two(&mut h).await;
    seek_back_from_end(&mut h).await;
    h.end_at(Duration::from_secs(599));
    let seen = events_until(&mut h.events, |e| {
        matches!(e, PlaybackEvent::NextUpCountdown { .. })
    })
    .await;
    assert!(seen.contains(&PlaybackEvent::Ended), "{seen:?}");
    assert_eq!(countdowns(&seen), vec![(EPISODES[2].to_owned(), 10)]);
    wait_requests(&h.mock, STOPPED, 2).await;
}

#[tokio::test]
async fn previous_restarts_a_finished_movie() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    h.start().await;
    h.end_at(Duration::from_millis(10_000));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::Ended).await;
    h.handle.send(PlaybackCommand::Previous);
    h.player
        .wait_call(|c| *c == Call::Seek(SeekTarget::Absolute(Duration::ZERO)))
        .await;
    wait_requests(&h.mock, START, 2).await;
}

#[tokio::test]
async fn play_during_the_countdown_cancels_it() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    end_episode_two(&mut h).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(EPISODES[0]),
        mode: PlayMode::Auto,
    });
    let seen = events_until(&mut h.events, |e| {
        matches!(e, PlaybackEvent::Loading { .. })
    })
    .await;
    assert!(seen.contains(&PlaybackEvent::NextUpCancelled), "{seen:?}");
    let PlaybackEvent::Loading { item, .. } = &seen[seen.len() - 1] else {
        unreachable!();
    };
    assert_eq!(*item, ItemId::from(EPISODES[0]));
    wait_loads(&h, 2).await;
    assert!(h.loaded_ids()[1].contains(EPISODES[0]));
}

#[tokio::test]
async fn absolute_seek_back_cancels_the_countdown() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    end_episode_two(&mut h).await;
    h.handle.send(PlaybackCommand::Seek(SeekTarget::Absolute(
        Duration::from_secs(100),
    )));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::NextUpCancelled).await;
}

#[tokio::test]
async fn shutdown_sends_final_report() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.sink.send(PlayerEvent::Position(Duration::from_secs(7)));
    wait_event(&mut h.events, |e| {
        *e == PlaybackEvent::Position(Duration::from_secs(7))
    })
    .await;
    h.handle.clone().shutdown().await;
    // The final report is sent before shutdown returns.
    let stopped = bodies(&h.mock, STOPPED).await;
    assert_eq!(stopped.len(), 1, "{stopped:?}");
    assert_eq!(stopped[0]["PositionTicks"], json!(70_000_000));
    assert!(h.player.calls().contains(&Call::Stop));
    // The controller has ended: its event channel closes.
    let closed = tokio::time::timeout(common::DEADLINE, async {
        while h.events.recv().await.is_some() {}
    })
    .await;
    assert!(closed.is_ok());
}

#[tokio::test]
async fn shutdown_gives_up_after_the_timeout() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(STOPPED))
        .respond_with(ResponseTemplate::new(204).set_delay(Duration::from_secs(10)))
        .with_priority(1)
        .mount(&mock)
        .await;
    let mut h = harness_on(
        mock,
        PlaybackTimings {
            shutdown_timeout: Duration::from_millis(200),
            ..quiet()
        },
    )
    .await;
    h.start().await;
    let started = tokio::time::Instant::now();
    h.handle.clone().shutdown().await;
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(150), "{elapsed:?}");
    assert!(elapsed < Duration::from_millis(1500), "{elapsed:?}");
}

#[tokio::test]
async fn shutdown_returns_while_a_fetch_is_slow() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{OTHER_ID}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("item_movie"))
                .set_delay(Duration::from_secs(10)),
        )
        .with_priority(1)
        .mount(&mock)
        .await;
    let h = harness_on(
        mock,
        PlaybackTimings {
            shutdown_timeout: Duration::from_millis(200),
            ..quiet()
        },
    )
    .await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(OTHER_ID),
        mode: PlayMode::Auto,
    });
    let started = tokio::time::Instant::now();
    h.handle.clone().shutdown().await;
    assert!(started.elapsed() < Duration::from_millis(1500));
}

#[tokio::test]
async fn shutdown_without_an_item_reports_nothing() {
    let h = harness(quiet()).await;
    h.handle.clone().shutdown().await;
    assert!(
        h.mock
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}

#[tokio::test]
async fn dropping_every_handle_reports_the_stop() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.sink.send(PlayerEvent::Position(Duration::from_secs(8)));
    wait_event(&mut h.events, |e| {
        *e == PlaybackEvent::Position(Duration::from_secs(8))
    })
    .await;
    let Harness {
        mock,
        player,
        handle,
        ..
    } = h;
    drop(handle);
    let stopped = wait_requests(&mock, STOPPED, 1).await;
    assert_eq!(stopped[0]["PositionTicks"], json!(80_000_000));
    player.wait_call(|c| *c == Call::Stop).await;
}

#[tokio::test]
async fn tracks_from_before_the_default_selection_are_ignored() {
    let mut h = harness(quiet()).await;
    h.start().await;
    // mpv reports its own selection (no subtitle) before applying ours.
    let mut stale = file_tracks();
    stale.push(external_track(false));
    h.sink.send(PlayerEvent::Tracks(stale));
    let mut external = track(1, TrackKind::Subtitle, 0, true);
    external.is_external = true;
    external.stream_index = None;
    let mut settled = file_tracks();
    settled.push(external);
    h.sink.send(PlayerEvent::Tracks(settled));
    let seen = h.stop().await;
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, PlaybackEvent::TracksChanged { .. })),
        "{seen:?}"
    );
    assert!(bodies(&h.mock, PROGRESS).await.is_empty());
}

#[tokio::test]
async fn user_selection_after_load_is_reported() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.handle.send(PlaybackCommand::SelectSubtitle(None));
    h.player
        .wait_call(|c| *c == Call::SelectSubtitle(None))
        .await;
    h.sink.send(PlayerEvent::Tracks(file_tracks()));
    let event = wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::TracksChanged { .. })
    })
    .await;
    let PlaybackEvent::TracksChanged { subtitles, .. } = event else {
        unreachable!();
    };
    assert!(subtitles.iter().all(|s| !s.selected));
    let progress = wait_requests(&h.mock, PROGRESS, 1).await;
    assert_eq!(progress[0].get("SubtitleStreamIndex"), None);
}

/// The external subtitle as mpv lists it once added (subtitle id 1).
fn external_track(selected: bool) -> Track {
    let mut external = track(1, TrackKind::Subtitle, 0, selected);
    external.is_external = true;
    external.stream_index = None;
    external
}

#[tokio::test]
async fn expected_track_missing_from_the_list_is_not_awaited() {
    let mut h = harness(quiet()).await;
    h.start().await;
    // The external subtitle the controller selected never appears.
    h.sink.send(PlayerEvent::Tracks(file_tracks()));
    let event = wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::TracksChanged { .. })
    })
    .await;
    let PlaybackEvent::TracksChanged { subtitles, .. } = event else {
        unreachable!();
    };
    assert!(subtitles.iter().all(|s| !s.selected), "{subtitles:?}");
    let progress = wait_requests(&h.mock, PROGRESS, 1).await;
    assert_eq!(progress[0].get("SubtitleStreamIndex"), None);
}

#[tokio::test]
async fn expected_selection_is_dropped_once_playback_moves() {
    let mut h = harness(quiet()).await;
    h.start().await;
    // mpv keeps French (id 2) although English (id 1) was asked for.
    let mut kept = file_tracks();
    kept[1].is_selected = false;
    kept[2].is_selected = true;
    kept.push(external_track(true));
    h.sink.send(PlayerEvent::Tracks(kept.clone()));
    h.sink.send(PlayerEvent::Position(Duration::from_secs(5)));
    wait_event(&mut h.events, |e| {
        *e == PlaybackEvent::Position(Duration::from_secs(5))
    })
    .await;
    assert!(bodies(&h.mock, PROGRESS).await.is_empty());
    h.sink.send(PlayerEvent::Tracks(kept));
    let progress = wait_requests(&h.mock, PROGRESS, 1).await;
    assert_eq!(progress[0]["AudioStreamIndex"], json!(3));
    assert_eq!(progress[0]["SubtitleStreamIndex"], json!(0));
}

#[tokio::test]
async fn user_pick_keeps_the_other_kind_expected() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.handle.send(PlaybackCommand::SelectAudio(2));
    h.player
        .wait_call(|c| *c == Call::SelectAudio(Some(TrackId(1))))
        .await;
    // Still the list from before the default subtitle: ignored.
    h.sink.send(PlayerEvent::Tracks({
        let mut stale = file_tracks();
        stale.push(external_track(false));
        stale
    }));
    let seen = h.stop().await;
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, PlaybackEvent::TracksChanged { .. })),
        "{seen:?}"
    );
    assert!(bodies(&h.mock, PROGRESS).await.is_empty());
}

#[tokio::test]
async fn no_load_after_shutdown() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{OTHER_ID}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("item_movie"))
                .set_delay(Duration::from_millis(500)),
        )
        .with_priority(1)
        .mount(&mock)
        .await;
    let h = harness_on(
        mock,
        PlaybackTimings {
            shutdown_timeout: Duration::from_millis(100),
            ..quiet()
        },
    )
    .await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(OTHER_ID),
        mode: PlayMode::Auto,
    });
    h.handle.clone().shutdown().await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(h.loads().is_empty(), "{:?}", h.loads());
    let info = bodies(&h.mock, &format!("/Items/{OTHER_ID}/PlaybackInfo")).await;
    assert!(info.is_empty());
}

/// Serves `fixture` on `route` after `delay`, ahead of the default routes.
async fn mount_slow(
    mock: &MockServer,
    verb: &str,
    route: &str,
    fixture_name: &str,
    delay: Duration,
) {
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture(fixture_name))
                .set_delay(delay),
        )
        .with_priority(1)
        .mount(mock)
        .await;
}

#[tokio::test]
async fn pause_is_handled_while_play_fetches() {
    let mock = MockServer::start().await;
    let route = format!("/Items/{MOVIE_ID}");
    mount_slow(&mock, "GET", &route, "item_movie", Duration::from_secs(5)).await;
    let h = harness_on(mock, quiet()).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(MOVIE_ID),
        mode: PlayMode::Auto,
    });
    wait_requests(&h.mock, &route, 1).await;
    let sent = tokio::time::Instant::now();
    h.handle.send(PlaybackCommand::SetPaused(true));
    h.player.wait_call(|c| *c == Call::SetPaused(true)).await;
    assert!(
        sent.elapsed() < Duration::from_millis(500),
        "{:?}",
        sent.elapsed()
    );
}

#[tokio::test]
async fn stop_during_fetch_cancels_the_load() {
    let mock = MockServer::start().await;
    let route = format!("/Items/{MOVIE_ID}");
    mount_slow(
        &mock,
        "GET",
        &route,
        "item_movie",
        Duration::from_millis(500),
    )
    .await;
    let mut h = harness_on(mock, quiet()).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(MOVIE_ID),
        mode: PlayMode::Auto,
    });
    wait_requests(&h.mock, &route, 1).await;
    h.handle.send(PlaybackCommand::Stop);
    wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Stopped)).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(h.loads().is_empty(), "{:?}", h.loads());
    for route in [START, PROGRESS, STOPPED] {
        assert!(bodies(&h.mock, route).await.is_empty(), "{route}");
    }
    let info = bodies(&h.mock, &format!("/Items/{MOVIE_ID}/PlaybackInfo")).await;
    assert!(info.is_empty());
}

#[tokio::test]
async fn second_play_supersedes_the_first() {
    let mock = MockServer::start().await;
    let route = format!("/Items/{MOVIE_ID}");
    mount_slow(
        &mock,
        "GET",
        &route,
        "item_movie",
        Duration::from_millis(500),
    )
    .await;
    let mut h = harness_on(mock, quiet()).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(MOVIE_ID),
        mode: PlayMode::Auto,
    });
    wait_requests(&h.mock, &route, 1).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(OTHER_ID),
        mode: PlayMode::FromStart,
    });
    wait_loads(&h, 1).await;
    // Past the moment the first item's response arrives.
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let loaded = h.loaded_ids();
    assert_eq!(loaded.len(), 1, "{loaded:?}");
    assert!(
        loaded[0].contains(&format!("/Videos/{OTHER_ID}/")),
        "{loaded:?}"
    );
    let ready = h.file_loaded().await;
    assert_other_started_single(&h, &ready).await;
    let mut loading = Vec::new();
    while let Ok(event) = h.events.try_recv() {
        loading.push(event);
    }
    assert!(
        !loading.iter().any(
            |e| matches!(e, PlaybackEvent::Loading { item, .. } if *item == ItemId::from(MOVIE_ID))
        ),
        "{loading:?}"
    );
}

#[tokio::test]
async fn episode_list_and_playback_info_run_concurrently() {
    let mock = MockServer::start().await;
    let delay = Duration::from_secs(2);
    let episodes = format!("/Shows/{SERIES_ID}/Episodes");
    mount_slow(&mock, "GET", &episodes, "shows_episodes", delay).await;
    let info = format!("/Items/{}/PlaybackInfo", EPISODES[0]);
    mount_slow(&mock, "POST", &info, "playback_info", delay).await;
    let mut h = harness_on(mock, quiet()).await;
    let started = tokio::time::Instant::now();
    h.play_item(EPISODES[0], PlayMode::Auto).await;
    h.file_loaded().await;
    assert!(
        started.elapsed() < Duration::from_millis(3500),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn no_load_once_shutdown_is_requested() {
    // The controller picks its ready inputs at random: repeat to catch the
    // unload winning over the shutdown message.
    for _ in 0..12 {
        let mut h = harness(PlaybackTimings {
            unload_grace: Duration::from_secs(5),
            shutdown_timeout: Duration::from_millis(200),
            ..quiet()
        })
        .await;
        h.start().await;
        h.handle.send(PlaybackCommand::Play {
            item: ItemId::from(OTHER_ID),
            mode: PlayMode::FromStart,
        });
        h.player.wait_call(|c| *c == Call::Stop).await;
        wait_requests(&h.mock, &format!("/Items/{OTHER_ID}/PlaybackInfo"), 1).await;
        // Lets the fetch result reach the controller and wait for the unload.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let mut shutdown = Box::pin(h.handle.clone().shutdown());
        tokio::select! {
            biased;
            () = &mut shutdown => {}
            () = std::future::ready(()) => {}
        }
        h.sink.send(PlayerEvent::Unloaded(UnloadReason::Stopped));
        shutdown.await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(h.loads().len(), 1, "{:?}", h.loads());
    }
}

/// Plays `EPISODES[1]` behind a slow item fetch, sends `command` during the
/// fetch, then loads it; returns the harness once the first item is `Ready`.
async fn navigate_during_fetch(command: PlaybackCommand) -> Harness {
    let mock = MockServer::start().await;
    let route = format!("/Items/{}", EPISODES[1]);
    mount_slow(
        &mock,
        "GET",
        &route,
        "item_episode",
        Duration::from_millis(500),
    )
    .await;
    let mut h = harness_on(mock, episode_timings(Duration::from_secs(3600))).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(EPISODES[1]),
        mode: PlayMode::Auto,
    });
    wait_requests(&h.mock, &route, 1).await;
    h.handle.send(command);
    wait_loads(&h, 1).await;
    assert!(h.loaded_ids()[0].contains(EPISODES[1]));
    h.file_loaded().await;
    h
}

#[tokio::test]
async fn next_during_a_load_plays_the_next_episode_once_loaded() {
    let h = navigate_during_fetch(PlaybackCommand::Next).await;
    wait_loads(&h, 2).await;
    assert!(
        h.loaded_ids()[1].contains(EPISODES[2]),
        "{:?}",
        h.loaded_ids()
    );
}

#[tokio::test]
async fn previous_during_a_load_plays_the_previous_episode_once_loaded() {
    let h = navigate_during_fetch(PlaybackCommand::Previous).await;
    wait_loads(&h, 2).await;
    assert!(
        h.loaded_ids()[1].contains(EPISODES[0]),
        "{:?}",
        h.loaded_ids()
    );
}

#[tokio::test]
async fn play_during_a_load_drops_the_queued_navigation() {
    let mock = MockServer::start().await;
    let route = format!("/Items/{}", EPISODES[1]);
    mount_slow(
        &mock,
        "GET",
        &route,
        "item_episode",
        Duration::from_millis(500),
    )
    .await;
    let mut h = harness_on(mock, episode_timings(Duration::from_secs(3600))).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(EPISODES[1]),
        mode: PlayMode::Auto,
    });
    wait_requests(&h.mock, &route, 1).await;
    h.handle.send(PlaybackCommand::Next);
    h.play_item(EPISODES[0], PlayMode::Auto).await;
    h.file_loaded().await;
    let marker = Duration::from_millis(999);
    h.sink.send(PlayerEvent::Position(marker));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::Position(marker)).await;
    assert!(
        !h.player.calls().contains(&Call::Stop),
        "{:?}",
        h.player.calls()
    );
    assert_eq!(h.loaded_ids().len(), 1, "{:?}", h.loaded_ids());
}

async fn mount_series(mock: &MockServer, next_up: Value) {
    Mock::given(method("GET"))
        .and(path(format!("/Items/{SERIES_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("item_series")))
        .mount(mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Shows/NextUp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(next_up))
        .mount(mock)
        .await;
}

#[tokio::test]
async fn play_series_starts_the_next_episode_to_watch() {
    let mock = MockServer::start().await;
    mount_series(&mock, fixture("shows_next_up")).await;
    let mut episode = fixture("item_episode");
    episode["Id"] = json!(EPISODES[1]);
    episode["UserData"]["PlaybackPositionTicks"] = json!(70_000_000);
    Mock::given(method("GET"))
        .and(path(format!("/Items/{}", EPISODES[1])))
        .respond_with(ResponseTemplate::new(200).set_body_json(episode))
        .with_priority(1)
        .mount(&mock)
        .await;
    let mut h = harness_on(mock, quiet()).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(SERIES_ID),
        mode: PlayMode::FromStart,
    });
    let event = wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::Loading { .. })
    })
    .await;
    let PlaybackEvent::Loading { item, title } = event else {
        unreachable!();
    };
    assert_eq!(item, ItemId::from(EPISODES[1]));
    assert_eq!(title.series_name.as_deref(), Some("Test Show"));
    assert_eq!(title.episode_label.as_deref(), Some("S01E01"));
    let start = tokio::time::Instant::now();
    while h.loads().is_empty() {
        assert!(start.elapsed() < common::DEADLINE, "no load");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let source = load_call(&h.player);
    assert!(
        source
            .location
            .contains(&format!("/Videos/{}/stream", EPISODES[1])),
        "{}",
        source.location
    );
    assert_eq!(source.start, Some(Duration::from_secs(7)));
}

#[tokio::test]
async fn play_series_without_episode_reports_nothing_to_play() {
    let mock = MockServer::start().await;
    mount_series(&mock, json!({ "Items": [], "TotalRecordCount": 0 })).await;
    let all_played = json!({ "Items": [], "TotalRecordCount": 0 });
    Mock::given(method("GET"))
        .and(path(format!("/Shows/{SERIES_ID}/Episodes")))
        .respond_with(ResponseTemplate::new(200).set_body_json(all_played))
        .with_priority(1)
        .mount(&mock)
        .await;
    let mut h = harness_on(mock, quiet()).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(SERIES_ID),
        mode: PlayMode::Auto,
    });
    wait_event(&mut h.events, |e| {
        *e == PlaybackEvent::Error(PlaybackError::NothingToPlay)
    })
    .await;
    assert!(!h.player.calls().iter().any(|c| matches!(c, Call::Load(_))));
}

fn segment_json(kind: &str, start: Duration, end: Duration) -> Value {
    json!({
        "Id": "0f1e2d3c4b5a69788796a5b4c3d2e1f0",
        "ItemId": MOVIE_ID,
        "Type": kind,
        "StartTicks": start.as_nanos() / 100,
        "EndTicks": end.as_nanos() / 100,
    })
}

async fn mount_segments(mock: &MockServer, id: &str, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(format!("/MediaSegments/{id}")))
        .respond_with(response)
        .mount(mock)
        .await;
}

fn segments_body(items: Vec<Value>) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "Items": items, "TotalRecordCount": 0 }))
}

fn intro(start: u64, end: u64) -> SkippableSegment {
    SkippableSegment {
        kind: SegmentKind::Intro,
        start: Duration::from_secs(start),
        end: Duration::from_secs(end),
    }
}

fn segment_events(seen: &[PlaybackEvent]) -> Vec<Option<SkippableSegment>> {
    seen.iter()
        .filter_map(|e| match e {
            PlaybackEvent::Segment(segment) => Some(*segment),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn intro_is_offered_while_the_position_is_inside() {
    let mock = MockServer::start().await;
    let body = segments_body(vec![
        segment_json("Intro", Duration::from_secs(2), Duration::from_secs(8)),
        segment_json("Commercial", Duration::ZERO, Duration::from_secs(10)),
    ]);
    mount_segments(&mock, MOVIE_ID, body).await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    let seen = h.file_loaded_events().await;
    assert!(segment_events(&seen).is_empty(), "{seen:?}");

    h.sink.send(PlayerEvent::Position(Duration::from_secs(1)));
    h.sink.send(PlayerEvent::Position(Duration::from_secs(3)));
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Segment(_))).await;
    assert_eq!(segment_events(&seen), vec![Some(intro(2, 8))]);
    assert!(seen.contains(&PlaybackEvent::Position(Duration::from_secs(3))));

    h.sink.send(PlayerEvent::Position(Duration::from_secs(4)));
    let target = SeekTarget::Absolute(Duration::from_secs(8));
    h.handle.send(PlaybackCommand::Seek(target));
    h.player.wait_call(|c| *c == Call::Seek(target)).await;
    h.sink.send(PlayerEvent::Seeking);
    h.sink
        .send(PlayerEvent::Position(Duration::from_millis(7_960)));
    h.sink.send(PlayerEvent::Seeked);
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;
    assert_eq!(segment_events(&seen), vec![None], "{seen:?}");

    h.sink.send(PlayerEvent::Position(Duration::from_secs(5)));
    let seen = h.stop().await;
    assert_eq!(
        segment_events(&seen),
        vec![Some(intro(2, 8)), None],
        "{seen:?}"
    );
}

#[tokio::test]
async fn segments_that_arrive_late_are_offered_at_once() {
    let mock = MockServer::start().await;
    let body = segments_body(vec![segment_json(
        "Intro",
        Duration::ZERO,
        Duration::from_secs(8),
    )])
    .set_delay(Duration::from_millis(300));
    mount_segments(&mock, MOVIE_ID, body).await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    let seen = h.file_loaded_events().await;
    assert!(segment_events(&seen).is_empty(), "{seen:?}");
    let segment = wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Segment(_))).await;
    assert_eq!(segment, PlaybackEvent::Segment(Some(intro(0, 8))));
}

#[tokio::test]
async fn segments_of_an_item_waiting_to_load_are_kept() {
    let mock = MockServer::start().await;
    let body = segments_body(vec![segment_json(
        "Recap",
        Duration::ZERO,
        Duration::from_secs(5),
    )]);
    mount_segments(&mock, OTHER_ID, body).await;
    let mut h = harness_on(
        mock,
        PlaybackTimings {
            unload_grace: Duration::from_secs(5),
            ..quiet()
        },
    )
    .await;
    h.start().await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(OTHER_ID),
        mode: PlayMode::FromStart,
    });
    h.player.wait_call(|c| *c == Call::Stop).await;
    let start = tokio::time::Instant::now();
    while bodies(&h.mock, &format!("/MediaSegments/{OTHER_ID}"))
        .await
        .is_empty()
    {
        assert!(start.elapsed() < common::DEADLINE, "no segments request");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    h.sink.send(PlayerEvent::Unloaded(UnloadReason::Stopped));
    wait_loads(&h, 2).await;
    h.file_loaded().await;
    h.sink.send(PlayerEvent::Position(Duration::from_secs(1)));
    let segment = wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Segment(_))).await;
    let recap = SkippableSegment {
        kind: SegmentKind::Recap,
        start: Duration::ZERO,
        end: Duration::from_secs(5),
    };
    assert_eq!(segment, PlaybackEvent::Segment(Some(recap)));
}

#[tokio::test]
async fn skipping_the_credits_to_the_end_starts_the_countdown() {
    let mock = MockServer::start().await;
    let credits = segment_json("Outro", Duration::from_secs(540), Duration::from_secs(700));
    mount_segments(&mock, EPISODES[1], segments_body(vec![credits])).await;
    let mut h = harness_on(mock, episode_timings(Duration::from_secs(3600))).await;
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.sink.send(PlayerEvent::Position(Duration::from_secs(550)));
    let segment = wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Segment(_))).await;
    let offered = SkippableSegment {
        kind: SegmentKind::Credits,
        start: Duration::from_secs(540),
        end: TEN_MINUTES,
    };
    assert_eq!(segment, PlaybackEvent::Segment(Some(offered)));

    let target = SeekTarget::Absolute(offered.end);
    h.handle.send(PlaybackCommand::Seek(target));
    h.player.wait_call(|c| *c == Call::Seek(target)).await;
    h.sink.send(PlayerEvent::Seeking);
    h.sink
        .send(PlayerEvent::Position(Duration::from_millis(599_990)));
    h.sink.send(PlayerEvent::Seeked);
    h.sink.send(PlayerEvent::Ended);
    let seen = events_until(&mut h.events, |e| {
        matches!(e, PlaybackEvent::NextUpCountdown { .. })
    })
    .await;
    assert_eq!(segment_events(&seen), vec![None], "{seen:?}");
    assert!(seen.contains(&PlaybackEvent::Ended), "{seen:?}");
    assert_eq!(countdowns(&seen), vec![(EPISODES[2].to_owned(), 10)]);
}

#[tokio::test]
async fn failed_segments_only_skip_the_feature() {
    let mock = MockServer::start().await;
    mount_segments(&mock, MOVIE_ID, ResponseTemplate::new(500)).await;
    let mut h = harness_on(mock, quiet()).await;
    h.start().await;
    wait_requests(&h.mock, &format!("/MediaSegments/{MOVIE_ID}"), 1).await;
    h.sink.send(PlayerEvent::Position(Duration::from_secs(4)));
    let seen = h.stop().await;
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, PlaybackEvent::Segment(_) | PlaybackEvent::Error(_))),
        "{seen:?}"
    );
}

fn large_box() -> SubtitleStyle {
    SubtitleStyle {
        size: SubtitleSize::Large,
        background: SubtitleBackground::Box,
        ..SubtitleStyle::default()
    }
}

#[tokio::test]
async fn subtitle_style_applies_with_nothing_playing() {
    let h = harness(quiet()).await;
    h.handle
        .send(PlaybackCommand::SetSubtitleStyle(large_box()));
    h.player
        .wait_call(|c| *c == Call::SetSubtitles(large_box().appearance()))
        .await;
}

#[tokio::test]
async fn subtitle_style_applies_again_before_each_load() {
    let mut h = harness(quiet()).await;
    h.handle
        .send(PlaybackCommand::SetSubtitleStyle(large_box()));
    h.start().await;
    let styled = Call::SetSubtitles(large_box().appearance());
    let calls = h.player.calls();
    let applied: Vec<usize> = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| **call == styled)
        .map(|(index, _)| index)
        .collect();
    let load = calls
        .iter()
        .position(|call| matches!(call, Call::Load(_)))
        .unwrap_or_else(|| panic!("no load in {calls:?}"));
    assert_eq!(applied.len(), 2, "{calls:?}");
    assert_eq!(applied[1] + 1, load, "{calls:?}");
}

#[tokio::test]
async fn without_a_style_loads_leave_subtitles_alone() {
    let mut h = harness(quiet()).await;
    h.start().await;
    assert!(
        !h.player
            .calls()
            .iter()
            .any(|call| matches!(call, Call::SetSubtitles(_)))
    );
}

#[tokio::test]
async fn trickplay_of_the_source_comes_just_before_ready() {
    let mock = MockServer::start().await;
    let mut item = fixture("item_movie");
    item["Trickplay"] = json!({
        MOVIE_ID: {
            "160": {"Width": 160, "Height": 90, "TileWidth": 3, "TileHeight": 3,
                    "ThumbnailCount": 10, "Interval": 1000, "Bandwidth": 27125}
        }
    });
    Mock::given(method("GET"))
        .and(path(format!("/Items/{MOVIE_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(item))
        .with_priority(1)
        .mount(&mock)
        .await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    let seen = h.file_loaded_events().await;
    let [
        ..,
        PlaybackEvent::Trickplay(trickplay),
        PlaybackEvent::Ready { .. },
    ] = &seen[..]
    else {
        panic!("no trickplay just before Ready: {seen:?}");
    };
    assert_eq!(
        *trickplay,
        Trickplay {
            item: ItemId::from(MOVIE_ID),
            media_source: MediaSourceId::from(MOVIE_ID),
            resolution: 160,
            width: 160,
            height: 90,
            columns: 3,
            rows: 3,
            count: 10,
            interval: Duration::from_secs(1),
        }
    );

    h.play_item(OTHER_ID, PlayMode::FromStart).await;
    let seen = h.file_loaded_events().await;
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, PlaybackEvent::Trickplay(_))),
        "an item without trickplay has none: {seen:?}"
    );
}

const ENCODINGS: &str = "/Videos/ActiveEncodings";
const PLAYBACK_INFO: &str = "/Items/76282b1c408f374beeee50519e9157a4/PlaybackInfo";
/// The 1.5 Mbps limit of [`QUALITIES`].
const LOW: Quality = QUALITIES[4];

/// The fixture answer for a transcode of the movie with its English audio.
fn transcode_info() -> Value {
    let mut info = fixture("playback_info");
    let source = &mut info["MediaSources"][0];
    source["SupportsDirectPlay"] = json!(false);
    source["SupportsTranscoding"] = json!(true);
    source["TranscodingUrl"] = json!(format!(
        "/videos/{MOVIE_ID}/master.m3u8?MediaSourceId={MOVIE_ID}&AudioStreamIndex=2&PlaySessionId={PLAY_SESSION_ID}&ApiKey={TEST_TOKEN}&TranscodeReasons=ContainerBitrateExceedsLimit"
    ));
    source["TranscodingSubProtocol"] = json!("hls");
    let subtitle = &mut source["MediaStreams"][0];
    subtitle["DeliveryMethod"] = json!("External");
    subtitle["DeliveryUrl"] = json!(format!(
        "/Videos/{MOVIE_ID}/{MOVIE_ID}/Subtitles/0/0/Stream.subrip?ApiKey={TEST_TOKEN}"
    ));
    info
}

/// Answers a playback info request asking for `bitrate` with a transcode.
async fn mount_transcode_for(mock: &MockServer, bitrate: u64) {
    Mock::given(method("POST"))
        .and(path(PLAYBACK_INFO))
        .and(body_partial_json(json!({ "MaxStreamingBitrate": bitrate })))
        .respond_with(ResponseTemplate::new(200).set_body_json(transcode_info()))
        .with_priority(1)
        .mount(mock)
        .await;
}

/// The tracks mpv reports for a transcode: one video and one audio track.
fn transcode_tracks() -> Vec<Track> {
    vec![
        track(1, TrackKind::Video, 0, true),
        track(1, TrackKind::Audio, 1, true),
    ]
}

/// URL of the subtitle file the server delivers with [`transcode_info`].
fn delivered_subtitle_url(h: &Harness) -> String {
    format!(
        "{}/Videos/{MOVIE_ID}/{MOVIE_ID}/Subtitles/0/0/Stream.subrip",
        h.mock.uri()
    )
}

impl Harness {
    /// mpv lists the subtitle file of `url`, added in the background, as
    /// subtitle track 2 next to the transcode's tracks.
    async fn subtitle_arrives(&mut self, url: &str, selected: bool) {
        let mut tracks = transcode_tracks();
        tracks.push(Track {
            id: TrackId(2),
            kind: TrackKind::Subtitle,
            language: Some("eng".into()),
            title: None,
            codec: None,
            channels: None,
            is_default: false,
            is_forced: false,
            is_external: true,
            is_selected: selected,
            stream_index: None,
            external_url: Some(url.to_owned()),
        });
        self.sink.send(PlayerEvent::Tracks(tracks));
    }
}

impl Harness {
    /// Waits until the player received `count` loads; returns the last one.
    async fn wait_load(&self, count: usize) -> oxyfin_player::MediaSource {
        let start = tokio::time::Instant::now();
        loop {
            let loads = self.loads();
            if loads.len() >= count {
                return loads[count - 1].clone();
            }
            assert!(start.elapsed() < common::DEADLINE, "no load {count}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Sends `FileLoaded` with `tracks` and waits for `Ready`.
    async fn loaded_with(&mut self, tracks: Vec<Track>) -> PlaybackEvent {
        self.sink.send(PlayerEvent::FileLoaded {
            duration: Some(Duration::from_millis(10_023)),
            tracks,
        });
        wait_event(&mut self.events, |e| {
            matches!(e, PlaybackEvent::Ready { .. })
        })
        .await
    }

    /// Moves the position to `position` and waits until the controller saw it.
    async fn move_to(&mut self, position: Duration) {
        self.sink.send(PlayerEvent::Position(position));
        wait_event(&mut self.events, |e| {
            *e == PlaybackEvent::Position(position)
        })
        .await;
    }
}

/// Whether a playback info request says the player draws the image subtitles of the file.
fn embeds_image_subtitles(info: &Value) -> bool {
    info["DeviceProfile"]["SubtitleProfiles"]
        .as_array()
        .is_some_and(|profiles| profiles.contains(&json!({"Format": "pgssub", "Method": "Embed"})))
}

/// The fixture answer with a French image subtitle (4) inside the file, shown by default.
fn image_subtitle_info() -> Value {
    let mut info = fixture("playback_info");
    let source = &mut info["MediaSources"][0];
    source["DefaultSubtitleStreamIndex"] = json!(4);
    let streams = source["MediaStreams"].as_array_mut();
    streams.unwrap_or_else(|| panic!("no streams")).push(json!({
        "Index": 4,
        "Type": "Subtitle",
        "Codec": "PGSSUB",
        "Language": "fre",
        "IsExternal": false,
        "IsTextSubtitleStream": false,
        "DeliveryMethod": "Embed",
    }));
    info
}

fn image_subtitle_tracks() -> Vec<Track> {
    let mut tracks = file_tracks();
    tracks.push(track(7, TrackKind::Subtitle, 3, false));
    tracks
}

#[tokio::test]
async fn the_default_image_subtitle_of_the_file_is_selected_on_direct_play() {
    let mock = MockServer::start().await;
    mount_playback_info(&mock, image_subtitle_info()).await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    let ready = h.loaded_with(image_subtitle_tracks()).await;
    position_of(&h.player.calls(), &Call::SelectSubtitle(Some(TrackId(7))));
    let PlaybackEvent::Ready { subtitles, .. } = ready else {
        unreachable!();
    };
    let selected: Vec<i32> = subtitles
        .iter()
        .filter(|subtitle| subtitle.selected)
        .map(|subtitle| subtitle.index)
        .collect();
    assert_eq!(selected, [4]);
    let start = wait_requests(&h.mock, START, 1).await;
    assert_eq!(start[0]["PlayMethod"], json!("DirectPlay"));
    assert_eq!(start[0]["SubtitleStreamIndex"], json!(4));
}

#[tokio::test]
async fn a_remembered_off_hides_the_default_image_subtitle_on_direct_play() {
    let mock = MockServer::start().await;
    mount_info_for(&mock, EPISODES[0], image_subtitle_info()).await;
    let mut h = harness_in(
        mock,
        episode_timings(Duration::from_secs(3600)),
        remembered_french(),
    )
    .await;
    let calls = h.load_episode(EPISODES[0], image_subtitle_tracks()).await;
    assert_eq!(selected_subtitles(&calls), [None], "{calls:?}");
}

#[tokio::test]
async fn the_quality_limit_is_asked_of_the_server() {
    let mut h = harness(quiet()).await;
    h.play(PlayMode::Auto).await;
    let first = wait_requests(&h.mock, PLAYBACK_INFO, 1).await;
    assert_eq!(first[0]["MaxStreamingBitrate"], json!(1_000_000_000));
    assert!(embeds_image_subtitles(&first[0]));
    assert!(first[0]["DeviceProfile"]["TranscodingProfiles"].is_array());

    h.handle.send(PlaybackCommand::SetQuality(LOW));
    h.play_item(OTHER_ID, PlayMode::Auto).await;
    let other = wait_requests(&h.mock, &format!("/Items/{OTHER_ID}/PlaybackInfo"), 1).await;
    assert_eq!(other[0]["MaxStreamingBitrate"], json!(1_500_000));
    assert!(!embeds_image_subtitles(&other[0]));
}

#[tokio::test]
async fn a_transcode_plays_the_transcoding_url() {
    let mock = MockServer::start().await;
    mount_playback_info(&mock, transcode_info()).await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    let source = load_call(&h.player);
    assert!(
        source
            .location
            .starts_with(&format!("{}/videos/{MOVIE_ID}/master.m3u8?", h.mock.uri())),
        "{}",
        source.location
    );
    assert!(!source.location.contains(TEST_TOKEN), "{}", source.location);
    assert!(
        source
            .headers
            .iter()
            .any(|(name, value)| name == "Authorization" && value.contains(TEST_TOKEN))
    );

    let ready = h.loaded_with(transcode_tracks()).await;
    let url = delivered_subtitle_url(&h);
    let calls = h.player.calls();
    assert!(
        calls.contains(&Call::AddExternalLater(url.clone())),
        "{calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|call| matches!(call, Call::AddExternal { .. })),
        "the server's subtitle file was awaited: {calls:?}"
    );
    let PlaybackEvent::Ready {
        audio, subtitles, ..
    } = ready
    else {
        unreachable!();
    };
    let audio: Vec<_> = audio.iter().map(|a| (a.index, a.selected)).collect();
    assert_eq!(audio, [(2, true), (3, false)]);
    assert!(subtitles.is_empty(), "{subtitles:?}");

    h.subtitle_arrives(&url, false).await;
    h.player
        .wait_call(|call| *call == Call::SelectSubtitle(Some(TrackId(2))))
        .await;
    h.subtitle_arrives(&url, true).await;
    let changed = wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::TracksChanged { .. })
    })
    .await;
    let PlaybackEvent::TracksChanged { subtitles, .. } = changed else {
        unreachable!();
    };
    let subtitles: Vec<_> = subtitles.iter().map(|s| (s.index, s.selected)).collect();
    assert_eq!(subtitles, [(0, true)]);

    let start = wait_requests(&h.mock, START, 1).await;
    assert_eq!(start[0]["PlayMethod"], json!("Transcode"));
    assert_eq!(start[0]["PlaySessionId"], json!(PLAY_SESSION_ID));
    assert_eq!(start[0]["AudioStreamIndex"], json!(2));
}

#[tokio::test]
async fn stopping_a_transcode_ends_it_on_the_server() {
    let mock = MockServer::start().await;
    mount_playback_info(&mock, transcode_info()).await;
    mount_status(&mock, "DELETE", ENCODINGS, 204).await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    h.loaded_with(transcode_tracks()).await;
    wait_requests(&h.mock, START, 1).await;
    h.stop().await;
    wait_requests(&h.mock, ENCODINGS, 1).await;
    let requests = h.mock.received_requests().await.unwrap_or_default();
    let delete = requests
        .iter()
        .find(|request| request.url.path() == ENCODINGS)
        .unwrap();
    let query = delete.url.query().unwrap_or_default();
    assert!(
        query.contains(&format!("playSessionId={PLAY_SESSION_ID}")),
        "{query}"
    );
    assert!(query.contains("deviceId=test-device-id"), "{query}");
    let stopped = position_of_route(&requests, STOPPED);
    assert!(stopped < position_of_route(&requests, ENCODINGS));
}

#[tokio::test]
async fn a_stop_report_is_told_once_the_server_answered_it() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(STOPPED))
        .respond_with(ResponseTemplate::new(204).set_delay(Duration::from_millis(200)))
        .with_priority(1)
        .mount(&mock)
        .await;
    let mut h = harness_on(mock, quiet()).await;
    h.start().await;
    h.handle.send(PlaybackCommand::Stop);
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::StopReported)).await;
    let stopped = seen
        .iter()
        .position(|e| matches!(e, PlaybackEvent::Stopped));
    assert!(
        stopped.is_some_and(|stopped| stopped + 1 < seen.len()),
        "{seen:?}"
    );
    assert_eq!(bodies(&h.mock, STOPPED).await.len(), 1);
}

#[tokio::test]
async fn direct_play_ends_no_transcode() {
    let mock = MockServer::start().await;
    mount_status(&mock, "DELETE", ENCODINGS, 204).await;
    let mut h = harness_on(mock, quiet()).await;
    h.start().await;
    h.stop().await;
    let start = bodies(&h.mock, START).await;
    assert_eq!(start[0]["PlayMethod"], json!("DirectPlay"));
    assert!(bodies(&h.mock, ENCODINGS).await.is_empty());
}

#[tokio::test]
async fn a_quality_change_reloads_at_the_position_with_the_tracks() {
    let mock = MockServer::start().await;
    mount_transcode_for(&mock, 1_500_000).await;
    mount_status(&mock, "DELETE", ENCODINGS, 204).await;
    let mut h = harness_on(mock, quiet()).await;
    h.start().await;
    h.move_to(Duration::from_secs(5)).await;
    h.handle.send(PlaybackCommand::SetSpeed(1.5));
    h.sink.send(PlayerEvent::Speed(1.5));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::Speed(1.5)).await;

    h.handle.send(PlaybackCommand::SetQuality(LOW));
    let reload = h.wait_load(2).await;
    assert!(
        reload.location.contains("/master.m3u8?"),
        "{}",
        reload.location
    );
    assert_eq!(reload.start, Some(Duration::from_secs(5)));
    let infos = wait_requests(&h.mock, PLAYBACK_INFO, 2).await;
    assert_eq!(infos[1]["MaxStreamingBitrate"], json!(1_500_000));
    assert!(infos[0]["DeviceProfile"].get("CodecProfiles").is_none());
    assert_eq!(
        infos[1]["DeviceProfile"]["CodecProfiles"][0]["Conditions"][0]["Value"],
        json!("480")
    );
    assert!(infos[0].get("MediaSourceId").is_none(), "{:?}", infos[0]);
    assert_eq!(infos[1]["MediaSourceId"], json!(MOVIE_ID));
    assert_eq!(infos[1]["AudioStreamIndex"], json!(2));
    assert_eq!(infos[1]["SubtitleStreamIndex"], json!(0));
    let stopped = wait_requests(&h.mock, STOPPED, 1).await;
    assert_eq!(stopped[0]["PositionTicks"], json!(50_000_000));
    assert!(bodies(&h.mock, ENCODINGS).await.is_empty());

    let calls_before = h.player.calls().len();
    h.loaded_with(transcode_tracks()).await;
    let url = delivered_subtitle_url(&h);
    h.subtitle_arrives(&url, false).await;
    h.player
        .wait_call(|call| *call == Call::SelectSubtitle(Some(TrackId(2))))
        .await;
    let calls = h.player.calls()[calls_before..].to_vec();
    position_of(&calls, &Call::SelectAudio(Some(TrackId(1))));
    let subtitle = calls
        .iter()
        .find_map(|call| match call {
            Call::SelectSubtitle(Some(id)) => Some(*id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no subtitle selected: {calls:?}"));
    assert_ne!(subtitle, TrackId(1), "the subtitle of the first load");
    position_of(&calls, &Call::SetSpeed(1.5));
    position_of(&calls, &Call::SetPaused(false));
    let start = wait_requests(&h.mock, START, 2).await;
    assert_eq!(start[1]["PlayMethod"], json!("Transcode"));

    h.handle
        .send(PlaybackCommand::SetQuality(Quality::Original));
    let back = h.wait_load(3).await;
    assert!(
        back.location.contains("/stream?static=true"),
        "{}",
        back.location
    );
    wait_requests(&h.mock, ENCODINGS, 1).await;
}

#[tokio::test]
async fn an_audio_change_during_a_transcode_reloads_with_that_audio() {
    let mock = MockServer::start().await;
    mount_playback_info(&mock, transcode_info()).await;
    mount_status(&mock, "DELETE", ENCODINGS, 204).await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    h.loaded_with(transcode_tracks()).await;
    h.move_to(Duration::from_secs(4)).await;

    h.handle.send(PlaybackCommand::SelectAudio(3));
    let reload = h.wait_load(2).await;
    assert_eq!(reload.start, Some(Duration::from_secs(4)));
    let infos = wait_requests(&h.mock, PLAYBACK_INFO, 2).await;
    assert_eq!(infos[1]["MediaSourceId"], json!(MOVIE_ID));
    assert_eq!(infos[1]["AudioStreamIndex"], json!(3));
    assert!(
        !h.player
            .calls()
            .iter()
            .any(|call| matches!(call, Call::SelectAudio(Some(TrackId(id))) if *id != 1)),
    );
    wait_requests(&h.mock, ENCODINGS, 1).await;
}

#[tokio::test]
async fn a_quality_change_while_loading_restarts_the_load() {
    let mock = MockServer::start().await;
    mount_transcode_for(&mock, 1_500_000).await;
    let h = harness_on(mock, quiet()).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(MOVIE_ID),
        mode: PlayMode::FromStart,
    });
    h.handle.send(PlaybackCommand::SetQuality(LOW));
    let load = h.wait_load(1).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(h.loads().len(), 1);
    assert!(load.location.contains("/master.m3u8?"), "{}", load.location);
}

fn position_of_route(requests: &[wiremock::Request], route: &str) -> usize {
    requests
        .iter()
        .position(|request| request.url.path() == route)
        .unwrap_or_else(|| panic!("no request on {route}"))
}

#[tokio::test]
async fn a_quality_change_keeps_the_pause_and_the_item_extras() {
    let mock = MockServer::start().await;
    mount_transcode_for(&mock, 1_500_000).await;
    let mut item = fixture("item_movie");
    item["Trickplay"] = json!({
        MOVIE_ID: {
            "160": {"Width": 160, "Height": 90, "TileWidth": 3, "TileHeight": 3,
                    "ThumbnailCount": 10, "Interval": 1000, "Bandwidth": 27125}
        }
    });
    Mock::given(method("GET"))
        .and(path(format!("/Items/{MOVIE_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(item))
        .with_priority(1)
        .mount(&mock)
        .await;
    let intro_json = segment_json("Intro", Duration::ZERO, Duration::from_secs(8));
    mount_segments(&mock, MOVIE_ID, segments_body(vec![intro_json])).await;
    let mut h = harness_on(mock, quiet()).await;
    h.handle
        .send(PlaybackCommand::SetSubtitleStyle(large_box()));
    h.start().await;
    h.move_to(Duration::from_secs(5)).await;
    h.sink.send(PlayerEvent::Paused(true));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::Paused(true)).await;

    let first_loads = h.player.calls().len();
    h.handle.send(PlaybackCommand::SetQuality(LOW));
    h.wait_load(2).await;
    let reload_calls = h.player.calls()[first_loads..].to_vec();
    let styled = position_of(&reload_calls, &Call::SetSubtitles(large_box().appearance()));
    assert!(
        styled
            < reload_calls
                .iter()
                .position(|c| matches!(c, Call::Load(_)))
                .unwrap()
    );

    let calls_before = h.player.calls().len();
    h.sink.send(PlayerEvent::FileLoaded {
        duration: Some(Duration::from_millis(10_023)),
        tracks: transcode_tracks(),
    });
    let seen = events_until(&mut h.events, |e| *e == PlaybackEvent::Paused(true)).await;
    let ready = seen
        .iter()
        .position(|e| matches!(e, PlaybackEvent::Ready { .. }))
        .unwrap_or_else(|| panic!("no Ready: {seen:?}"));
    assert!(
        matches!(seen[ready - 1], PlaybackEvent::Trickplay(_)),
        "{seen:?}"
    );
    let calls = h.player.calls()[calls_before..].to_vec();
    position_of(&calls, &Call::SetPaused(true));
    let start = wait_requests(&h.mock, START, 2).await;
    assert_eq!(start[1]["IsPaused"], json!(true));
    assert_eq!(start[1]["PositionTicks"], json!(50_000_000));

    let segment = PlaybackEvent::Segment(Some(intro(0, 8)));
    if !seen[ready..].contains(&segment) {
        wait_event(&mut h.events, |e| *e == segment).await;
    }
}

#[tokio::test]
async fn subtitles_off_while_the_subtitle_file_loads_stay_off() {
    let mock = MockServer::start().await;
    mount_playback_info(&mock, transcode_info()).await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    h.loaded_with(transcode_tracks()).await;
    let url = delivered_subtitle_url(&h);
    h.player
        .wait_call(|call| *call == Call::AddExternalLater(url.clone()))
        .await;

    let offs = |h: &Harness| {
        h.player
            .calls()
            .iter()
            .filter(|call| **call == Call::SelectSubtitle(None))
            .count()
    };
    let before = offs(&h);
    h.handle.send(PlaybackCommand::SelectSubtitle(None));
    let start = tokio::time::Instant::now();
    while offs(&h) == before {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "subtitles not turned off"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let calls_before = h.player.calls().len();
    h.subtitle_arrives(&url, false).await;
    h.move_to(Duration::from_secs(1)).await;
    let calls = h.player.calls()[calls_before..].to_vec();
    assert!(
        !calls
            .iter()
            .any(|call| matches!(call, Call::SelectSubtitle(Some(_)))),
        "the late subtitle overrode the choice: {calls:?}"
    );
}

#[tokio::test]
async fn a_quality_switch_while_the_subtitle_file_loads_keeps_the_subtitle() {
    let mock = MockServer::start().await;
    mount_playback_info(&mock, transcode_info()).await;
    mount_transcode_for(&mock, 1_500_000).await;
    mount_status(&mock, "DELETE", ENCODINGS, 204).await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    h.loaded_with(transcode_tracks()).await;

    h.handle.send(PlaybackCommand::SetQuality(LOW));
    h.wait_load(2).await;
    let infos = wait_requests(&h.mock, PLAYBACK_INFO, 2).await;
    assert_eq!(infos[1]["SubtitleStreamIndex"], json!(0));
}

#[tokio::test]
async fn quality_switch_at_start_keeps_position_zero() {
    let mock = MockServer::start().await;
    mount_transcode_for(&mock, 1_500_000).await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    h.file_loaded().await;

    h.handle.send(PlaybackCommand::SetQuality(LOW));
    let reload = h.wait_load(2).await;
    assert_eq!(reload.start, None);
}

#[tokio::test]
async fn quality_switch_near_end_keeps_the_end() {
    let mock = MockServer::start().await;
    mount_transcode_for(&mock, 1_500_000).await;
    let mut h = harness_on(mock, quiet()).await;
    h.start().await;
    h.move_to(Duration::from_secs(9)).await;

    h.handle.send(PlaybackCommand::SetQuality(LOW));
    let reload = h.wait_load(2).await;
    assert_eq!(reload.start, Some(Duration::from_secs(9)));
}

#[tokio::test]
async fn delays_are_set_and_reported_in_milliseconds() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.handle.send(PlaybackCommand::SetSubtitleDelay(300));
    h.handle.send(PlaybackCommand::SetAudioDelay(-100));
    let seen = events_until(&mut h.events, |e| *e == PlaybackEvent::AudioDelay(-100)).await;
    assert!(
        seen.contains(&PlaybackEvent::SubtitleDelay(300)),
        "{seen:?}"
    );
    let calls = h.player.calls();
    assert!(calls.contains(&Call::SetSubtitleDelay(300)));
    assert!(calls.contains(&Call::SetAudioDelay(-100)));
}

#[tokio::test]
async fn the_next_item_starts_without_the_delays() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.handle.send(PlaybackCommand::SetSubtitleDelay(300));
    h.handle.send(PlaybackCommand::SetAudioDelay(-100));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::AudioDelay(-100)).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(OTHER_ID),
        mode: PlayMode::FromStart,
    });
    h.player.wait_call(|c| *c == Call::Stop).await;
    h.sink.send(PlayerEvent::Unloaded(UnloadReason::Stopped));
    wait_loads(&h, 2).await;
    let seen = h.file_loaded_events().await;
    assert!(seen.contains(&PlaybackEvent::SubtitleDelay(0)), "{seen:?}");
    assert!(seen.contains(&PlaybackEvent::AudioDelay(0)), "{seen:?}");
    let calls = h.player.calls();
    let second_load = calls
        .iter()
        .rposition(|call| matches!(call, Call::Load(_)))
        .unwrap();
    assert!(position_of(&calls, &Call::SetSubtitleDelay(0)) < second_load);
    assert!(position_of(&calls, &Call::SetAudioDelay(0)) < second_load);
}

#[tokio::test]
async fn a_quality_change_keeps_the_delays() {
    let mock = MockServer::start().await;
    mount_transcode_for(&mock, 1_500_000).await;
    mount_status(&mock, "DELETE", ENCODINGS, 204).await;
    let mut h = harness_on(mock, quiet()).await;
    h.start().await;
    h.handle.send(PlaybackCommand::SetSubtitleDelay(300));
    wait_event(&mut h.events, |e| *e == PlaybackEvent::SubtitleDelay(300)).await;

    h.handle.send(PlaybackCommand::SetQuality(LOW));
    h.wait_load(2).await;
    h.loaded_with(transcode_tracks()).await;
    let calls = h.player.calls();
    assert!(!calls.contains(&Call::SetSubtitleDelay(0)), "{calls:?}");
}

fn chapter(seconds: u64) -> Chapter {
    Chapter {
        start: Duration::from_secs(seconds),
        name: format!("At {seconds}"),
        image: None,
    }
}

#[test]
fn chapter_at_picks_last_started() {
    let chapters = [chapter(0), chapter(60), chapter(120)];
    assert_eq!(chapter_at(&chapters, Duration::from_secs(90)), Some(1));
    assert_eq!(chapter_at(&chapters, Duration::ZERO), Some(0));
    assert_eq!(chapter_at(&chapters, Duration::from_secs(60)), Some(1));
    assert_eq!(chapter_at(&chapters, Duration::from_secs(500)), Some(2));
    assert_eq!(chapter_at(&[], Duration::from_secs(90)), None);
    assert_eq!(
        chapter_at(&[chapter(10), chapter(20)], Duration::from_secs(5)),
        None
    );
}

#[test]
fn chapter_steps_follow_mpv() {
    let at = Duration::from_secs;
    let starts = [at(0), at(60), at(120)];
    assert_eq!(start_at(&starts, at(90)), Some(1));
    assert_eq!(start_at(&starts, at(60)), Some(1));
    assert_eq!(start_at(&[], at(90)), None);
    assert_eq!(next_chapter(&starts, at(70)), Some(2));
    assert_eq!(next_chapter(&starts, at(0)), Some(1));
    assert_eq!(next_chapter(&starts, at(130)), None);
    assert_eq!(previous_chapter(&starts, at(70)), Some(1));
    assert_eq!(previous_chapter(&starts, at(61)), Some(0));
    assert_eq!(
        previous_chapter(&starts, Duration::from_millis(62_900)),
        Some(0),
        "within the grace up to its end"
    );
    assert_eq!(previous_chapter(&starts, at(63)), Some(1));
    assert_eq!(previous_chapter(&starts, at(1)), Some(0));
    assert_eq!(previous_chapter(&[], at(70)), None);
}

async fn mount_item_with_chapters(mock: &MockServer, chapters: Value) {
    let mut item = fixture("item_movie");
    item["Chapters"] = chapters;
    Mock::given(method("GET"))
        .and(path(format!("/Items/{MOVIE_ID}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(item))
        .with_priority(1)
        .mount(mock)
        .await;
}

#[tokio::test]
async fn chapters_event_follows_ready() {
    let mock = MockServer::start().await;
    mount_item_with_chapters(
        &mock,
        json!([
            {"StartPositionTicks": 0, "Name": "Opening", "ImageTag": "tag0"},
            {"StartPositionTicks": 600_000_000, "Name": "", "ImageTag": null},
            {"StartPositionTicks": 1_200_000_000, "Name": "Credits", "ImageTag": "tag2"},
        ]),
    )
    .await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    let mut seen = h.file_loaded_events().await;
    seen.push(h.events.recv().await.unwrap());
    let [
        ..,
        PlaybackEvent::Ready { .. },
        PlaybackEvent::Chapters(chapters),
    ] = &seen[..]
    else {
        panic!("no chapters right after Ready: {seen:?}");
    };
    assert_eq!(chapters.len(), 3);
    assert_eq!(
        chapters[1].start,
        Duration::from_secs(600_000_000 / 10_000_000)
    );
    assert_eq!(chapters[0].name, "Opening");
    assert_eq!(chapters[1].name, "");
    assert_eq!(chapters[1].image, None);
    assert_eq!(
        chapters[2].image,
        Some(ImageRef {
            item_id: ItemId::from(MOVIE_ID),
            image_type: ImageType::Chapter,
            tag: "tag2".into(),
            index: Some(2),
        })
    );
}

#[tokio::test]
async fn single_chapter_sends_empty_list() {
    let mock = MockServer::start().await;
    mount_item_with_chapters(
        &mock,
        json!([{"StartPositionTicks": 0, "Name": "Whole", "ImageTag": "tag0"}]),
    )
    .await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    h.file_loaded().await;
    assert_eq!(
        h.events.recv().await.unwrap(),
        PlaybackEvent::Chapters(Vec::new())
    );
}

#[tokio::test]
async fn delay_steps_sent_in_a_row_add_up() {
    let mut h = harness(quiet()).await;
    h.start().await;
    h.handle.send(PlaybackCommand::AdjustSubtitleDelay(100));
    h.handle.send(PlaybackCommand::AdjustSubtitleDelay(100));
    h.handle.send(PlaybackCommand::AdjustAudioDelay(-100));
    h.handle.send(PlaybackCommand::AdjustAudioDelay(-100));
    let seen = events_until(&mut h.events, |e| *e == PlaybackEvent::AudioDelay(-200)).await;
    assert!(
        seen.contains(&PlaybackEvent::SubtitleDelay(100)),
        "{seen:?}"
    );
    assert!(
        seen.contains(&PlaybackEvent::SubtitleDelay(200)),
        "{seen:?}"
    );
    let calls = h.player.calls();
    assert!(calls.contains(&Call::SetSubtitleDelay(200)), "{calls:?}");
    assert!(calls.contains(&Call::SetAudioDelay(-200)), "{calls:?}");
}

#[tokio::test]
async fn request_stats_reports_method() {
    let mut h = harness(quiet()).await;
    h.player.set_stats(PlaybackStats {
        video_codec: Some("h264".into()),
        source_size: Some((1920, 1080)),
        ..PlaybackStats::default()
    });
    h.start().await;

    h.handle.send(PlaybackCommand::RequestStats);
    let PlaybackEvent::Stats(report) =
        wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Stats(_))).await
    else {
        unreachable!()
    };
    assert_eq!(report.method, PlayMethod::DirectPlay);
    assert!(report.transcode_reasons.is_empty());
    assert_eq!(report.player.source_size, Some((1920, 1080)));
    assert_eq!(report.player.video_codec.as_deref(), Some("h264"));
}

#[tokio::test]
async fn request_stats_reports_the_transcode_reasons() {
    let mock = MockServer::start().await;
    mount_playback_info(&mock, transcode_info()).await;
    let mut h = harness_on(mock, quiet()).await;
    h.play(PlayMode::FromStart).await;
    h.loaded_with(transcode_tracks()).await;

    h.handle.send(PlaybackCommand::RequestStats);
    let PlaybackEvent::Stats(report) =
        wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Stats(_))).await
    else {
        unreachable!()
    };
    assert_eq!(report.method, PlayMethod::Transcode);
    assert_eq!(
        report.transcode_reasons,
        vec!["ContainerBitrateExceedsLimit".to_owned()]
    );
}

#[tokio::test]
async fn request_stats_without_media_reports_nothing() {
    let mut h = harness(quiet()).await;
    h.handle.send(PlaybackCommand::RequestStats);
    h.handle.send(PlaybackCommand::Stop);
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Stopped)).await;
    assert!(
        !seen.iter().any(|e| matches!(e, PlaybackEvent::Stats(_))),
        "{seen:?}"
    );
}

fn prefs(change: impl FnOnce(&mut PlaybackPrefs)) -> PlaybackPrefs {
    let mut prefs = PlaybackPrefs::default();
    change(&mut prefs);
    prefs
}

#[test]
fn default_prefs_follow_the_spec() {
    let prefs = PlaybackPrefs::default();
    assert!(prefs.autoplay_next);
    assert_eq!(prefs.countdown, Duration::from_secs(10));
    assert!(!prefs.skip_intros);
    assert!(!prefs.skip_recaps);
    assert!(prefs.remember_tracks);
}

#[tokio::test]
async fn countdown_uses_configured_length() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    h.handle.send(PlaybackCommand::SetPrefs(prefs(|p| {
        p.countdown = Duration::from_secs(5);
    })));
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.end_at(Duration::from_secs(599));
    let first = wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::NextUpCountdown { .. })
    })
    .await;
    let PlaybackEvent::NextUpCountdown {
        episode,
        remaining,
        total,
    } = first
    else {
        unreachable!()
    };
    assert_eq!(episode.id.as_str(), EPISODES[2]);
    assert_eq!((remaining, total), (5, 5));
}

#[tokio::test]
async fn ask_mode_never_autoplays() {
    let mut h = harness(episode_timings(Duration::from_millis(20))).await;
    h.handle.send(PlaybackCommand::SetPrefs(prefs(|p| {
        p.autoplay_next = false;
    })));
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.end_at(Duration::from_secs(599));
    let seen = events_until(&mut h.events, |e| {
        matches!(e, PlaybackEvent::NextUpReady { .. })
    })
    .await;
    assert!(seen.contains(&PlaybackEvent::Ended), "{seen:?}");
    let Some(PlaybackEvent::NextUpReady { episode }) = seen.last() else {
        unreachable!()
    };
    assert_eq!(episode.id.as_str(), EPISODES[2]);
    // More than three steps of the countdown.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.loads().len(), 1);
    let mut later = Vec::new();
    while let Ok(event) = h.events.try_recv() {
        later.push(event);
    }
    assert!(countdowns(&later).is_empty(), "{later:?}");
    assert!(
        !later
            .iter()
            .any(|e| matches!(e, PlaybackEvent::Loading { .. })),
        "{later:?}"
    );

    h.handle.send(PlaybackCommand::PlayNextNow);
    wait_event(
        &mut h.events,
        |e| matches!(e, PlaybackEvent::Loading { item, .. } if *item == ItemId::from(EPISODES[2])),
    )
    .await;
    wait_loads(&h, 2).await;
}

#[tokio::test]
async fn ask_mode_close_cancels_the_offer() {
    let mut h = harness(episode_timings(Duration::from_millis(20))).await;
    h.handle.send(PlaybackCommand::SetPrefs(prefs(|p| {
        p.autoplay_next = false;
    })));
    h.start_item(EPISODES[1], TEN_MINUTES).await;
    h.end_at(Duration::from_secs(599));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::NextUpReady { .. })
    })
    .await;
    h.handle.send(PlaybackCommand::CancelNextUp);
    wait_event(&mut h.events, |e| *e == PlaybackEvent::NextUpCancelled).await;
    h.handle.send(PlaybackCommand::PlayNextNow);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.loads().len(), 1);
}

const SEGMENTED: &str = "/MediaSegments/76282b1c408f374beeee50519e9157a4";

/// Plays the movie with `segments` and `prefs`, loaded and waiting for its segments.
async fn segmented(segments: Vec<Value>, prefs: PlaybackPrefs) -> Harness {
    let mock = MockServer::start().await;
    mount_segments(&mock, MOVIE_ID, segments_body(segments)).await;
    let mut h = harness_on(mock, quiet()).await;
    h.handle.send(PlaybackCommand::SetPrefs(prefs));
    h.play(PlayMode::FromStart).await;
    h.file_loaded_events().await;
    wait_requests(&h.mock, SEGMENTED, 1).await;
    h
}

fn seeks_to(h: &Harness, target: Duration) -> usize {
    h.player
        .calls()
        .iter()
        .filter(|call| **call == Call::Seek(SeekTarget::Absolute(target)))
        .count()
}

/// Lands a seek of the player at `position`.
fn land(h: &Harness, position: Duration) {
    h.sink.send(PlayerEvent::Seeking);
    h.sink.send(PlayerEvent::Position(position));
    h.sink.send(PlayerEvent::Seeked);
}

fn skipped(seen: &[PlaybackEvent]) -> Vec<(SegmentKind, Duration)> {
    seen.iter()
        .filter_map(|e| match e {
            PlaybackEvent::SegmentSkipped { kind, from } => Some((*kind, *from)),
            _ => None,
        })
        .collect()
}

fn secs(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

#[tokio::test]
async fn auto_skip_intro_once() {
    let mut h = segmented(
        vec![segment_json("Intro", secs(5), secs(9))],
        prefs(|p| p.skip_intros = true),
    )
    .await;
    h.sink.send(PlayerEvent::Position(secs(1)));
    h.sink.send(PlayerEvent::Position(secs(5)));
    let seen = events_until(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SegmentSkipped { .. })
    })
    .await;
    assert_eq!(skipped(&seen), vec![(SegmentKind::Intro, secs(5))]);
    assert!(segment_events(&seen).is_empty(), "{seen:?}");
    h.player
        .wait_call(|c| *c == Call::Seek(SeekTarget::Absolute(secs(9))))
        .await;
    // A position reported before the seek lands neither skips again nor offers the button.
    h.sink
        .send(PlayerEvent::Position(Duration::from_millis(5_250)));
    land(&h, secs(9));
    wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;

    let target = SeekTarget::Absolute(secs(6));
    h.handle.send(PlaybackCommand::Seek(target));
    h.player.wait_call(|c| *c == Call::Seek(target)).await;
    land(&h, secs(6));
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;
    assert_eq!(segment_events(&seen), vec![Some(intro(5, 9))], "{seen:?}");
    let seen = h.stop().await;
    assert!(skipped(&seen).is_empty(), "{seen:?}");
    assert_eq!(seeks_to(&h, secs(9)), 1);
}

#[tokio::test]
async fn undo_skip_returns_and_disables() {
    let mut h = segmented(
        vec![segment_json("Intro", secs(5), secs(9))],
        prefs(|p| p.skip_intros = true),
    )
    .await;
    h.sink.send(PlayerEvent::Position(secs(5)));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SegmentSkipped { .. })
    })
    .await;
    land(&h, secs(9));
    wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;

    h.handle.send(PlaybackCommand::UndoSkip);
    h.player
        .wait_call(|c| *c == Call::Seek(SeekTarget::Absolute(secs(5))))
        .await;
    land(&h, secs(5));
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;
    assert_eq!(segment_events(&seen), vec![Some(intro(5, 9))], "{seen:?}");
    h.sink.send(PlayerEvent::Position(secs(6)));
    let seen = h.stop().await;
    assert!(skipped(&seen).is_empty(), "{seen:?}");
    assert_eq!(seeks_to(&h, secs(9)), 1);
}

#[tokio::test]
async fn a_seek_into_a_fresh_intro_is_not_skipped() {
    let mut h = segmented(
        vec![
            segment_json("Recap", secs(2), secs(4)),
            segment_json("Intro", secs(5), secs(9)),
        ],
        prefs(|p| {
            p.skip_intros = true;
            p.skip_recaps = true;
        }),
    )
    .await;
    h.sink.send(PlayerEvent::Position(secs(1)));
    let target = SeekTarget::Forward(secs(5));
    h.handle.send(PlaybackCommand::Seek(target));
    h.player.wait_call(|c| *c == Call::Seek(target)).await;
    land(&h, secs(6));
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;
    assert_eq!(segment_events(&seen), vec![Some(intro(5, 9))], "{seen:?}");

    let target = SeekTarget::Absolute(secs(3));
    h.handle.send(PlaybackCommand::Seek(target));
    h.player.wait_call(|c| *c == Call::Seek(target)).await;
    land(&h, secs(3));
    h.sink.send(PlayerEvent::Position(secs(7)));
    let seen = h.stop().await;
    assert!(skipped(&seen).is_empty(), "{seen:?}");
    assert_eq!(seeks_to(&h, secs(4)), 0);
    assert_eq!(seeks_to(&h, secs(9)), 0);
}

#[tokio::test]
async fn the_skip_button_keeps_the_next_segment_skipped() {
    let mut h = segmented(
        vec![
            segment_json("Recap", secs(0), secs(4)),
            segment_json("Intro", secs(4), secs(8)),
        ],
        prefs(|p| p.skip_intros = true),
    )
    .await;
    h.sink.send(PlayerEvent::Position(secs(1)));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::Segment(Some(segment)) if segment.kind == SegmentKind::Recap)
    })
    .await;
    h.handle.send(PlaybackCommand::SkipSegment);
    h.player
        .wait_call(|c| *c == Call::Seek(SeekTarget::Absolute(secs(4))))
        .await;
    land(&h, secs(4));
    let seen = events_until(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SegmentSkipped { .. })
    })
    .await;
    assert_eq!(skipped(&seen), vec![(SegmentKind::Intro, secs(4))]);
    h.player
        .wait_call(|c| *c == Call::Seek(SeekTarget::Absolute(secs(8))))
        .await;
}

#[tokio::test]
async fn undo_after_a_seek_does_nothing() {
    let mut h = segmented(
        vec![segment_json("Intro", secs(5), secs(9))],
        prefs(|p| p.skip_intros = true),
    )
    .await;
    h.sink.send(PlayerEvent::Position(secs(5)));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SegmentSkipped { .. })
    })
    .await;
    land(&h, secs(9));
    wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;

    let target = SeekTarget::Absolute(secs(20));
    h.handle.send(PlaybackCommand::Seek(target));
    h.player.wait_call(|c| *c == Call::Seek(target)).await;
    land(&h, secs(20));
    wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;
    h.handle.send(PlaybackCommand::UndoSkip);
    h.stop().await;
    assert_eq!(seeks_to(&h, secs(5)), 0);
}

#[tokio::test]
async fn undo_after_back_to_back_skips_returns_to_the_last_one() {
    let mut h = segmented(
        vec![
            segment_json("Recap", secs(0), secs(4)),
            segment_json("Intro", secs(4), secs(8)),
        ],
        prefs(|p| {
            p.skip_intros = true;
            p.skip_recaps = true;
        }),
    )
    .await;
    h.sink.send(PlayerEvent::Position(secs(1)));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SegmentSkipped { .. })
    })
    .await;
    land(&h, secs(4));
    let seen = events_until(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SegmentSkipped { .. })
    })
    .await;
    assert_eq!(skipped(&seen), vec![(SegmentKind::Intro, secs(4))]);
    land(&h, secs(8));
    wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;
    h.handle.send(PlaybackCommand::UndoSkip);
    h.player
        .wait_call(|c| *c == Call::Seek(SeekTarget::Absolute(secs(4))))
        .await;
    assert_eq!(seeks_to(&h, secs(1)), 0);
}

#[tokio::test]
async fn recap_flag_covers_preview() {
    let mut h = segmented(
        vec![
            segment_json("Preview", secs(2), secs(5)),
            segment_json("Intro", secs(6), secs(9)),
        ],
        prefs(|p| p.skip_recaps = true),
    )
    .await;
    h.sink.send(PlayerEvent::Position(secs(2)));
    wait_event(&mut h.events, |e| {
        *e == PlaybackEvent::SegmentSkipped {
            kind: SegmentKind::Preview,
            from: secs(2),
        }
    })
    .await;
    h.player
        .wait_call(|c| *c == Call::Seek(SeekTarget::Absolute(secs(5))))
        .await;
    land(&h, secs(6));
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;
    assert_eq!(segment_events(&seen), vec![Some(intro(6, 9))], "{seen:?}");
    assert!(skipped(&seen).is_empty(), "{seen:?}");
}

#[tokio::test]
async fn credits_never_auto_skipped() {
    let mut h = segmented(
        vec![segment_json("Outro", secs(2), secs(9))],
        prefs(|p| {
            p.skip_intros = true;
            p.skip_recaps = true;
        }),
    )
    .await;
    h.sink.send(PlayerEvent::Position(secs(3)));
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Segment(_))).await;
    assert_eq!(
        segment_events(&seen),
        vec![Some(SkippableSegment {
            kind: SegmentKind::Credits,
            start: secs(2),
            end: secs(9),
        })]
    );
    let seen = h.stop().await;
    assert!(skipped(&seen).is_empty(), "{seen:?}");
    assert!(
        !h.player.calls().iter().any(|c| matches!(c, Call::Seek(_))),
        "{:?}",
        h.player.calls()
    );
}

#[tokio::test]
async fn skip_button_mode_never_seeks() {
    let mut h = segmented(
        vec![segment_json("Intro", secs(2), secs(9))],
        PlaybackPrefs::default(),
    )
    .await;
    h.sink.send(PlayerEvent::Position(secs(3)));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::Segment(Some(_)))
    })
    .await;
    let seen = h.stop().await;
    assert!(skipped(&seen).is_empty(), "{seen:?}");
    assert!(!h.player.calls().iter().any(|c| matches!(c, Call::Seek(_))));
}

#[tokio::test]
async fn a_quality_reload_keeps_the_skips() {
    let mut h = segmented(
        vec![segment_json("Intro", secs(5), secs(9))],
        prefs(|p| p.skip_intros = true),
    )
    .await;
    h.sink.send(PlayerEvent::Position(secs(5)));
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SegmentSkipped { .. })
    })
    .await;
    land(&h, secs(9));
    wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;
    h.handle.send(PlaybackCommand::UndoSkip);
    h.player
        .wait_call(|c| *c == Call::Seek(SeekTarget::Absolute(secs(5))))
        .await;
    land(&h, secs(5));
    wait_event(&mut h.events, |e| matches!(e, PlaybackEvent::Seeked)).await;

    h.handle.send(PlaybackCommand::SetQuality(QUALITIES[3]));
    h.player.wait_call(|c| *c == Call::Stop).await;
    h.sink.send(PlayerEvent::Unloaded(UnloadReason::Stopped));
    wait_loads(&h, 2).await;
    h.file_loaded_events().await;
    h.sink.send(PlayerEvent::Position(secs(6)));
    let seen = h.stop().await;
    assert!(skipped(&seen).is_empty(), "{seen:?}");
    assert!(
        segment_events(&seen).contains(&Some(intro(5, 9))),
        "{seen:?}"
    );
    assert_eq!(seeks_to(&h, secs(9)), 1);
}

/// Playback info of an episode with English and French audio (1, 2) and
/// subtitles (3, 4): the server defaults are English audio, French subtitles.
fn two_languages() -> Value {
    let stream = |index: i32, kind: &str, language: Option<&str>| {
        json!({
            "Index": index,
            "Type": kind,
            "Language": language,
            "Channels": if kind == "Audio" { json!(2) } else { Value::Null },
            "IsForced": false,
            "IsExternal": false,
            "IsTextSubtitleStream": kind == "Subtitle",
        })
    };
    let mut info = fixture("playback_info");
    let source = &mut info["MediaSources"][0];
    source["MediaStreams"] = json!([
        stream(0, "Video", None),
        stream(1, "Audio", Some("eng")),
        stream(2, "Audio", Some("fre")),
        stream(3, "Subtitle", Some("eng")),
        stream(4, "Subtitle", Some("fre")),
    ]);
    source["DefaultAudioStreamIndex"] = json!(1);
    source["DefaultSubtitleStreamIndex"] = json!(4);
    info
}

/// The tracks mpv reports for [`two_languages`].
fn two_language_tracks() -> Vec<Track> {
    vec![
        track(1, TrackKind::Video, 0, true),
        track(1, TrackKind::Audio, 1, true),
        track(2, TrackKind::Audio, 2, false),
        track(1, TrackKind::Subtitle, 3, false),
        track(2, TrackKind::Subtitle, 4, false),
    ]
}

async fn mount_info_for(mock: &MockServer, id: &str, body: Value) {
    Mock::given(method("POST"))
        .and(path(format!("/Items/{id}/PlaybackInfo")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .with_priority(2)
        .mount(mock)
        .await;
}

/// French audio and no subtitle, remembered for [`SERIES_ID`] in a new data directory.
fn remembered_french() -> TempDir {
    let data = tempfile::tempdir().unwrap_or_else(|err| panic!("no temporary directory: {err}"));
    TrackMemory::load(&Paths::under(data.path()), ACCOUNT)
        .set(
            &ItemId::from(SERIES_ID),
            TrackChoice {
                audio: Some(AudioPick {
                    language: Some("fra".to_owned()),
                    channels: Some(1),
                }),
                subtitle: SubtitlePick::Off,
            },
        )
        .unwrap_or_else(|err| panic!("cannot seed the track memory: {err}"));
    data
}

impl Harness {
    /// Sends `command` and waits until the player receives `call` because of it.
    async fn pick(&self, command: PlaybackCommand, call: Call) {
        let before = self.player.calls().len();
        self.handle.send(command);
        let start = tokio::time::Instant::now();
        while !self.player.calls()[before..].contains(&call) {
            assert!(start.elapsed() < common::DEADLINE, "no {call:?}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Plays `id`, loads it with `tracks` and returns the player calls made from the load on.
    async fn load_episode(&mut self, id: &str, tracks: Vec<Track>) -> Vec<Call> {
        self.play_item(id, PlayMode::FromStart).await;
        let before = self.player.calls().len();
        self.loaded_with(tracks).await;
        self.player.calls()[before..].to_vec()
    }

    fn memory(&self) -> TrackMemory {
        TrackMemory::load(&Paths::under(self.data.path()), ACCOUNT)
    }
}

fn selected_audio(calls: &[Call]) -> Vec<Option<TrackId>> {
    calls
        .iter()
        .filter_map(|call| match call {
            Call::SelectAudio(id) => Some(*id),
            _ => None,
        })
        .collect()
}

fn selected_subtitles(calls: &[Call]) -> Vec<Option<TrackId>> {
    calls
        .iter()
        .filter_map(|call| match call {
            Call::SelectSubtitle(id) => Some(*id),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn user_pick_is_remembered_for_series() {
    let mock = MockServer::start().await;
    mount_info_for(&mock, EPISODES[1], two_languages()).await;
    let mut h = harness_on(mock, episode_timings(Duration::from_secs(3600))).await;
    h.start_item(EPISODES[0], TEN_MINUTES).await;
    h.pick(
        PlaybackCommand::SelectAudio(3),
        Call::SelectAudio(Some(TrackId(2))),
    )
    .await;
    h.pick(
        PlaybackCommand::SelectSubtitle(None),
        Call::SelectSubtitle(None),
    )
    .await;
    h.pick(
        PlaybackCommand::SelectSubtitle(Some(0)),
        Call::SelectSubtitle(Some(TrackId(1))),
    )
    .await;
    assert_eq!(
        h.memory().get(&ItemId::from(SERIES_ID)),
        Some(TrackChoice {
            audio: Some(AudioPick {
                language: Some("fra".to_owned()),
                channels: Some(1),
            }),
            subtitle: SubtitlePick::Track {
                language: Some("eng".to_owned()),
                forced: false,
                hearing_impaired: false,
                external: true,
            },
        })
    );

    let calls = h.load_episode(EPISODES[1], two_language_tracks()).await;
    assert_eq!(selected_audio(&calls), [Some(TrackId(2))], "{calls:?}");
    assert_eq!(selected_subtitles(&calls), [Some(TrackId(1))], "{calls:?}");
}

#[tokio::test]
async fn an_audio_pick_keeps_the_remembered_subtitle() {
    let mock = MockServer::start().await;
    mount_info_for(&mock, EPISODES[1], two_languages()).await;
    let mut h = harness_in(
        mock,
        episode_timings(Duration::from_secs(3600)),
        remembered_french(),
    )
    .await;
    h.start_item(EPISODES[0], TEN_MINUTES).await;
    h.pick(
        PlaybackCommand::SelectAudio(2),
        Call::SelectAudio(Some(TrackId(1))),
    )
    .await;
    let calls = h.load_episode(EPISODES[1], two_language_tracks()).await;
    assert_eq!(selected_audio(&calls), [Some(TrackId(1))], "{calls:?}");
    assert_eq!(selected_subtitles(&calls), [None], "{calls:?}");
}

#[tokio::test]
async fn picks_without_a_language_keep_the_memory() {
    let mut info = fixture("playback_info");
    let streams = &mut info["MediaSources"][0]["MediaStreams"];
    streams[0]["Language"] = Value::Null;
    streams[2]["Language"] = json!("und");
    let mock = MockServer::start().await;
    mount_info_for(&mock, EPISODES[0], info).await;
    let mut h = harness_in(
        mock,
        episode_timings(Duration::from_secs(3600)),
        remembered_french(),
    )
    .await;
    let remembered = h.memory().get(&ItemId::from(SERIES_ID));
    h.start_item(EPISODES[0], TEN_MINUTES).await;
    h.pick(
        PlaybackCommand::SelectAudio(2),
        Call::SelectAudio(Some(TrackId(1))),
    )
    .await;
    h.pick(
        PlaybackCommand::SelectSubtitle(Some(0)),
        Call::SelectSubtitle(Some(TrackId(1))),
    )
    .await;
    assert_eq!(h.memory().get(&ItemId::from(SERIES_ID)), remembered);
}

#[tokio::test]
async fn no_matching_language_keeps_the_server_default() {
    let mut info = two_languages();
    info["MediaSources"][0]["MediaStreams"][2]["Language"] = json!("jpn");
    let mock = MockServer::start().await;
    mount_info_for(&mock, EPISODES[1], info).await;
    let mut h = harness_on(mock, episode_timings(Duration::from_secs(3600))).await;
    h.start_item(EPISODES[0], TEN_MINUTES).await;
    h.pick(
        PlaybackCommand::SelectAudio(3),
        Call::SelectAudio(Some(TrackId(2))),
    )
    .await;
    let calls = h.load_episode(EPISODES[1], two_language_tracks()).await;
    assert_eq!(selected_audio(&calls), [Some(TrackId(1))], "{calls:?}");
    assert_eq!(selected_subtitles(&calls), [Some(TrackId(2))], "{calls:?}");
}

#[tokio::test]
async fn movies_are_not_remembered() {
    let mut h = harness(episode_timings(Duration::from_secs(3600))).await;
    h.start().await;
    h.pick(
        PlaybackCommand::SelectAudio(3),
        Call::SelectAudio(Some(TrackId(2))),
    )
    .await;
    let calls = h.load_episode(OTHER_ID, file_tracks()).await;
    assert_eq!(selected_audio(&calls), [Some(TrackId(1))], "{calls:?}");
    assert!(!h.data.path().join("data/track-memory.json").exists());
}

#[tokio::test]
async fn server_default_when_disabled() {
    let mock = MockServer::start().await;
    mount_info_for(&mock, EPISODES[1], two_languages()).await;
    let mut h = harness_in(
        mock,
        episode_timings(Duration::from_secs(3600)),
        remembered_french(),
    )
    .await;
    h.handle.send(PlaybackCommand::SetPrefs(prefs(|p| {
        p.remember_tracks = false;
    })));
    h.start_item(EPISODES[0], TEN_MINUTES).await;
    h.pick(
        PlaybackCommand::SelectSubtitle(Some(0)),
        Call::SelectSubtitle(Some(TrackId(1))),
    )
    .await;
    let calls = h.load_episode(EPISODES[1], two_language_tracks()).await;
    assert_eq!(selected_audio(&calls), [Some(TrackId(1))], "{calls:?}");
    assert_eq!(selected_subtitles(&calls), [Some(TrackId(2))], "{calls:?}");
    let kept = h.memory().get(&ItemId::from(SERIES_ID));
    assert_eq!(kept.map(|choice| choice.subtitle), Some(SubtitlePick::Off));
}

#[tokio::test]
async fn resume_choice_wins_over_memory() {
    let mut h = harness_in(
        MockServer::start().await,
        episode_timings(Duration::from_secs(3600)),
        remembered_french(),
    )
    .await;
    h.handle.send(PlaybackCommand::SetPrefs(prefs(|p| {
        p.remember_tracks = false;
    })));
    h.start_item(EPISODES[0], TEN_MINUTES).await;
    h.handle
        .send(PlaybackCommand::SetPrefs(PlaybackPrefs::default()));
    let before = h.player.calls().len();
    h.handle.send(PlaybackCommand::SetQuality(LOW));
    h.player.wait_call(|c| *c == Call::Stop).await;
    h.sink.send(PlayerEvent::Unloaded(UnloadReason::Stopped));
    h.wait_load(2).await;
    h.loaded_with(file_tracks()).await;
    let calls = h.player.calls()[before..].to_vec();
    assert_eq!(selected_audio(&calls), [Some(TrackId(1))], "{calls:?}");
    let subtitles = selected_subtitles(&calls);
    assert!(matches!(subtitles[..], [Some(_)]), "{calls:?}");
}

/// A transcode of episode `id` carrying the audio stream `audio`.
fn episode_transcode(id: &str, audio: i32) -> Value {
    let mut info = transcode_info();
    info["MediaSources"][0]["TranscodingUrl"] = json!(format!(
        "/videos/{id}/master.m3u8?MediaSourceId={id}&AudioStreamIndex={audio}&PlaySessionId={PLAY_SESSION_ID}&ApiKey={TEST_TOKEN}&TranscodeReasons=ContainerBitrateExceedsLimit"
    ));
    info
}

#[tokio::test]
async fn remembered_audio_is_asked_of_a_transcode() {
    let episode = EPISODES[0];
    let route = format!("/Items/{episode}/PlaybackInfo");
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(&route))
        .and(body_partial_json(json!({ "AudioStreamIndex": 3 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(episode_transcode(episode, 3)))
        .with_priority(1)
        .mount(&mock)
        .await;
    mount_info_for(&mock, episode, episode_transcode(episode, 2)).await;
    let mut h = harness_in(
        mock,
        episode_timings(Duration::from_secs(3600)),
        remembered_french(),
    )
    .await;
    h.play_item(episode, PlayMode::FromStart).await;
    let load = h.wait_load(1).await;
    assert!(
        load.location.contains("AudioStreamIndex=3"),
        "{}",
        load.location
    );
    let infos = bodies(&h.mock, &route).await;
    assert_eq!(infos.len(), 2);
    assert_eq!(infos[1]["MediaSourceId"], json!(MOVIE_ID));
    assert_eq!(infos[1]["AudioStreamIndex"], json!(3));
    let ready = h.loaded_with(transcode_tracks()).await;
    let PlaybackEvent::Ready { audio, .. } = ready else {
        unreachable!();
    };
    let audio: Vec<_> = audio.iter().map(|a| (a.index, a.selected)).collect();
    assert_eq!(audio, [(2, false), (3, true)]);
    assert_eq!(h.loads().len(), 1);
}

/// [`episode_transcode`] with a French image subtitle (4) burned into the picture.
fn burned_transcode(id: &str, audio: i32) -> Value {
    let mut info = episode_transcode(id, audio);
    let source = &mut info["MediaSources"][0];
    let url = source["TranscodingUrl"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    source["TranscodingUrl"] = json!(format!("{url}&SubtitleStreamIndex=4&SubtitleMethod=Encode"));
    source["DefaultSubtitleStreamIndex"] = json!(4);
    let streams = source["MediaStreams"].as_array_mut();
    streams.unwrap_or_else(|| panic!("no streams")).push(json!({
        "Index": 4,
        "Type": "Subtitle",
        "Codec": "PGSSUB",
        "Language": "fre",
        "IsExternal": false,
        "IsTextSubtitleStream": false,
        "DeliveryMethod": "Encode",
    }));
    info
}

#[tokio::test]
async fn remembered_subtitle_off_is_asked_of_a_burning_transcode() {
    let episode = EPISODES[0];
    let route = format!("/Items/{episode}/PlaybackInfo");
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(&route))
        .and(body_partial_json(json!({ "SubtitleStreamIndex": -1 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(episode_transcode(episode, 3)))
        .with_priority(1)
        .mount(&mock)
        .await;
    mount_info_for(&mock, episode, burned_transcode(episode, 3)).await;
    let mut h = harness_in(
        mock,
        episode_timings(Duration::from_secs(3600)),
        remembered_french(),
    )
    .await;
    h.play_item(episode, PlayMode::FromStart).await;
    let load = h.wait_load(1).await;
    assert!(
        !load.location.contains("SubtitleStreamIndex"),
        "{}",
        load.location
    );
    let infos = bodies(&h.mock, &route).await;
    assert_eq!(infos.len(), 2);
    assert!(
        infos[0].get("SubtitleStreamIndex").is_none(),
        "{:?}",
        infos[0]
    );
    assert_eq!(infos[1]["MediaSourceId"], json!(MOVIE_ID));
    assert_eq!(infos[1]["SubtitleStreamIndex"], json!(-1));
    assert_eq!(infos[1]["AudioStreamIndex"], json!(3));
    h.loaded_with(transcode_tracks()).await;
    assert_eq!(h.loads().len(), 1);
}

#[tokio::test]
async fn a_remembered_text_subtitle_replaces_a_burned_one() {
    let episode = EPISODES[0];
    let route = format!("/Items/{episode}/PlaybackInfo");
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(&route))
        .and(body_partial_json(json!({ "SubtitleStreamIndex": 0 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(episode_transcode(episode, 2)))
        .with_priority(1)
        .mount(&mock)
        .await;
    mount_info_for(&mock, episode, burned_transcode(episode, 2)).await;
    let data = tempfile::tempdir().unwrap_or_else(|err| panic!("no temporary directory: {err}"));
    TrackMemory::load(&Paths::under(data.path()), ACCOUNT)
        .set(
            &ItemId::from(SERIES_ID),
            TrackChoice {
                audio: None,
                subtitle: SubtitlePick::Track {
                    language: Some("eng".to_owned()),
                    forced: false,
                    hearing_impaired: false,
                    external: true,
                },
            },
        )
        .unwrap_or_else(|err| panic!("cannot seed the track memory: {err}"));
    let mut h = harness_in(mock, episode_timings(Duration::from_secs(3600)), data).await;
    h.play_item(episode, PlayMode::FromStart).await;
    h.wait_load(1).await;
    let infos = bodies(&h.mock, &route).await;
    assert_eq!(infos.len(), 2);
    assert_eq!(infos[1]["SubtitleStreamIndex"], json!(0));
    assert!(infos[1].get("AudioStreamIndex").is_none(), "{:?}", infos[1]);
}

#[tokio::test]
async fn a_transcode_with_the_remembered_streams_is_not_asked_again() {
    let episode = EPISODES[0];
    let route = format!("/Items/{episode}/PlaybackInfo");
    let mock = MockServer::start().await;
    mount_info_for(&mock, episode, episode_transcode(episode, 3)).await;
    let mut h = harness_in(
        mock,
        episode_timings(Duration::from_secs(3600)),
        remembered_french(),
    )
    .await;
    h.play_item(episode, PlayMode::FromStart).await;
    h.wait_load(1).await;
    assert_eq!(bodies(&h.mock, &route).await.len(), 1);
}

#[tokio::test]
async fn next_sent_before_a_remembered_audio_request_is_kept() {
    let episode = EPISODES[0];
    let route = format!("/Items/{episode}/PlaybackInfo");
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(&route))
        .and(body_partial_json(json!({ "AudioStreamIndex": 3 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(episode_transcode(episode, 3)))
        .with_priority(1)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path(&route))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(episode_transcode(episode, 2))
                .set_delay(Duration::from_millis(300)),
        )
        .with_priority(2)
        .mount(&mock)
        .await;
    let mut h = harness_in(
        mock,
        episode_timings(Duration::from_secs(3600)),
        remembered_french(),
    )
    .await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(episode),
        mode: PlayMode::FromStart,
    });
    wait_requests(&h.mock, &route, 1).await;
    h.handle.send(PlaybackCommand::Next);
    h.wait_load(1).await;
    assert_eq!(bodies(&h.mock, &route).await.len(), 2);
    h.loaded_with(transcode_tracks()).await;
    wait_loads(&h, 2).await;
    assert!(
        h.loaded_ids()[1].contains(EPISODES[1]),
        "{:?}",
        h.loaded_ids()
    );
}

/// The fixture playback info once the server lists a French subtitle file
/// downloaded next to the movie: it comes first, so every stream index moves up.
fn playback_info_with_french_file() -> Value {
    let mut info = fixture("playback_info");
    let source = &mut info["MediaSources"][0];
    let Some(streams) = source["MediaStreams"].as_array_mut() else {
        panic!("bad fixture");
    };
    for stream in streams.iter_mut() {
        let index = stream["Index"].as_i64().unwrap_or_default();
        stream["Index"] = json!(index + 1);
    }
    streams.insert(
        0,
        json!({
            "Index": 0,
            "Type": "Subtitle",
            "Codec": "subrip",
            "Language": "fre",
            "DisplayTitle": "French - SUBRIP - External",
            "IsExternal": true,
            "IsTextSubtitleStream": true,
            "Path": "/media/movies/Test Movie (2020)/Test Movie (2020).fr.srt",
        }),
    );
    source["DefaultAudioStreamIndex"] = json!(3);
    source["DefaultSubtitleStreamIndex"] = json!(1);
    info
}

/// The `TracksChanged` that follows a `ReloadSubtitles`, with the calls made to the player.
async fn reload_french(
    h: &mut Harness,
) -> (
    Vec<oxyfin_core::AudioOption>,
    Vec<oxyfin_core::SubtitleOption>,
) {
    h.handle.send(PlaybackCommand::ReloadSubtitles {
        select_language: "fre".to_owned(),
    });
    let event = wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::TracksChanged { .. })
    })
    .await;
    let PlaybackEvent::TracksChanged { audio, subtitles } = event else {
        unreachable!();
    };
    (audio, subtitles)
}

#[tokio::test]
async fn reload_adds_new_external_track() {
    let mut h = harness(quiet()).await;
    h.start().await;
    mount_playback_info(&h.mock, playback_info_with_french_file()).await;

    let (audio, subtitles) = reload_french(&mut h).await;
    let reloaded = wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SubtitlesReloaded { .. })
    })
    .await;
    assert_eq!(reloaded, PlaybackEvent::SubtitlesReloaded { added: true });

    let calls = h.player.calls();
    let added: Vec<_> = calls
        .iter()
        .filter_map(|call| match call {
            Call::AddExternal { url, language, .. } => Some((url.clone(), language.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(added.len(), 2, "{calls:?}");
    assert!(
        added[1].0.ends_with(&format!(
            "/Videos/{MOVIE_ID}/{MOVIE_ID}/Subtitles/0/Stream.srt"
        )),
        "{added:?}"
    );
    assert_eq!(added[1].1.as_deref(), Some("fre"));
    assert_eq!(calls.last(), Some(&Call::SelectSubtitle(Some(TrackId(2)))));

    let shown: Vec<_> = subtitles
        .iter()
        .map(|s| (s.index, s.language.as_deref(), s.external, s.selected))
        .collect();
    assert_eq!(
        shown,
        vec![(0, Some("fre"), true, true), (1, Some("eng"), true, false)]
    );
    let audio: Vec<_> = audio.iter().map(|a| (a.index, a.selected)).collect();
    assert_eq!(audio, vec![(3, true), (4, false)]);

    let progress = wait_requests(&h.mock, PROGRESS, 1).await;
    let last = &progress[progress.len() - 1];
    assert_eq!(last["SubtitleStreamIndex"], json!(0));
    assert_eq!(last["AudioStreamIndex"], json!(3));

    // The tracks mpv lists afterwards map to the new indexes.
    let mut tracks = file_tracks();
    tracks.push(external_track(false));
    let mut french = external_track(true);
    french.id = TrackId(2);
    tracks.push(french);
    h.sink.send(PlayerEvent::Tracks(tracks));
    h.handle.send(PlaybackCommand::SelectAudio(4));
    let call = h
        .player
        .wait_call(|call| *call == Call::SelectAudio(Some(TrackId(2))))
        .await;
    assert_eq!(call, Call::SelectAudio(Some(TrackId(2))));
    h.handle.send(PlaybackCommand::SelectSubtitle(Some(1)));
    h.player
        .wait_call(|call| *call == Call::SelectSubtitle(Some(TrackId(1))))
        .await;
}

#[tokio::test]
async fn reload_waits_for_the_server_to_list_the_new_subtitle() {
    let mut h = harness(PlaybackTimings {
        subtitle_retry: Duration::from_millis(20),
        ..quiet()
    })
    .await;
    h.start().await;
    Mock::given(method("POST"))
        .and(path(format!("/Items/{MOVIE_ID}/PlaybackInfo")))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("playback_info")))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&h.mock)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/Items/{MOVIE_ID}/PlaybackInfo")))
        .respond_with(ResponseTemplate::new(200).set_body_json(playback_info_with_french_file()))
        .with_priority(2)
        .mount(&h.mock)
        .await;

    let (_, subtitles) = reload_french(&mut h).await;

    assert!(subtitles.iter().any(|s| s.index == 0 && s.selected));
    assert_eq!(
        bodies(&h.mock, &format!("/Items/{MOVIE_ID}/PlaybackInfo"))
            .await
            .len(),
        4
    );
}

#[tokio::test]
async fn reload_without_a_new_subtitle_changes_nothing() {
    let mut h = harness(PlaybackTimings {
        subtitle_retry: Duration::from_millis(5),
        ..quiet()
    })
    .await;
    h.start().await;
    h.handle.send(PlaybackCommand::ReloadSubtitles {
        select_language: "fre".to_owned(),
    });
    let reloaded = wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SubtitlesReloaded { .. })
    })
    .await;
    assert_eq!(reloaded, PlaybackEvent::SubtitlesReloaded { added: false });
    let info = format!("/Items/{MOVIE_ID}/PlaybackInfo");
    let asked = bodies(&h.mock, &info).await.len();
    assert_eq!(asked, 2 + oxyfin_core::SUBTITLE_RETRIES as usize);
    let seen = h.stop().await;
    assert!(
        !seen.iter().any(|e| matches!(
            e,
            PlaybackEvent::TracksChanged { .. } | PlaybackEvent::Error(_)
        )),
        "{seen:?}"
    );
    let adds = h
        .player
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::AddExternal { .. }))
        .count();
    assert_eq!(adds, 1);
}

/// Asks for the French subtitle and waits for the answer.
async fn reloaded(h: &mut Harness) -> PlaybackEvent {
    wait_event(&mut h.events, |e| {
        matches!(e, PlaybackEvent::SubtitlesReloaded { .. })
    })
    .await
}

fn reload_subtitles(h: &Harness) {
    h.handle.send(PlaybackCommand::ReloadSubtitles {
        select_language: "fre".to_owned(),
    });
}

#[tokio::test]
async fn a_reload_of_the_item_keeps_the_subtitle_request() {
    let mut h = harness(PlaybackTimings {
        subtitle_retry: Duration::from_secs(60),
        ..quiet()
    })
    .await;
    h.start().await;
    reload_subtitles(&h);
    wait_requests(&h.mock, PLAYBACK_INFO, 2).await;
    mount_playback_info(&h.mock, playback_info_with_french_file()).await;
    h.handle.send(PlaybackCommand::SetQuality(LOW));
    h.wait_load(2).await;
    h.loaded_with(file_tracks()).await;

    assert_eq!(
        reloaded(&mut h).await,
        PlaybackEvent::SubtitlesReloaded { added: true }
    );
    // The reload added the French file (index 0) as track 2, the English one as track 3.
    assert_eq!(
        h.player.calls().last(),
        Some(&Call::SelectSubtitle(Some(TrackId(2))))
    );
}

#[tokio::test]
async fn a_subtitle_request_during_a_reload_waits_for_the_item() {
    let mut h = harness(quiet()).await;
    h.start().await;
    mount_playback_info(&h.mock, playback_info_with_french_file()).await;
    h.handle.send(PlaybackCommand::SetQuality(LOW));
    reload_subtitles(&h);
    h.wait_load(2).await;
    h.loaded_with(file_tracks()).await;

    assert_eq!(
        reloaded(&mut h).await,
        PlaybackEvent::SubtitlesReloaded { added: true }
    );
    assert_eq!(
        h.player.calls().last(),
        Some(&Call::SelectSubtitle(Some(TrackId(2))))
    );
}

#[tokio::test]
async fn stopping_answers_a_pending_subtitle_request() {
    let mut h = harness(PlaybackTimings {
        subtitle_retry: Duration::from_secs(60),
        ..quiet()
    })
    .await;
    h.start().await;
    reload_subtitles(&h);
    wait_requests(&h.mock, PLAYBACK_INFO, 2).await;
    h.handle.send(PlaybackCommand::Stop);
    assert_eq!(
        reloaded(&mut h).await,
        PlaybackEvent::SubtitlesReloaded { added: false }
    );
}

#[tokio::test]
async fn another_item_answers_a_pending_subtitle_request() {
    let mut h = harness(PlaybackTimings {
        subtitle_retry: Duration::from_secs(60),
        ..quiet()
    })
    .await;
    h.start().await;
    reload_subtitles(&h);
    wait_requests(&h.mock, PLAYBACK_INFO, 2).await;
    h.handle.send(PlaybackCommand::Play {
        item: ItemId::from(OTHER_ID),
        mode: PlayMode::FromStart,
    });
    assert_eq!(
        reloaded(&mut h).await,
        PlaybackEvent::SubtitlesReloaded { added: false }
    );
}

#[tokio::test]
async fn a_subtitle_request_with_nothing_playing_is_answered() {
    let mut h = harness(quiet()).await;
    reload_subtitles(&h);
    assert_eq!(
        reloaded(&mut h).await,
        PlaybackEvent::SubtitlesReloaded { added: false }
    );
}

#[tokio::test]
async fn a_file_downloaded_again_in_the_language_replaces_its_track() {
    let mock = MockServer::start().await;
    mount_playback_info(&mock, playback_info_with_french_file()).await;
    let mut h = harness_on(
        mock,
        PlaybackTimings {
            subtitle_retry: Duration::from_millis(5),
            ..quiet()
        },
    )
    .await;
    h.start().await;
    // The French file (index 0) is track 1, the English one track 2.
    let calls_before = h.player.calls().len();
    reload_subtitles(&h);

    assert_eq!(
        reloaded(&mut h).await,
        PlaybackEvent::SubtitlesReloaded { added: true }
    );
    let calls = h.player.calls()[calls_before..].to_vec();
    let french = format!("/Videos/{MOVIE_ID}/{MOVIE_ID}/Subtitles/0/Stream.srt");
    let removed = position_of(&calls, &Call::RemoveSubtitle(TrackId(1)));
    let added = calls
        .iter()
        .position(|call| matches!(call, Call::AddExternal { url, .. } if url.ends_with(&french)))
        .unwrap_or_else(|| panic!("the French file is not added again: {calls:?}"));
    assert!(removed < added, "{calls:?}");
    assert_eq!(calls.last(), Some(&Call::SelectSubtitle(Some(TrackId(3)))));
}
