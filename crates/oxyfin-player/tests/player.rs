//! Integration tests for the mpv-backed player.

#![cfg(feature = "mpv")]
#![allow(clippy::unwrap_used)]

mod support;

use std::time::{Duration, Instant};

use oxyfin_player::{MediaSource, PlayerEvent, UnloadReason};
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
