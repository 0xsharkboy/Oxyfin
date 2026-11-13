//! Integration tests for the mpv-backed player.

#![cfg(feature = "mpv")]
#![allow(clippy::unwrap_used)]

mod support;

use std::time::{Duration, Instant};

use oxyfin_player::{MediaSource, PlayerError, PlayerEvent, SeekTarget, UnloadReason};
use support::{Events, fixture, headless};

const TIMEOUT: Duration = Duration::from_secs(10);

fn sample() -> MediaSource {
    MediaSource::new(fixture("sample.mkv").to_string_lossy())
}

fn is_file_loaded(event: &PlayerEvent) -> bool {
    matches!(event, PlayerEvent::FileLoaded { .. })
}

fn is_unload_error(event: &PlayerEvent) -> bool {
    matches!(event, PlayerEvent::Unloaded(UnloadReason::Error(_)))
}

#[test]
fn creates_and_drops_a_headless_player() {
    let (_events, on_event) = Events::new();
    drop(headless(on_event));
}

#[test]
fn loading_a_file_reports_file_loaded() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);
    player.load(&sample()).unwrap();

    let PlayerEvent::FileLoaded { duration, tracks } = events.wait_for(TIMEOUT, is_file_loaded)
    else {
        unreachable!()
    };
    let duration = duration.unwrap().as_secs_f64();
    assert!((2.9..=3.1).contains(&duration), "duration {duration}");
    assert_eq!(tracks.len(), 4, "tracks {tracks:#?}");
}

#[test]
fn playing_to_the_end_reports_ended_and_keeps_the_media() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);
    player.load(&sample()).unwrap();

    events.wait_for(TIMEOUT, |e| *e == PlayerEvent::Ended);
    player.stop().unwrap();
    events.wait_for(TIMEOUT, |e| {
        *e == PlayerEvent::Unloaded(UnloadReason::Stopped)
    });
}

#[test]
fn an_unreadable_file_reports_an_error() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);
    let manifest = format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
    player.load(&MediaSource::new(manifest)).unwrap();

    events.wait_for(TIMEOUT, is_unload_error);
    assert!(!events.drain().iter().any(is_file_loaded));
}

#[test]
fn an_unreachable_server_reports_an_error() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);
    player
        .load(&MediaSource::new("http://127.0.0.1:1/missing.mkv"))
        .unwrap();

    events.wait_for(TIMEOUT, is_unload_error);
}

#[test]
fn loading_again_reports_replaced() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);
    player.load(&sample()).unwrap();
    events.wait_for(TIMEOUT, is_file_loaded);

    player.load(&sample()).unwrap();
    events.wait_for(TIMEOUT, |e| {
        *e == PlayerEvent::Unloaded(UnloadReason::Replaced)
    });
    events.wait_for(TIMEOUT, is_file_loaded);
}

#[test]
fn loading_twice_quickly_ends_with_one_file_loaded() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);
    player.load(&sample()).unwrap();
    player.load(&sample()).unwrap();

    events.wait_for(TIMEOUT, is_file_loaded);
    let unexpected = events.drain().into_iter().find(|e| {
        matches!(
            e,
            PlayerEvent::Ended | PlayerEvent::Unloaded(UnloadReason::Stopped)
        )
    });
    assert_eq!(unexpected, None);
}

#[test]
fn a_path_with_spaces_and_commas_loads() {
    let dir = std::env::temp_dir().join(format!("oxyfin-player-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("my, video file.mkv");
    std::fs::copy(fixture("sample.mkv"), &path).unwrap();

    let (events, on_event) = Events::new();
    let player = headless(on_event);
    player
        .load(&MediaSource::new(path.to_string_lossy()))
        .unwrap();
    events.wait_for(TIMEOUT, is_file_loaded);

    drop(player);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn dropping_the_player_returns_promptly() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);
    player.load(&sample()).unwrap();
    events.wait_for(TIMEOUT, is_file_loaded);

    let started = Instant::now();
    drop(player);
    assert!(started.elapsed() < Duration::from_secs(2));
}

fn position_secs(event: &PlayerEvent) -> Option<f64> {
    match event {
        PlayerEvent::Position(position) => Some(position.as_secs_f64()),
        _ => None,
    }
}

#[test]
fn start_position_is_honoured() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);
    player
        .load(&sample().with_start(Duration::from_secs(2)))
        .unwrap();
    events.wait_for(TIMEOUT, is_file_loaded);

    let first = events.wait_for(TIMEOUT, |e| position_secs(e).is_some());
    let secs = position_secs(&first).unwrap();
    assert!(secs >= 1.9, "first position {secs}");
}

