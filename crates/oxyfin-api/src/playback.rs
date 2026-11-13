//! Playback info, stream sources and playback reports.

use crate::{ItemId, MediaSourceId, PlaySessionId, PlaybackInfoResponse, Result, Session, Ticks};
use serde::Serialize;
use serde::de::IgnoredAny;
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
    play_method: Option<&'static str>,
}

impl<'a> ReportBody<'a> {
    fn new(report: &'a PlaybackReport, play_method: Option<&'static str>) -> Self {
        Self {
            item_id: &report.item_id,
            media_source_id: &report.media_source_id,
            play_session_id: report.play_session_id.as_ref(),
            position_ticks: report.position,
            is_paused: report.is_paused,
            audio_stream_index: report.audio_stream_index,
            subtitle_stream_index: report.subtitle_stream_index,
            play_method,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PlaybackInfoBody<'a> {
    user_id: &'a str,
    enable_direct_play: bool,
    enable_direct_stream: bool,
    enable_transcoding: bool,
    auto_open_live_stream: bool,
}

impl Session {
    /// Fetches the media sources of an item (`POST Items/{id}/PlaybackInfo`).
    pub async fn playback_info(&self, id: &ItemId) -> Result<PlaybackInfoResponse> {
        let body = PlaybackInfoBody {
            user_id: self.user_id().as_str(),
            enable_direct_play: true,
            enable_direct_stream: true,
            enable_transcoding: false,
            auto_open_live_stream: false,
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

    async fn report(
        &self,
        route: &str,
        report: &PlaybackReport,
        play_method: Option<&'static str>,
    ) -> Result<()> {
        let _: IgnoredAny = self
            .server()
            .http()
            .post_json(
                Some(self.token()),
                route,
                &[],
                &ReportBody::new(report, play_method),
            )
            .await?;
        Ok(())
    }

    /// Tells the server playback started (`POST Sessions/Playing`).
    pub async fn report_playback_start(&self, report: &PlaybackReport) -> Result<()> {
        self.report("Sessions/Playing", report, Some("DirectPlay"))
            .await
    }

    /// Tells the server the playback position (`POST Sessions/Playing/Progress`).
    pub async fn report_playback_progress(&self, report: &PlaybackReport) -> Result<()> {
        self.report("Sessions/Playing/Progress", report, Some("DirectPlay"))
            .await
    }

    /// Tells the server playback ended (`POST Sessions/Playing/Stopped`).
    pub async fn report_playback_stopped(&self, report: &PlaybackReport) -> Result<()> {
        self.report("Sessions/Playing/Stopped", report, None).await
    }
}
