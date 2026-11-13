//! Public types shared by the player and its consumers.

use std::fmt;
use std::time::Duration;

/// A media to load, with the HTTP headers needed to fetch it.
#[derive(Clone, PartialEq)]
pub struct MediaSource {
    /// URL or path of the media.
    pub location: String,
    /// HTTP headers sent with every request, in order.
    pub headers: Vec<(String, String)>,
    /// Position at which playback starts.
    pub start: Option<Duration>,
}

impl MediaSource {
    /// Creates a source for `location` with no headers and no start position.
    pub fn new(location: impl Into<String>) -> Self {
        Self {
            location: location.into(),
            headers: Vec::new(),
            start: None,
        }
    }

    /// Appends an HTTP header.
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Sets the position at which playback starts.
    #[must_use]
    pub fn with_start(mut self, start: Duration) -> Self {
        self.start = Some(start);
        self
    }
}

impl fmt::Debug for MediaSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, _)| (name.as_str(), "<redacted>"))
            .collect();
        f.debug_struct("MediaSource")
            .field("location", &self.location)
            .field("headers", &headers)
            .field("start", &self.start)
            .finish()
    }
}

/// A seek request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekTarget {
    /// Jump to an absolute position.
    Absolute(Duration),
    /// Jump forward relative to the current position.
    Forward(Duration),
    /// Jump backward relative to the current position.
    Backward(Duration),
}

/// Identifier of a track, as assigned by mpv.
///
/// Ids are unique only within a [`TrackKind`]: audio 1 and subtitle 1 coexist,
/// so the pair kind + id identifies a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrackId(pub i64);

/// Kind of a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackKind {
    /// A video track.
    Video,
    /// An audio track.
    Audio,
    /// A subtitle track.
    Subtitle,
}

/// A track of the loaded media.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// Identifier assigned by mpv.
    pub id: TrackId,
    /// Kind of track.
    pub kind: TrackKind,
    /// Language code, when known.
    pub language: Option<String>,
    /// Title, when known.
    pub title: Option<String>,
    /// Codec name, when known.
    pub codec: Option<String>,
    /// Number of audio channels, when known.
    pub channels: Option<u32>,
    /// Whether the file flags the track as default.
    pub is_default: bool,
    /// Whether the file flags the track as forced.
    pub is_forced: bool,
    /// Whether the track comes from an external file.
    pub is_external: bool,
    /// Whether the track is currently selected.
    pub is_selected: bool,
    /// Index of the stream in the file (mpv `ff-index`), equal to the Jellyfin
    /// `MediaStream.index` for an internal track.
    pub stream_index: Option<i64>,
    /// URL of the external file, for an external track.
    pub external_url: Option<String>,
}

/// Why the media was unloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnloadReason {
    /// Playback was stopped.
    Stopped,
    /// Another media replaced it.
    Replaced,
    /// Loading or playback failed.
    Error(String),
}

/// An event emitted by the player.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    /// The media is loaded and ready.
    FileLoaded {
        /// Total duration, when known.
        duration: Option<Duration>,
        /// Tracks of the media.
        tracks: Vec<Track>,
    },
    /// The track list or the selection changed. The list may repeat unchanged,
    /// so handle it idempotently.
    Tracks(Vec<Track>),
    /// The playback position changed.
    Position(Duration),
    /// The paused state changed.
    Paused(bool),
    /// The volume changed.
    Volume(f64),
    /// The muted state changed.
    Muted(bool),
    /// A seek started.
    Seeking,
    /// A seek finished. mpv also reports it when playback first starts, so it
    /// can arrive without a preceding [`PlayerEvent::Seeking`].
    Seeked,
    /// Playback reached the end of the media.
    ///
    /// The media stays loaded, and seeking backward resumes playback. A stream
    /// that drops mid-file may also end this way, so compare the last position
    /// with the duration before treating the media as fully watched.
    ///
    /// May be reported again after a renderer is created once playback has
    /// ended; de-duplicate per media.
    Ended,
    /// The media was unloaded.
    Unloaded(UnloadReason),
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn media_source_debug_hides_header_values() {
        let source = MediaSource::new("http://x/v")
            .with_header("Authorization", "MediaBrowser Token=\"secret\"");
        let debug = format!("{source:?}");
        assert!(debug.contains("Authorization"));
        assert!(debug.contains("http://x/v"));
        assert!(!debug.contains("secret"));
    }

    #[test]
    fn media_source_builders_set_fields() {
        let source = MediaSource::new("http://x/v")
            .with_start(Duration::from_secs(12))
            .with_header("A", "1")
            .with_header("B", "2");
        assert_eq!(source.start, Some(Duration::from_secs(12)));
        assert_eq!(
            source.headers,
            vec![
                ("A".to_owned(), "1".to_owned()),
                ("B".to_owned(), "2".to_owned())
            ]
        );
    }
}