#[test]
fn pause_is_reported() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);
    player.load(&sample()).unwrap();
    events.wait_for(TIMEOUT, is_file_loaded);

    player.set_paused(true).unwrap();
    events.wait_for(TIMEOUT, |e| *e == PlayerEvent::Paused(true));
    std::thread::sleep(Duration::from_millis(300));
    let settled = events.drain().len();
    std::thread::sleep(Duration::from_secs(1));
    let late = events.drain().split_off(settled);
    assert!(
        !late.iter().any(|e| position_secs(e).is_some()),
        "positions while paused: {late:#?}"
    );
}

#[test]
fn absolute_and_relative_seeks_move_the_position() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);
    player.load(&sample()).unwrap();
    events.wait_for(TIMEOUT, is_file_loaded);
    player.set_paused(true).unwrap();
    events.wait_for(TIMEOUT, |e| *e == PlayerEvent::Paused(true));

    player
        .seek(SeekTarget::Absolute(Duration::from_secs(2)))
        .unwrap();
    events.wait_for(TIMEOUT, |e| *e == PlayerEvent::Seeking);
    events.wait_for(TIMEOUT, |e| *e == PlayerEvent::Seeked);
    events.wait_for(TIMEOUT, |e| {
        position_secs(e).is_some_and(|s| (s - 2.0).abs() <= 0.15)
    });

    player
        .seek(SeekTarget::Backward(Duration::from_secs(1)))
        .unwrap();
    events.wait_for(TIMEOUT, |e| *e == PlayerEvent::Seeked);
    events.wait_for(TIMEOUT, |e| {
        position_secs(e).is_some_and(|s| (s - 1.0).abs() <= 0.15)
    });
}

#[test]
fn seeking_past_the_end_reports_ended() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);
    player.load(&sample()).unwrap();
    events.wait_for(TIMEOUT, is_file_loaded);

    player
        .seek(SeekTarget::Absolute(Duration::from_secs(10)))
        .unwrap();
    events.wait_for(TIMEOUT, |e| *e == PlayerEvent::Ended);

    player.seek(SeekTarget::Absolute(Duration::ZERO)).unwrap();
    events.wait_for(TIMEOUT, |e| *e == PlayerEvent::Seeked);
    events.wait_for(TIMEOUT, |e| position_secs(e).is_some_and(|s| s > 0.2));
}

#[test]
fn seek_right_after_load_never_panics() {
    let (_events, on_event) = Events::new();
    let player = headless(on_event);
    player.load(&sample()).unwrap();
    let result = player.seek(SeekTarget::Absolute(Duration::from_secs(1)));
    assert!(
        matches!(result, Ok(()) | Err(PlayerError::NoMediaLoaded)),
        "{result:?}"
    );
}

#[test]
fn seek_without_media_fails() {
    let (_events, on_event) = Events::new();
    let player = headless(on_event);
    let result = player.seek(SeekTarget::Absolute(Duration::from_secs(1)));
    assert!(matches!(result, Err(PlayerError::NoMediaLoaded)));
}

#[test]
fn volume_and_mute_are_reported() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);

    player.set_volume(150.0).unwrap();
    events.wait_for(TIMEOUT, |e| *e == PlayerEvent::Volume(100.0));
    player.set_volume(40.0).unwrap();
    events.wait_for(TIMEOUT, |e| *e == PlayerEvent::Volume(40.0));
    player.set_muted(true).unwrap();
    events.wait_for(TIMEOUT, |e| *e == PlayerEvent::Muted(true));
}

#[test]
fn stop_then_load_reports_stopped_then_loads() {
    let (events, on_event) = Events::new();
    let player = headless(on_event);
    player.load(&sample()).unwrap();
    events.wait_for(TIMEOUT, is_file_loaded);

    player.stop().unwrap();
    events.wait_for(TIMEOUT, |e| {
        *e == PlayerEvent::Unloaded(UnloadReason::Stopped)
    });
    player.load(&sample()).unwrap();
    events.wait_for(TIMEOUT, is_file_loaded);
}
