//! The next episode offered at the end of an episode: counted down to, or
//! waiting for the user.

use std::time::Duration;

use tokio::time::Instant;

use super::types::PlaybackEvent;
use crate::models::Episode;

/// The next episode offered at the end of an episode. With a tick, it starts
/// after one step per tick, from the length of the countdown down to 1;
/// without one, it waits for the user.
pub(super) struct Countdown {
    episode: Episode,
    remaining: u32,
    total: u32,
    tick: Duration,
    /// When the next step happens; `None` while waiting for the user or held.
    pub(super) deadline: Option<Instant>,
    /// Time left before the next step while playback is paused.
    held: Option<Duration>,
}

impl Countdown {
    /// Starts a countdown of `length` to `episode`; returns it with its first event.
    pub(super) fn start(
        episode: Episode,
        length: Duration,
        tick: Duration,
    ) -> (Self, PlaybackEvent) {
        let total = u32::try_from(length.as_secs()).unwrap_or(u32::MAX).max(1);
        let countdown = Self {
            episode,
            remaining: total,
            total,
            tick,
            deadline: Some(Instant::now() + tick),
            held: None,
        };
        let event = countdown.event();
        (countdown, event)
    }

    /// Offers `episode` without counting down; returns the offer with its event.
    pub(super) fn wait(episode: Episode) -> (Self, PlaybackEvent) {
        let event = PlaybackEvent::NextUpReady {
            episode: Box::new(episode.clone()),
        };
        let countdown = Self {
            episode,
            remaining: 0,
            total: 0,
            tick: Duration::ZERO,
            deadline: None,
            held: None,
        };
        (countdown, event)
    }

    /// Moves one step; returns the event to emit, or `None` when the episode
    /// should start.
    pub(super) fn step(&mut self) -> Option<PlaybackEvent> {
        self.remaining = self.remaining.saturating_sub(1);
        if self.remaining == 0 {
            return None;
        }
        self.deadline = self.deadline.map(|deadline| deadline + self.tick);
        Some(self.event())
    }

    /// Stops counting until [`Countdown::resume`], keeping the time left.
    pub(super) fn hold(&mut self) {
        if let Some(deadline) = self.deadline.take() {
            self.held = Some(deadline.saturating_duration_since(Instant::now()));
        }
    }

    /// Counts again from where [`Countdown::hold`] stopped.
    pub(super) fn resume(&mut self) {
        if let Some(left) = self.held.take() {
            self.deadline = Some(Instant::now() + left);
        }
    }

    /// The episode offered.
    pub(super) fn into_episode(self) -> Episode {
        self.episode
    }

    fn event(&self) -> PlaybackEvent {
        PlaybackEvent::NextUpCountdown {
            episode: Box::new(self.episode.clone()),
            remaining: self.remaining,
            total: self.total,
        }
    }
}
