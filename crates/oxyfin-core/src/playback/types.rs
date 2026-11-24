//! Public types of the playback controller: commands, events, errors and
//! timings.

use std::time::Duration;

use oxyfin_api::{ItemId, PlayMethod};
use oxyfin_player::{PlaybackStats, PlayerError, SeekTarget};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{AudioOption, Chapter, Quality, SegmentKind, SkippableSegment, SubtitleOption};
use crate::SubtitleStyle;
use crate::error::CoreError;
use crate::models::{Episode, ImageRef};
use crate::trickplay::Trickplay;

/// Interval between two progress reports while playing.
pub const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);
/// Step of the skip backward and skip forward buttons.
pub const SEEK_STEP: Duration = Duration::from_secs(10);
/// Lengths of the countdown before the next episode starts, offered to the user.
pub const COUNTDOWNS: [Duration; 4] = [
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(15),
    Duration::from_secs(30),
];
/// Longest wait for the last report when shutting down.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
/// Playback speeds offered to the user.
pub const SPEEDS: [f64; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
/// Longest wait for the player to unload the previous media before loading the next.
pub const UNLOAD_GRACE: Duration = Duration::from_millis(500);
/// Wait between two requests for the subtitles of the playing item, while the
/// server lists no new one after [`PlaybackCommand::ReloadSubtitles`].
pub const SUBTITLE_RETRY: Duration = Duration::from_secs(1);
/// How many times the subtitles of the playing item are asked again after
/// [`PlaybackCommand::ReloadSubtitles`], while the server lists no new one.
pub const SUBTITLE_RETRIES: u32 = 10;

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
    /// Wait between two requests for the subtitles of the playing item after
    /// [`PlaybackCommand::ReloadSubtitles`], while the server lists none new.
    pub subtitle_retry: Duration,
}

impl Default for PlaybackTimings {
    fn default() -> Self {
        Self {
            progress_interval: PROGRESS_INTERVAL,
            next_up_countdown_tick: Duration::from_secs(1),
            shutdown_timeout: SHUTDOWN_TIMEOUT,
            unload_grace: UNLOAD_GRACE,
            subtitle_retry: SUBTITLE_RETRY,
        }
    }
}

/// What the user chose for the end of an episode and for skippable segments.
///
/// Stored as is in the configuration: a countdown that is not one of
/// [`COUNTDOWNS`] reads as the default one, and a missing field as its default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackPrefs {
    /// Whether the next episode starts by itself after the countdown, rather
    /// than waiting for the user.
    pub autoplay_next: bool,
    /// Length of the countdown before the next episode starts.
    #[serde(
        serialize_with = "countdown_to_secs",
        deserialize_with = "countdown_from_secs"
    )]
    pub countdown: Duration,
    /// Whether intros are skipped without asking.
    pub skip_intros: bool,
    /// Whether recaps and previews are skipped without asking.
    pub skip_recaps: bool,
    /// Whether the audio and subtitle choices are kept for the rest of a series.
    pub remember_tracks: bool,
}

impl Default for PlaybackPrefs {
    fn default() -> Self {
        Self {
            autoplay_next: true,
            countdown: Duration::from_secs(10),
            skip_intros: false,
            skip_recaps: false,
            remember_tracks: true,
        }
    }
}

impl PlaybackPrefs {
    /// Whether segments of `kind` are skipped without asking. Credits never
    /// are: they lead to the next episode.
    pub fn skips(&self, kind: SegmentKind) -> bool {
        match kind {
            SegmentKind::Intro => self.skip_intros,
            SegmentKind::Recap | SegmentKind::Preview => self.skip_recaps,
            SegmentKind::Credits => false,
        }
    }
}

fn countdown_to_secs<S: Serializer>(
    countdown: &Duration,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_u64(countdown.as_secs())
}

