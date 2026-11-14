//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use oxyfin_api::{ClientInfo, Server, Session, UserId};
use oxyfin_core::{
    CoreError, MediaPlayer, MemoryStore, Paths, PlaybackEvent, Result, SecretStore, SessionManager,
    TokenVault,
};
use oxyfin_player::{MediaSource, PlayerError, SeekTarget, TrackId};
use serde_json::Value;
use tempfile::TempDir;
use tokio::sync::mpsc::UnboundedReceiver;
use wiremock::MockServer;

/// Token used by [`session`].
pub const TEST_TOKEN: &str = "test-token";

/// User id used by [`session`].
pub const TEST_USER_ID: &str = "0123456789abcdef0123456789abcdef";

/// Loads `tests/fixtures/<name>.json`.
pub fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("bad JSON in {}: {err}", path.display()))
}

/// A session restored on `mock` with [`TEST_TOKEN`] and [`TEST_USER_ID`].
pub fn session(mock: &MockServer) -> Session {
    Server::new(
        &mock.uri(),
        ClientInfo::new("Test Device", "test-device-id"),
    )
    .unwrap_or_else(|err| panic!("cannot create a server for the mock: {err}"))
    .restore_session(TEST_TOKEN, UserId::from(TEST_USER_ID), None)
}

/// A vault whose primary and fallback stores are fresh in-memory stores.
pub fn memory_vault() -> TokenVault {
    TokenVault::new(
        Arc::new(MemoryStore::default()),
        Arc::new(MemoryStore::default()),
    )
}

/// A manager storing its files under `dir`. The mock is not used: servers come from `connect`.
pub fn manager(_mock: &MockServer, dir: &TempDir, vault: TokenVault) -> SessionManager {
    SessionManager::new(Paths::under(dir.path()), vault, "Test Device")
        .map(|manager| manager.with_quick_connect_interval(std::time::Duration::from_millis(20)))
        .unwrap_or_else(|err| panic!("cannot create the session manager: {err}"))
}

/// A store whose every operation fails.
pub struct FailingStore;

impl SecretStore for FailingStore {
    fn get(&self, _: &str) -> Result<Option<String>> {
        Err(CoreError::Storage("unavailable".into()))
    }
    fn set(&self, _: &str, _: &str) -> Result<()> {
        Err(CoreError::Storage("unavailable".into()))
    }
    fn delete(&self, _: &str) -> Result<()> {
        Err(CoreError::Storage("unavailable".into()))
    }
}

/// How long the wait helpers below wait before failing the test.
pub const DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// A call received by [`FakePlayer`].
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    /// `load`.
    Load(MediaSource),
    /// `stop`.
    Stop,
    /// `set_paused`.
    SetPaused(bool),
    /// `seek`.
    Seek(SeekTarget),
    /// `set_volume`.
    SetVolume(f64),
    /// `set_muted`.
    SetMuted(bool),
    /// `set_speed`.
    SetSpeed(f64),
    /// `select_audio`.
    SelectAudio(Option<TrackId>),
    /// `select_subtitle`.
    SelectSubtitle(Option<TrackId>),
    /// `add_external_subtitle`.
    AddExternal {
        /// URL of the subtitle file.
        url: String,
        /// Track title.
        title: Option<String>,
        /// Track language.
        language: Option<String>,
    },
}

/// Pause, speed and volume of a [`FakePlayer`]. Like mpv, a load keeps them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerState {
    /// Last pause state set.
    pub paused: bool,
    /// Last speed set.
    pub speed: f64,
    /// Last volume set.
    pub volume: f64,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            paused: false,
            speed: 1.0,
            volume: 100.0,
        }
    }
}

/// A player that records the calls it receives.
#[derive(Default)]
pub struct FakePlayer {
    calls: Mutex<Vec<Call>>,
    state: Mutex<PlayerState>,
    next_subtitle: AtomicI64,
    no_media: AtomicBool,
    fail_load: AtomicBool,
}

impl FakePlayer {
    /// Makes every call but `load` fail with `NoMediaLoaded`, as before `FileLoaded`.
    pub fn set_no_media(&self, no_media: bool) {
        self.no_media.store(no_media, Ordering::SeqCst);
    }

    /// Makes `load` fail.
    pub fn set_fail_load(&self, fail: bool) {
        self.fail_load.store(fail, Ordering::SeqCst);
    }

