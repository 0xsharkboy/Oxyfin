//! Jellyfin tick counts.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Number of ticks (100 ns units) in one second.
pub const TICKS_PER_SECOND: i64 = 10_000_000;

const NANOS_PER_TICK: u128 = 100;

/// A duration in Jellyfin ticks of 100 nanoseconds.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Ticks(pub i64);

impl Ticks {
    /// Converts a duration to ticks, truncating below 100 ns and saturating at `i64::MAX`.
    pub fn from_duration(d: Duration) -> Ticks {
        let ticks = d.as_nanos() / NANOS_PER_TICK;
        Ticks(i64::try_from(ticks).unwrap_or(i64::MAX))
    }

    /// Converts ticks to a duration; negative values give `Duration::ZERO`.
    pub fn to_duration(self) -> Duration {
        let Ok(ticks) = u64::try_from(self.0) else {
            return Duration::ZERO;
        };
        let per_second = TICKS_PER_SECOND.unsigned_abs();
        let nanos = (ticks % per_second) * NANOS_PER_TICK as u64;
        Duration::new(ticks / per_second, nanos as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn ticks_round_trip() {
        assert_eq!(
            Ticks::from_duration(Duration::from_secs(90)),
            Ticks(900_000_000)
        );
        assert_eq!(Ticks(15_000_000).to_duration(), Duration::from_millis(1500));
    }

    #[test]
    fn negative_ticks_are_zero_duration() {
        assert_eq!(Ticks(-5).to_duration(), Duration::ZERO);
    }

    #[test]
    fn huge_duration_saturates() {
        assert_eq!(Ticks::from_duration(Duration::MAX), Ticks(i64::MAX));
    }
}
