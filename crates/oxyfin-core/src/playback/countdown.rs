//! The countdown before the next episode starts.

use std::time::Duration;

use tokio::time::Instant;

use super::types::{NEXT_UP_COUNTDOWN, PlaybackEvent};
use crate::models::Episode;

/// A running countdown: one step per tick, from [`NEXT_UP_COUNTDOWN`] seconds down to 1.
pub(super) struct Countdown {
    episode: Episode,
    remaining: u32,
    tick: Duration,
    pub(super) deadline: Instant,
}

impl Countdown {
    /// Starts a countdown to `episode`; returns it with its first event.
    pub(super) fn start(episode: Episode, tick: Duration) -> (Self, PlaybackEvent) {
        let remaining = u32::try_from(NEXT_UP_COUNTDOWN.as_secs()).unwrap_or(u32::MAX);
        let countdown = Self {
            episode,
            remaining,
            tick,
            deadline: Instant::now() + tick,
        };
        let event = countdown.event();
        (countdown, event)
    }

    /// Moves one step; returns the event to emit, or `None` when the episode
    /// should start.
    pub(super) fn step(&mut self) -> Option<PlaybackEvent> {
        self.remaining = self.remaining.saturating_sub(1);
        if self.remaining == 0 {
            return None;
        }
        self.deadline += self.tick;
        Some(self.event())
    }

    /// The episode counted down to.
    pub(super) fn into_episode(self) -> Episode {
        self.episode
    }

    fn event(&self) -> PlaybackEvent {
        PlaybackEvent::NextUpCountdown {
            episode: Box::new(self.episode.clone()),
            remaining: self.remaining,
        }
    }
}
