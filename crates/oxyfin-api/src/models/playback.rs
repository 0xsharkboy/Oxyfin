//! Playback info: media sources and their streams.

use super::enums::{MediaStreamType, SubtitleDeliveryMethod};
use crate::{MediaSourceId, PlaySessionId, Ticks};
use serde::{Deserialize, Deserializer};

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
    /// Whether the subtitle stream is flagged for the deaf and hard of hearing (SDH).
    pub is_hearing_impaired: bool,
    /// Whether the stream lives in a separate file.
    pub is_external: bool,
    /// Whether the stream is a text subtitle, which the server can deliver as a separate file.
    pub is_text_subtitle_stream: bool,
    /// Number of audio channels.
    pub channels: Option<u32>,
    /// Audio channel layout.
    pub channel_layout: Option<String>,
    /// How the server delivers this subtitle stream for the requested playback.
    pub delivery_method: Option<SubtitleDeliveryMethod>,
    /// Server-relative URL to fetch the stream, for subtitles delivered as
    /// separate files. Credentials the server put in it are removed.
    #[serde(deserialize_with = "without_credentials")]
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
            is_hearing_impaired: false,
            is_external: false,
            is_text_subtitle_stream: false,
            channels: None,
            channel_layout: None,
            delivery_method: None,
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
    /// Whether the server can remux the file without encoding it.
    #[serde(default)]
    pub supports_direct_stream: bool,
    /// Whether the server can transcode the file.
    #[serde(default)]
    pub supports_transcoding: bool,
    /// Server-relative URL of the transcoded stream, when the server chose to
    /// transcode. Credentials the server put in it are removed.
    #[serde(default, deserialize_with = "without_credentials")]
    pub transcoding_url: Option<String>,
    /// Streaming protocol of the transcoded stream, such as `hls`.
    #[serde(default)]
    pub transcoding_sub_protocol: Option<String>,
    /// Tracks of the file.
    #[serde(default)]
    pub media_streams: Vec<MediaStream>,
    /// Index of the audio stream the server selects by default.
    pub default_audio_stream_index: Option<i32>,
    /// Index of the subtitle stream the server selects by default.
    pub default_subtitle_stream_index: Option<i32>,
}

impl MediaSourceInfo {
    /// Why the server transcodes, as listed in the transcoding URL, such as
    /// `ContainerBitrateExceedsLimit`. Empty without a transcode.
    pub fn transcode_reasons(&self) -> Vec<String> {
        self.transcoding_parameter("TranscodeReasons")
            .map(|reasons| {
                reasons
                    .split(',')
                    .filter(|reason| !reason.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The audio stream the transcode carries, as listed in the transcoding
    /// URL. `None` without a transcode.
    pub fn transcoding_audio_stream_index(&self) -> Option<i32> {
        self.transcoding_parameter("AudioStreamIndex")?.parse().ok()
    }

    fn transcoding_parameter(&self, name: &str) -> Option<String> {
        let url = self.transcoding_url.as_deref()?;
        let query = url.split_once('?').map_or("", |(_, query)| query);
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.into_owned())
    }
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

/// Query parameters that carry the access token: Jellyfin 10.10 and later
/// write `ApiKey`, older servers `api_key`.
const CREDENTIAL_KEYS: [&str; 2] = ["apikey", "api_key"];

/// Removes the access token from the query of a server-relative URL.
pub(crate) fn strip_credentials(url: &str) -> String {
    let Some((path, query)) = url.split_once('?') else {
        return url.to_owned();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let key = pair.split_once('=').map_or(*pair, |(key, _)| key);
            !CREDENTIAL_KEYS
                .iter()
                .any(|credential| key.eq_ignore_ascii_case(credential))
        })
        .collect();
    if kept.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", kept.join("&"))
    }
}

fn without_credentials<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let url = Option::<String>::deserialize(deserializer)?;
    Ok(url.map(|url| strip_credentials(&url)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_stripped_whatever_their_case() {
        assert_eq!(
            strip_credentials("/videos/a/master.m3u8?DeviceId=d&ApiKey=secret&Tag=t"),
            "/videos/a/master.m3u8?DeviceId=d&Tag=t"
        );
        assert_eq!(
            strip_credentials("/Videos/a/Stream.srt?api_key=secret"),
            "/Videos/a/Stream.srt"
        );
        assert_eq!(strip_credentials("/a?APIKEY=x&b=1"), "/a?b=1");
        assert_eq!(strip_credentials("/a"), "/a");
    }

    #[test]
    fn hearing_impaired_flag_is_read() {
        let stream: MediaStream = serde_json::from_value(serde_json::json!({
            "Index": 4,
            "Type": "Subtitle",
            "IsHearingImpaired": true,
        }))
        .unwrap();
        assert!(stream.is_hearing_impaired);
        let stream: MediaStream =
            serde_json::from_value(serde_json::json!({ "Index": 4, "Type": "Subtitle" })).unwrap();
        assert!(!stream.is_hearing_impaired);
    }

    #[test]
    fn transcode_reasons_come_from_the_url() {
        let source: MediaSourceInfo = serde_json::from_value(serde_json::json!({
            "Id": "a",
            "TranscodingUrl": "/videos/a/master.m3u8?ApiKey=k&TranscodeReasons=ContainerBitrateExceedsLimit,VideoCodecNotSupported",
        }))
        .unwrap();
        assert_eq!(
            source.transcode_reasons(),
            ["ContainerBitrateExceedsLimit", "VideoCodecNotSupported"]
        );
        assert_eq!(source.transcoding_audio_stream_index(), None);
        assert_eq!(
            source.transcoding_url.as_deref(),
            Some(
                "/videos/a/master.m3u8?TranscodeReasons=ContainerBitrateExceedsLimit,VideoCodecNotSupported"
            )
        );
    }
}
