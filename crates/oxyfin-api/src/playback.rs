//! Playback info, stream sources and playback reports.

use crate::{ItemId, MediaSourceId, PlaySessionId, PlaybackInfoResponse, Result, Session, Ticks};
use serde::Serialize;
use serde::de::IgnoredAny;
use serde_json::{Value, json};
use std::fmt;
use url::Url;

/// Where and how a player fetches the media.
///
/// The URL carries no credentials; the player sends `authorization` as the `Authorization` header.
#[derive(Clone, PartialEq, Eq)]
pub struct StreamSource {
    /// Stream URL.
    pub url: Url,
    /// Full `Authorization` header value, including the access token.
    pub authorization: String,
}

impl fmt::Debug for StreamSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamSource")
            .field("url", &self.url.as_str())
            .field("authorization", &"<redacted>")
            .finish()
    }
}

/// How the media reaches the player, as reported to the server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum PlayMethod {
    /// The original file, as is.
    #[default]
    DirectPlay,
    /// The original streams in another container.
    DirectStream,
    /// A stream the server encodes.
    Transcode,
}

/// What the player asks of the playback info request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaybackOptions {
    /// Highest bitrate the player accepts, in bits per second: a file above it
    /// is transcoded. `None` accepts any bitrate.
    pub max_streaming_bitrate: Option<u64>,
    /// Audio stream a transcode carries; the server's default when `None`.
    pub audio_stream_index: Option<i32>,
    /// Subtitle stream to deliver, `-1` for none; the server's default when `None`.
    pub subtitle_stream_index: Option<i32>,
    /// Highest picture height the player accepts, in lines: a transcode is
    /// scaled down to it. `None` accepts any height.
    pub max_height: Option<u32>,
}

/// Playback state sent to the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaybackReport {
    /// Item being played.
    pub item_id: ItemId,
    /// Media source being played.
    pub media_source_id: MediaSourceId,
    /// Playback session, from the playback info.
    pub play_session_id: Option<PlaySessionId>,
    /// Current position.
    pub position: Ticks,
    /// Whether playback is paused.
    pub is_paused: bool,
    /// Selected audio stream.
    pub audio_stream_index: Option<i32>,
    /// Selected subtitle stream.
    pub subtitle_stream_index: Option<i32>,
    /// How the media reaches the player.
    pub play_method: PlayMethod,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ReportBody<'a> {
    item_id: &'a ItemId,
    media_source_id: &'a MediaSourceId,
    #[serde(skip_serializing_if = "Option::is_none")]
    play_session_id: Option<&'a PlaySessionId>,
    position_ticks: Ticks,
    is_paused: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio_stream_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subtitle_stream_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    play_method: Option<PlayMethod>,
}

