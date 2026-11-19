//! Streaming quality: the bitrate and height limits the player asks of the server.

use serde::{Deserialize, Serialize};

/// Highest quality the player asks of the server. A file above the limit is
/// transcoded by the server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Quality {
    /// No limit: the file plays as is whenever the server allows it.
    #[default]
    Original,
    /// At most `bitrate` bits per second and `height` lines; the server
    /// transcodes a file above either limit.
    Limited {
        /// Highest bitrate, in bits per second.
        bitrate: u64,
        /// Highest picture height, in lines.
        height: u32,
    },
}

impl Quality {
    /// The bitrate limit, in bits per second; `None` for no limit.
    pub fn max_bitrate(self) -> Option<u64> {
        match self {
            Self::Original => None,
            Self::Limited { bitrate, .. } => Some(bitrate),
        }
    }

    /// The picture height limit, in lines; `None` for no limit.
    pub fn max_height(self) -> Option<u32> {
        match self {
            Self::Original => None,
            Self::Limited { height, .. } => Some(height),
        }
    }
}

/// Qualities offered to the user, from the highest. The limits follow the
/// quality menu of Jellyfin's web client.
pub const QUALITIES: [Quality; 7] = [
    Quality::Original,
    Quality::Limited {
        bitrate: 20_000_000,
        height: 1080,
    },
    Quality::Limited {
        bitrate: 10_000_000,
        height: 1080,
    },
    Quality::Limited {
        bitrate: 4_000_000,
        height: 720,
    },
    Quality::Limited {
        bitrate: 1_500_000,
        height: 480,
    },
    Quality::Limited {
        bitrate: 720_000,
        height: 480,
    },
    Quality::Limited {
        bitrate: 420_000,
        height: 360,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_quality_has_a_height() {
        assert_eq!(Quality::Original.max_height(), None);
        assert_eq!(QUALITIES[4].max_height(), Some(480));
    }

    #[test]
    fn original_has_no_limit() {
        assert_eq!(Quality::Original.max_bitrate(), None);
        assert_eq!(QUALITIES[0], Quality::Original);
        assert_eq!(QUALITIES[3].max_bitrate(), Some(4_000_000));
    }

    #[test]
    fn qualities_go_down() {
        let limits: Vec<u64> = QUALITIES.iter().filter_map(|q| q.max_bitrate()).collect();
        assert!(
            limits.windows(2).all(|pair| pair[0] > pair[1]),
            "{limits:?}"
        );
    }

    #[test]
    fn quality_round_trips_through_json() {
        for quality in QUALITIES {
            let json = serde_json::to_string(&quality).unwrap();
            assert_eq!(serde_json::from_str::<Quality>(&json).unwrap(), quality);
        }
    }
}
