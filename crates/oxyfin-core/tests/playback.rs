//! Integration tests for the playback controller, with a fake player.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    Call, FakePlayer, TEST_TOKEN, bodies, events_until, fixture, session, wait_event, wait_requests,
};
use oxyfin_api::ItemId;
use oxyfin_core::{
    PROGRESS_INTERVAL, PlayMode, PlaybackCommand, PlaybackController, PlaybackError, PlaybackEvent,
    PlaybackHandle, PlaybackTimings, PlayerEventSink, SHUTDOWN_TIMEOUT, UNLOAD_GRACE,
    player_events,
};
use oxyfin_player::{PlayerEvent, SeekTarget, Track, TrackId, TrackKind, UnloadReason};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;
use wiremock::matchers::{method, path};
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
    );
    Harness {
        mock,
        player,
        sink,
        handle,
        events,
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
async fn no_direct_play_source_is_an_error() {
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
        *e == PlaybackEvent::Error(PlaybackError::NoDirectPlaySource)
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
        *e == PlaybackEvent::Error(PlaybackError::NoDirectPlaySource)
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
    h.handle.send(PlaybackCommand::Stop);
    let seen = events_until(&mut h.events, |e| matches!(e, PlaybackEvent::Stopped)).await;
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
            PlaybackEvent::NextUpCountdown { episode, remaining } => {
                Some((episode.id.to_string(), *remaining))
            }
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
