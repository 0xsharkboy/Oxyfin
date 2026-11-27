//! Waits of the player for data, to tell when the stream is too slow for the chosen quality.

use std::time::Duration;

use tokio::time::Instant;

use super::quality::{QUALITIES, Quality};

/// The waits of one load: each starts when the player stops for data and ends when it plays on.
#[derive(Debug, Default)]
pub(super) struct Stalls {
    waits: Vec<(Instant, Option<Instant>)>,
}

impl Stalls {
    /// The player stopped for data.
    pub(super) fn start(&mut self, now: Instant) {
        if !self.waiting() {
            self.waits.push((now, None));
        }
    }

    /// The player plays on, or stopped waiting for another reason.
    pub(super) fn stop(&mut self, now: Instant) {
        if let Some((_, end @ None)) = self.waits.last_mut() {
            *end = Some(now);
        }
    }

    fn waiting(&self) -> bool {
        matches!(self.waits.last(), Some((_, None)))
    }

    /// How long the player waited within `window` before `now`.
    fn waited(&mut self, now: Instant, window: Duration) -> Duration {
        let since = now.checked_sub(window).unwrap_or(now);
        self.waits
            .retain(|(_, end)| end.is_none_or(|end| end > since));
        self.waits
            .iter()
            .map(|(start, end)| {
                end.unwrap_or(now)
                    .saturating_duration_since((*start).max(since))
            })
            .sum()
    }

    /// When the waits reach `limit` within `window`, while the player waits; `now` once reached.
    pub(super) fn deadline(
        &mut self,
        now: Instant,
        window: Duration,
        limit: Duration,
    ) -> Option<Instant> {
        if !self.waiting() {
            return None;
        }
        Some(now + limit.saturating_sub(self.waited(now, window)))
    }
}

/// The quality to offer when `current` streams too slowly a source of `bitrate`: the highest one
/// below both. `None` when there is no lower one.
pub(super) fn lower_quality(current: Quality, bitrate: Option<u64>) -> Option<Quality> {
    let ceiling = match (current.max_bitrate(), bitrate) {
        (Some(limit), Some(bitrate)) => limit.min(bitrate),
        (Some(limit), None) | (None, Some(limit)) => limit,
        (None, None) => u64::MAX,
    };
    QUALITIES
        .into_iter()
        .find(|quality| quality.max_bitrate().is_some_and(|limit| limit < ceiling))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_secs(60);
    const LIMIT: Duration = Duration::from_secs(10);

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    #[test]
    fn no_deadline_while_playing() {
        let now = Instant::now();
        let mut stalls = Stalls::default();
        assert_eq!(stalls.deadline(now, WINDOW, LIMIT), None);
        stalls.start(now);
        stalls.stop(now + secs(3));
        assert_eq!(stalls.deadline(now + secs(4), WINDOW, LIMIT), None);
    }

    #[test]
    fn waits_add_up_within_the_window() {
        let now = Instant::now();
        let mut stalls = Stalls::default();
        stalls.start(now);
        stalls.stop(now + secs(4));
        stalls.start(now + secs(20));
        assert_eq!(
            stalls.deadline(now + secs(20), WINDOW, LIMIT),
            Some(now + secs(26))
        );
        assert_eq!(
            stalls.deadline(now + secs(30), WINDOW, LIMIT),
            Some(now + secs(30))
        );
    }

    #[test]
    fn old_waits_leave_the_window() {
        let now = Instant::now();
        let mut stalls = Stalls::default();
        stalls.start(now);
        stalls.stop(now + secs(8));
        stalls.start(now + secs(65));
        // 3 s of the first wait are still within the minute.
        assert_eq!(
            stalls.deadline(now + secs(65), WINDOW, LIMIT),
            Some(now + secs(72))
        );
        stalls.stop(now + secs(66));
        stalls.start(now + secs(130));
        assert_eq!(
            stalls.deadline(now + secs(130), WINDOW, LIMIT),
            Some(now + secs(140))
        );
    }

    #[test]
    fn the_offer_is_below_the_limit_and_the_file() {
        assert_eq!(lower_quality(Quality::Original, None), Some(QUALITIES[1]));
        assert_eq!(
            lower_quality(Quality::Original, Some(6_000_000)),
            Some(QUALITIES[3])
        );
        assert_eq!(
            lower_quality(QUALITIES[2], Some(50_000_000)),
            Some(QUALITIES[3])
        );
        assert_eq!(lower_quality(QUALITIES[6], None), None);
    }
}