fn countdown_from_secs<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
    let countdown = Duration::from_secs(u64::deserialize(deserializer)?);
    Ok(if COUNTDOWNS.contains(&countdown) {
        countdown
    } else {
        PlaybackPrefs::default().countdown
    })
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
    ///
    /// A series plays its next episode to watch, in [`PlayMode::Auto`]
    /// whatever `mode` is; [`PlaybackError::NothingToPlay`] when it has none.
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
    /// Go to the previous episode, or restart the current item when there is
    /// none.
    ///
    /// Sent while an item loads, it waits for that item to be ready, then
    /// applies to it; only the latest `Previous` or `Next` waits, and a later
    /// `Play` or `Stop` drops it.
    Previous,
    /// Go to the next episode, if any.
    ///
    /// Sent while an item loads, it waits like [`PlaybackCommand::Previous`].
    Next,
    /// Start the next episode offered at the end, without waiting for the
    /// countdown.
    PlayNextNow,
    /// Cancel the next episode countdown, or the offer of the next episode.
    CancelNextUp,
    /// Apply the user's playback choices, from the next end of an episode
    /// and the next segment on.
    SetPrefs(PlaybackPrefs),
    /// Skip the segment offered at the current position, as its skip button
    /// does: unlike a seek, the segments after it may still be skipped by
    /// themselves.
    SkipSegment,
    /// Go back to where the last automatic skip of the current item
    /// happened. The segment is then only offered, never skipped again.
    UndoSkip,
    /// Set the playback speed.
    SetSpeed(f64),
    /// Set the volume, in percent.
    SetVolume(f64),
    /// Mute or unmute.
    SetMuted(bool),
    /// Delay the subtitles by this many milliseconds, earlier when
    /// negative. The delay stays when the item loads again, such as for a
    /// quality change, and goes back to zero for the next item.
    SetSubtitleDelay(i64),
    /// Delay the audio by this many milliseconds, earlier when negative.
    /// Kept and reset like [`PlaybackCommand::SetSubtitleDelay`].
    SetAudioDelay(i64),
    /// Change the subtitle delay by this many milliseconds, from the delay the
    /// controller holds, so that steps sent before their event comes back add up.
    AdjustSubtitleDelay(i64),
    /// Change the audio delay by this many milliseconds, like
    /// [`PlaybackCommand::AdjustSubtitleDelay`].
    AdjustAudioDelay(i64),
    /// Select the audio track with this Jellyfin stream index.
    SelectAudio(i32),
    /// Select the subtitle track with this Jellyfin stream index, or none.
    SelectSubtitle(Option<i32>),
    /// Set the highest streaming quality. The current item, unless it
    /// reached its end, loads again at its position with its tracks, pause
    /// and speed, as from a new [`PlaybackEvent::Loading`], and its
    /// [`PlaybackEvent::Ready`] and [`PlaybackEvent::Chapters`] are followed
    /// by its [`PlaybackEvent::Paused`] state. The quality stays for the next items.
    SetQuality(Quality),
    /// Change how subtitles look. Applies at once, even with nothing
    /// playing, and again before each item loads.
    SetSubtitleStyle(SubtitleStyle),
    /// Show the subtitle files the server added to the playing item since it
    /// loaded, such as one just downloaded with [`Catalog::download_subtitle`],
    /// and select the one in `select_language`, or else the first of them. The
    /// server lists a downloaded file once it has refreshed the item, so the
    /// controller asks for the subtitles of the item every
    /// [`PlaybackTimings::subtitle_retry`], at most [`SUBTITLE_RETRIES`] more
    /// times. When the item loads again first, as for another quality, the
    /// request waits until it is ready. A file downloaded again in a language
    /// the item already has replaces it, once no new file came. Answered with
    /// [`PlaybackEvent::SubtitlesReloaded`], after [`PlaybackEvent::TracksChanged`]
    /// when a file was added.
    ///
    /// [`Catalog::download_subtitle`]: crate::Catalog::download_subtitle
    ReloadSubtitles {
        /// Language to select, as an ISO 639-2 code such as `fre`.
        select_language: String,
    },
    /// Ask for the statistics of the loaded item, answered with
    /// [`PlaybackEvent::Stats`]; ignored while nothing is loaded.
    RequestStats,
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
    /// Production year.
    pub year: Option<i32>,
    /// Image standing for the item, such as its series poster when it has none.
    pub artwork: Option<ImageRef>,
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
    /// The loaded item has seek bar thumbnails: sent just before
    /// [`PlaybackEvent::Ready`], and only then. The next
    /// [`PlaybackEvent::Loading`] drops them.
    ///
    /// The tile of a time is [`Trickplay::tile_at`], and its sheet comes from
    /// [`ImageCache::trickplay_sheet`].
    ///
    /// [`ImageCache::trickplay_sheet`]: crate::ImageCache::trickplay_sheet
    Trickplay(Trickplay),
    /// The chapters of the loaded item: sent right after each
    /// [`PlaybackEvent::Ready`], empty when the item has fewer than two.
    /// Chapter images come from [`ImageCache::get`].
    ///
    /// [`ImageCache::get`]: crate::ImageCache::get
    Chapters(Vec<Chapter>),
    /// The position changed.
    Position(Duration),
    /// The position entered a part the user can skip, or left it (`None`).
    ///
    /// Segments come from the server after the item starts; without them,
    /// or when fetching them fails, this event never comes. Skipping is a
    /// [`PlaybackCommand::Seek`] to [`SkippableSegment::end`]. The real end
    /// of the item always leaves the segment. A segment the
    /// [`PlaybackPrefs`] skip is not offered when entered:
    /// [`PlaybackEvent::SegmentSkipped`] comes instead, once per segment.
    Segment(Option<SkippableSegment>),
    /// The paused state changed.
    Paused(bool),
    /// The playback speed changed.
    Speed(f64),
    /// The volume changed.
    Volume(f64),
    /// The muted state changed.
    Muted(bool),
    /// The subtitle delay changed, in milliseconds.
    SubtitleDelay(i64),
    /// The audio delay changed, in milliseconds.
    AudioDelay(i64),
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
    /// The next episode starts in `remaining` seconds, unless cancelled.
    NextUpCountdown {
        /// Episode about to start.
        episode: Box<Episode>,
        /// Seconds left.
        remaining: u32,
        /// Seconds of the whole countdown.
        total: u32,
    },
    /// The item played to its end and the next episode waits for
    /// [`PlaybackCommand::PlayNextNow`]: the user asked not to start it by
    /// itself.
    NextUpReady {
        /// Episode offered.
        episode: Box<Episode>,
    },
    /// The next episode countdown, or its offer, was cancelled.
    NextUpCancelled,
    /// A segment was skipped without asking, from `from` to its end.
    /// [`PlaybackCommand::UndoSkip`] goes back to `from`.
    SegmentSkipped {
        /// What the segment held.
        kind: SegmentKind,
        /// Where the skip happened.
        from: Duration,
    },
    /// Playback stopped.
    Stopped,
    /// The server answered the report of a stop: what it lists from now on takes it into account.
    StopReported,
    /// Playback or a command failed.
    Error(PlaybackError),
    /// The answer to [`PlaybackCommand::ReloadSubtitles`]: whether a new
    /// subtitle file was added to the playing item. Sent once per request:
    /// `false` as well when the item stops or another one loads first.
    SubtitlesReloaded {
        /// Whether a file was added.
        added: bool,
    },
    /// The statistics asked with [`PlaybackCommand::RequestStats`].
    Stats(Box<StatsReport>),
    /// The server no longer accepts the session; playback stopped.
    ///
    /// The binary must then call [`SessionManager::expire`] and rebuild the
    /// [`Catalog`], the [`ImageCache`] and the playback controller for the
    /// next session.
    ///
    /// [`SessionManager::expire`]: crate::SessionManager::expire
    /// [`Catalog`]: crate::Catalog
    /// [`ImageCache`]: crate::ImageCache
    SessionExpired,
}

/// Statistics of the loaded item.
#[derive(Clone, Debug, PartialEq)]
pub struct StatsReport {
    /// How the server delivers the item.
    pub method: PlayMethod,
    /// Why the server transcodes, such as `VideoCodecNotSupported`, as the
    /// server names it. Empty without a transcode or when the server gives none.
    pub transcode_reasons: Vec<String>,
    /// What the player measures.
    pub player: PlaybackStats,
}

/// Why playback or a command failed. The UI translates each variant.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlaybackError {
    /// The server offers no source the player can play, as is or transcoded.
    #[error("no source can be played")]
    NoPlayableSource,
    /// The series has no episode to watch.
    #[error("the series has no episode to watch")]
    NothingToPlay,
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
