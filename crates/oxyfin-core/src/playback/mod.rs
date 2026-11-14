//! Playback of a media: track selection and episode navigation.

#[allow(dead_code, reason = "used by the playback session added later")]
mod queue;
#[allow(dead_code, reason = "used by the playback session added later")]
mod tracks;

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
