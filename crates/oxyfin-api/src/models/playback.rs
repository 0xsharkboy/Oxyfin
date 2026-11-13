//! Playback info: media sources and their streams.

use super::enums::MediaStreamType;
use crate::{MediaSourceId, PlaySessionId, Ticks};
use serde::Deserialize;

/// One audio, video or subtitle track of a media source.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct MediaStream {
    /// Index of the stream inside its media source.
    pub index: i32,
    /// Kind of stream.
    pub r#type: MediaStreamType,
    /// Codec name.
    pub codec: Option<String>,
    /// Language code, as reported by the server.
    pub language: Option<String>,
    /// Title of the stream.
    pub title: Option<String>,
    /// Title built by the server for display.
    pub display_title: Option<String>,
    /// Whether the stream is flagged as default.
    pub is_default: bool,
    /// Whether the stream is flagged as forced.
    pub is_forced: bool,
    /// Whether the stream lives in a separate file.
    pub is_external: bool,
    /// Number of audio channels.
    pub channels: Option<u32>,
    /// Audio channel layout.
    pub channel_layout: Option<String>,
    /// Server-relative URL to fetch the stream, for external subtitles.
    pub delivery_url: Option<String>,
}

impl Default for MediaStream {
    fn default() -> Self {
        Self {
            index: 0,
            r#type: MediaStreamType::Other(String::new()),
            codec: None,
            language: None,
            title: None,
            display_title: None,
            is_default: false,
            is_forced: false,
            is_external: false,
            channels: None,
            channel_layout: None,
            delivery_url: None,
        }
    }
}

/// A playable file of an item, with its streams.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSourceInfo {
    /// Identifier of the media source.
    pub id: MediaSourceId,
    /// Container format, such as `mkv`.
    pub container: Option<String>,
    /// Duration of the media.
    pub run_time_ticks: Option<Ticks>,
    /// Whether the server allows playing the file as is.
    #[serde(default)]
    pub supports_direct_play: bool,
    /// Tracks of the file.
    #[serde(default)]
    pub media_streams: Vec<MediaStream>,
    /// Index of the audio stream the server selects by default.
    pub default_audio_stream_index: Option<i32>,
    /// Index of the subtitle stream the server selects by default.
    pub default_subtitle_stream_index: Option<i32>,
}

/// Answer of the playback info request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PlaybackInfoResponse {
    /// Playable files of the item.
    pub media_sources: Vec<MediaSourceInfo>,
    /// Identifier to quote in playback reports.
    pub play_session_id: Option<PlaySessionId>,
}
