//! Shared helpers for the player integration tests.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use oxyfin_player::{Player, PlayerEvent, PlayerOptions, VideoOutput};

/// Collects the events emitted by a player.
pub struct Events {
    receiver: Receiver<PlayerEvent>,
    seen: Mutex<Vec<PlayerEvent>>,
}

impl Events {
    /// Creates a collector and the callback to hand to [`Player::new`].
    pub fn new() -> (Self, impl Fn(PlayerEvent) + Send + 'static) {
        let (sender, receiver) = mpsc::channel();
        let events = Self {
            receiver,
            seen: Mutex::new(Vec::new()),
        };
        let callback = move |event| {
            let _ = sender.send(event);
        };
        (events, callback)
    }

    /// Waits for the first event matching `pred`, panicking with every event
    /// received so far if `timeout` expires.
    pub fn wait_for(&self, timeout: Duration, pred: impl Fn(&PlayerEvent) -> bool) -> PlayerEvent {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.receiver.recv_timeout(remaining) {
                Ok(event) => {
                    self.seen.lock().unwrap().push(event.clone());
                    if pred(&event) {
                        return event;
                    }
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => panic!(
                    "no matching event within {timeout:?}; received: {:#?}",
                    self.seen.lock().unwrap()
                ),
            }
        }
    }

    /// Returns every event received so far, including those still queued.
    pub fn drain(&self) -> Vec<PlayerEvent> {
        let mut seen = self.seen.lock().unwrap();
        seen.extend(self.receiver.try_iter());
        seen.clone()
    }
}

/// Returns the path of a file in `tests/fixtures`.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Creates a player with no video and no audio output.
pub fn headless(on_event: impl Fn(PlayerEvent) + Send + 'static) -> Player {
    Player::new(
        PlayerOptions {
            video_output: VideoOutput::None,
        },
        on_event,
    )
    .unwrap()
}
