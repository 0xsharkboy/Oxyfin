//! Playback of a media: track selection and episode navigation.

mod chapters;
mod controller;
mod countdown;
mod current;
mod loader;
mod media_player;
mod quality;
mod queue;
mod refresh;
mod reporter;
mod segments;
mod track_memory;
mod tracks;
mod types;

pub use chapters::{
    CHAPTER_RESTART_GRACE, Chapter, chapter_at, next_chapter, previous_chapter, start_at,
};
pub use controller::{PlaybackController, PlaybackHandle};
pub use media_player::{MediaPlayer, PlayerEventSink, PlayerEvents, player_events};
pub use oxyfin_api::PlayMethod;
pub use oxyfin_player::{HdrFormat, HdrInfo, PlaybackStats};
pub use quality::{QUALITIES, Quality};
pub use segments::{SegmentKind, SkippableSegment};
pub use track_memory::{
    AudioPick, SubtitlePick, TRACK_MEMORY_LIMIT, TrackChoice, TrackMemory, best_audio,
    best_subtitle,
};
pub use types::{
    COUNTDOWNS, PROGRESS_INTERVAL, PlayMode, PlaybackCommand, PlaybackError, PlaybackEvent,
    PlaybackPrefs, PlaybackTimings, PlaybackTitle, SEEK_STEP, SHUTDOWN_TIMEOUT, SPEEDS,
    SUBTITLE_RETRIES, SUBTITLE_RETRY, StatsReport, UNLOAD_GRACE,
};

/// An audio track the user can pick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioOption {
    /// Jellyfin stream index.
    pub index: i32,
    /// Label shown in the menu.
    pub label: String,
    /// Language code, when known.
    pub language: Option<String>,
    /// Number of channels, when known.
    pub channels: Option<u32>,
    /// Whether the track is playing.
    pub selected: bool,
}

/// A subtitle track the user can pick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleOption {
    /// Jellyfin stream index.
    pub index: i32,
    /// Label shown in the menu.
    pub label: String,
    /// Language code, when known.
    pub language: Option<String>,
    /// Whether the track is flagged as forced.
    pub forced: bool,
    /// Whether the track comes from a separate file.
    pub external: bool,
    /// Whether the track is shown.
    pub selected: bool,
}