impl<'a> ReportBody<'a> {
    fn new(report: &'a PlaybackReport, with_method: bool) -> Self {
        Self {
            item_id: &report.item_id,
            media_source_id: &report.media_source_id,
            play_session_id: report.play_session_id.as_ref(),
            position_ticks: report.position,
            is_paused: report.is_paused,
            audio_stream_index: report.audio_stream_index,
            subtitle_stream_index: report.subtitle_stream_index,
            play_method: with_method.then_some(report.play_method),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PlaybackInfoBody<'a> {
    user_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_streaming_bitrate: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio_stream_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subtitle_stream_index: Option<i32>,
    enable_direct_play: bool,
    enable_direct_stream: bool,
    enable_transcoding: bool,
    allow_video_stream_copy: bool,
    allow_audio_stream_copy: bool,
    auto_open_live_stream: bool,
    device_profile: Value,
}

/// Text subtitle formats the player loads as separate files.
const TEXT_SUBTITLES: [&str; 6] = ["srt", "subrip", "ass", "ssa", "vtt", "webvtt"];

/// What the player can play, for the server to decide between direct play
/// and transcoding.
///
/// mpv plays nearly every container and codec, so direct play has no
/// restriction and only the bitrate and height limits force a transcode.
/// The transcode is H.264 in MPEG-TS over HLS, the most widely decodable,
/// with text subtitles as separate files.
fn device_profile(max_height: Option<u32>) -> Value {
    let subtitles: Vec<Value> = TEXT_SUBTITLES
        .iter()
        .map(|format| json!({ "Format": format, "Method": "External" }))
        .collect();
    let mut profile = json!({
        "Name": crate::CLIENT_NAME,
        "DirectPlayProfiles": [{ "Type": "Video" }, { "Type": "Audio" }],
        "TranscodingProfiles": [{
            "Type": "Video",
            "Container": "ts",
            "Protocol": "hls",
            "Context": "Streaming",
            "VideoCodec": "h264",
            "AudioCodec": "aac,mp3,ac3",
            "MaxAudioChannels": "6",
        }],
        "SubtitleProfiles": subtitles,
    });
    if let Some(height) = max_height {
        // Under a bitrate limit the server ignores a height alone, so the 16:9 width goes along.
        let width = (u64::from(height) * 16).div_ceil(9);
        let limit = |property: &str, value: u64| {
            json!({
                "Condition": "LessThanEqual",
                "Property": property,
                "Value": value.to_string(),
                "IsRequired": false,
            })
        };
        profile["CodecProfiles"] = json!([{
            "Type": "Video",
            "Conditions": [limit("Height", u64::from(height)), limit("Width", width)],
        }]);
    }
    profile
}

impl Session {
    /// Fetches the media sources of an item (`POST Items/{id}/PlaybackInfo`),
    /// with no bitrate limit and the server's default tracks.
    pub async fn playback_info(&self, id: &ItemId) -> Result<PlaybackInfoResponse> {
        self.playback_info_with(id, &PlaybackOptions::default())
            .await
    }

    /// Fetches the media sources of an item (`POST Items/{id}/PlaybackInfo`)
    /// for the given options. A source the server will not play directly
    /// carries a `TranscodingUrl`, for [`Session::transcode_source`].
    pub async fn playback_info_with(
        &self,
        id: &ItemId,
        options: &PlaybackOptions,
    ) -> Result<PlaybackInfoResponse> {
        let body = PlaybackInfoBody {
            user_id: self.user_id().as_str(),
            max_streaming_bitrate: options.max_streaming_bitrate,
            audio_stream_index: options.audio_stream_index,
            subtitle_stream_index: options.subtitle_stream_index,
            enable_direct_play: true,
            enable_direct_stream: true,
            enable_transcoding: true,
            allow_video_stream_copy: true,
            allow_audio_stream_copy: true,
            auto_open_live_stream: false,
            device_profile: device_profile(options.max_height),
        };
        let query = [("userId", self.user_id().to_string())];
        self.server()
            .http()
            .post_json(
                Some(self.token()),
                &format!("Items/{id}/PlaybackInfo"),
                &query,
                &body,
            )
            .await
    }

    /// Direct stream URL of a media source, with the header that authenticates it.
    pub fn stream_source(&self, id: &ItemId, media_source_id: &MediaSourceId) -> StreamSource {
        let http = self.server().http();
        let query = [
            ("static", "true".to_owned()),
            ("mediaSourceId", media_source_id.to_string()),
        ];
        StreamSource {
            url: http.url(&format!("Videos/{id}/stream"), &query),
            authorization: self.authorization_header(),
        }
    }

    /// The transcoded stream of a media source, from its `TranscodingUrl`,
    /// with the header that authenticates it and every nested request of the
    /// HLS playlist. `None` when the URL is unusable.
    pub fn transcode_source(&self, transcoding_url: &str) -> Option<StreamSource> {
        Some(StreamSource {
            url: self.server_url(transcoding_url)?,
            authorization: self.authorization_header(),
        })
    }

    /// Absolute URL of a server-relative URL, such as a subtitle's
    /// `DeliveryUrl`, keeping the server's base path. `None` when it is not
    /// a valid URL.
    pub fn server_url(&self, relative: &str) -> Option<Url> {
        let (path, query) = match relative.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (relative, None),
        };
        let mut url = self.server().http().url(path.trim_start_matches('/'), &[]);
        url.set_query(query.filter(|query| !query.is_empty()));
        (url.scheme() == "http" || url.scheme() == "https").then_some(url)
    }

    /// URL of an external subtitle, without credentials.
    pub fn subtitle_url(
        &self,
        id: &ItemId,
        media_source_id: &MediaSourceId,
        index: i32,
        format: &str,
    ) -> Url {
        let route = format!("Videos/{id}/{media_source_id}/Subtitles/{index}/Stream.{format}");
        self.server().http().url(&route, &[])
    }

    /// Stops the server's transcode of a playback session
    /// (`DELETE Videos/ActiveEncodings`).
    pub async fn stop_transcoding(&self, play_session_id: &PlaySessionId) -> Result<()> {
        let query = [
            ("deviceId", self.server().http().device_id().to_owned()),
            ("playSessionId", play_session_id.to_string()),
        ];
        let _: IgnoredAny = self
            .server()
            .http()
            .delete_empty(Some(self.token()), "Videos/ActiveEncodings", &query)
            .await?;
        Ok(())
    }

    async fn report(&self, route: &str, report: &PlaybackReport, with_method: bool) -> Result<()> {
        let _: IgnoredAny = self
            .server()
            .http()
            .post_json(
                Some(self.token()),
                route,
                &[],
                &ReportBody::new(report, with_method),
            )
            .await?;
        Ok(())
    }

    /// Tells the server playback started (`POST Sessions/Playing`).
    pub async fn report_playback_start(&self, report: &PlaybackReport) -> Result<()> {
        self.report("Sessions/Playing", report, true).await
    }

    /// Tells the server the playback position (`POST Sessions/Playing/Progress`).
    pub async fn report_playback_progress(&self, report: &PlaybackReport) -> Result<()> {
        self.report("Sessions/Playing/Progress", report, true).await
    }

    /// Tells the server playback ended (`POST Sessions/Playing/Stopped`).
    pub async fn report_playback_stopped(&self, report: &PlaybackReport) -> Result<()> {
        self.report("Sessions/Playing/Stopped", report, false).await
    }
}