    /// The calls received so far.
    pub fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// The pause, speed and volume set so far, kept across loads.
    pub fn state(&self) -> PlayerState {
        *self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update(&self, change: impl FnOnce(&mut PlayerState)) {
        change(
            &mut self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
    }

    /// Waits until a received call matches `pred` and returns it.
    pub async fn wait_call(&self, pred: impl Fn(&Call) -> bool) -> Call {
        let start = tokio::time::Instant::now();
        loop {
            if let Some(call) = self.calls().into_iter().find(&pred) {
                return call;
            }
            assert!(
                start.elapsed() < DEADLINE,
                "no matching call in {:?}",
                self.calls()
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    fn record(&self, call: Call) -> std::result::Result<(), PlayerError> {
        let is_load = matches!(call, Call::Load(_));
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(call);
        if is_load {
            if self.fail_load.load(Ordering::SeqCst) {
                return Err(PlayerError::Command {
                    name: "loadfile".into(),
                    message: "loading failed".into(),
                });
            }
            return Ok(());
        }
        if self.no_media.load(Ordering::SeqCst) {
            return Err(PlayerError::NoMediaLoaded);
        }
        Ok(())
    }
}

impl MediaPlayer for FakePlayer {
    fn load(&self, source: &MediaSource) -> std::result::Result<(), PlayerError> {
        self.record(Call::Load(source.clone()))
    }
    fn stop(&self) -> std::result::Result<(), PlayerError> {
        self.record(Call::Stop)
    }
    fn set_paused(&self, paused: bool) -> std::result::Result<(), PlayerError> {
        self.record(Call::SetPaused(paused))?;
        self.update(|state| state.paused = paused);
        Ok(())
    }
    fn seek(&self, target: SeekTarget) -> std::result::Result<(), PlayerError> {
        self.record(Call::Seek(target))
    }
    fn set_volume(&self, volume: f64) -> std::result::Result<(), PlayerError> {
        self.record(Call::SetVolume(volume))?;
        self.update(|state| state.volume = volume);
        Ok(())
    }
    fn set_muted(&self, muted: bool) -> std::result::Result<(), PlayerError> {
        self.record(Call::SetMuted(muted))
    }
    fn set_speed(&self, speed: f64) -> std::result::Result<(), PlayerError> {
        self.record(Call::SetSpeed(speed))?;
        self.update(|state| state.speed = speed);
        Ok(())
    }
    fn select_audio(&self, track: Option<TrackId>) -> std::result::Result<(), PlayerError> {
        self.record(Call::SelectAudio(track))
    }
    fn select_subtitle(&self, track: Option<TrackId>) -> std::result::Result<(), PlayerError> {
        self.record(Call::SelectSubtitle(track))
    }
    fn add_external_subtitle(
        &self,
        url: &str,
        title: Option<&str>,
        language: Option<&str>,
    ) -> std::result::Result<TrackId, PlayerError> {
        self.record(Call::AddExternal {
            url: url.to_owned(),
            title: title.map(str::to_owned),
            language: language.map(str::to_owned),
        })?;
        Ok(TrackId(
            self.next_subtitle.fetch_add(1, Ordering::SeqCst) + 1,
        ))
    }
}

/// Receives events until one matches `pred`; returns every event received, the match last.
pub async fn events_until(
    events: &mut UnboundedReceiver<PlaybackEvent>,
    pred: impl Fn(&PlaybackEvent) -> bool,
) -> Vec<PlaybackEvent> {
    let mut seen = Vec::new();
    let result = tokio::time::timeout(DEADLINE, async {
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
        matches!(result, Ok(true)),
        "no matching event, received {seen:?}"
    );
    seen
}

/// Receives events until one matches `pred` and returns it.
pub async fn wait_event(
    events: &mut UnboundedReceiver<PlaybackEvent>,
    pred: impl Fn(&PlaybackEvent) -> bool,
) -> PlaybackEvent {
    let mut seen = events_until(events, pred).await;
    seen.pop()
        .unwrap_or_else(|| panic!("events_until returned nothing"))
}

/// JSON bodies of the requests received on `route`, in order.
pub async fn bodies(mock: &MockServer, route: &str) -> Vec<Value> {
    mock.received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|request| request.url.path() == route)
        .map(|request| serde_json::from_slice(&request.body).unwrap_or(Value::Null))
        .collect()
}

/// Waits until `route` received at least `count` requests and returns their JSON bodies.
pub async fn wait_requests(mock: &MockServer, route: &str, count: usize) -> Vec<Value> {
    let start = tokio::time::Instant::now();
    loop {
        let found = bodies(mock, route).await;
        if found.len() >= count {
            return found;
        }
        assert!(
            start.elapsed() < DEADLINE,
            "{route} received {} requests, expected {count}",
            found.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}
