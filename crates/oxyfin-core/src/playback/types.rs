//! Public types of the playback controller: commands, events, errors and
//! timings.

use std::time::Duration;

use oxyfin_api::ItemId;
use oxyfin_player::{PlayerError, SeekTarget};

use super::{AudioOption, SubtitleOption};
use crate::error::CoreError;
use crate::models::Episode;

/// Interval between two progress reports while playing.
pub const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);
/// Step of the skip backward and skip forward buttons.
pub const SEEK_STEP: Duration = Duration::from_secs(10);
/// Length of the countdown before the next episode starts.
pub const NEXT_UP_COUNTDOWN: Duration = Duration::from_secs(10);
/// Longest wait for the last report when shutting down.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
/// Playback speeds offered to the user.
pub const SPEEDS: [f64; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
/// Longest wait for the player to unload the previous media before loading the next.
pub const UNLOAD_GRACE: Duration = Duration::from_millis(500);

/// Durations driving the controller's timers.
///
/// [`Default`] gives the production values; tests shorten them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackTimings {
    /// Interval between two progress reports while playing.
    pub progress_interval: Duration,
    /// Duration of one step of the next episode countdown.
    pub next_up_countdown_tick: Duration,
    /// Longest wait for the last report when shutting down.
    pub shutdown_timeout: Duration,
    /// Longest wait for the player to unload the previous media before
    /// loading the next one. mpv reports no unload for a media stopped
    /// before it started.
    pub unload_grace: Duration,
}

impl Default for PlaybackTimings {
    fn default() -> Self {
        Self {
            progress_interval: PROGRESS_INTERVAL,
            next_up_countdown_tick: Duration::from_secs(1),
            shutdown_timeout: SHUTDOWN_TIMEOUT,
            unload_grace: UNLOAD_GRACE,
        }
    }
}

/// Where playback of an item starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    /// Resume at the saved position, if any.
    Auto,
    /// Start at the beginning.
    FromStart,
}

/// A request sent to the playback controller.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaybackCommand {
    /// Play an item, replacing the current one.
    Play {
        /// Item to play.
        item: ItemId,
        /// Where playback starts.
        mode: PlayMode,
    },
    /// Pause or resume.
    SetPaused(bool),
    /// Seek in the current item.
    Seek(SeekTarget),
    /// Go to the previous item.
    Previous,
    /// Go to the next item.
    Next,
    /// Start the next episode without waiting for the countdown.
    PlayNextNow,
    /// Cancel the next episode countdown.
    CancelNextUp,
    /// Set the playback speed.
    SetSpeed(f64),
    /// Set the volume, in percent.
    SetVolume(f64),
    /// Mute or unmute.
    SetMuted(bool),
    /// Select the audio track with this Jellyfin stream index.
    SelectAudio(i32),
    /// Select the subtitle track with this Jellyfin stream index, or none.
    SelectSubtitle(Option<i32>),
    /// Stop playback.
    Stop,
}

/// What the UI shows while an item plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackTitle {
    /// Title of the item.
    pub title: String,
    /// Name of the series, for an episode.
    pub series_name: Option<String>,
    /// Label such as `S01E02`, for an episode.
    pub episode_label: Option<String>,
}

/// A change reported by the playback controller.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaybackEvent {
    /// An item is being loaded.
    Loading {
        /// Item being loaded.
        item: ItemId,
        /// Titles to display.
        title: PlaybackTitle,
    },
    /// The item is loaded and playing.
    Ready {
        /// Duration, when known.
        duration: Option<Duration>,
        /// Audio tracks.
        audio: Vec<AudioOption>,
        /// Subtitle tracks.
        subtitles: Vec<SubtitleOption>,
        /// Whether the previous button does something: it plays the previous
        /// episode, or restarts a movie or a first episode. Always true.
        has_previous: bool,
        /// Whether a next episode exists. False for a movie, the last
        /// episode, or when the episode list could not be fetched.
        has_next: bool,
    },
    /// The position changed.
    Position(Duration),
    /// The paused state changed.
    Paused(bool),
    /// The playback speed changed.
    Speed(f64),
    /// The volume changed.
    Volume(f64),
    /// The muted state changed.
    Muted(bool),
    /// A seek requested by the user started.
    Seeking,
    /// A seek requested by the user finished.
    Seeked,
    /// The tracks or their selection changed.
    TracksChanged {
        /// Audio tracks.
        audio: Vec<AudioOption>,
        /// Subtitle tracks.
        subtitles: Vec<SubtitleOption>,
    },
    /// The item played to its end: the last position was within 10 s of the
    /// duration or past 95 % of it, or the duration is unknown. An earlier
    /// end is reported as [`PlaybackEvent::Stopped`].
    Ended,
    /// The next episode starts in `remaining` seconds.
    NextUpCountdown {
        /// Episode about to start.
        episode: Box<Episode>,
        /// Seconds left.
        remaining: u32,
    },
    /// The next episode countdown was cancelled.
    NextUpCancelled,
    /// Playback stopped.
    Stopped,
    /// Playback or a command failed.
    Error(PlaybackError),
    /// The server no longer accepts the session; playback stopped.
    SessionExpired,
}

/// Why playback or a command failed. The UI translates each variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlaybackError {
    /// The server offers no source that can be played directly.
    #[error("no source can be played directly")]
    NoDirectPlaySource,
    /// The media could not be loaded or stopped playing on an error.
    #[error("the media could not be played: {detail}")]
    LoadFailed {
        /// Message of the player, for logs.
        detail: String,
    },
    /// The server could not be reached.
    #[error("the server could not be reached")]
    Network,
    /// The server answered with an unexpected HTTP status.
    #[error("the server answered with HTTP {status}")]
    Server {
        /// HTTP status code.
        status: u16,
    },
    /// The player rejected a command, such as a volume change. Playback
    /// continues.
    #[error("the player rejected a command: {detail}")]
    Command {
        /// Message of the player, for logs.
        detail: String,
    },
}

impl PlaybackError {
    /// A failed server request other than an expired session.
    pub(super) fn request(error: CoreError) -> Self {
        match error {
            CoreError::Network(_) => Self::Network,
            CoreError::Server { status } => Self::Server { status },
            other => Self::LoadFailed {
                detail: other.to_string(),
            },
        }
    }

    /// A media that failed to load or to play.
    pub(super) fn load(detail: impl ToString) -> Self {
        Self::LoadFailed {
            detail: detail.to_string(),
        }
    }

    /// A player command that failed.
    pub(super) fn command(error: &PlayerError) -> Self {
        Self::Command {
            detail: error.to_string(),
        }
    }
}
