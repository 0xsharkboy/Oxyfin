//! Mapping between Jellyfin media streams and mpv tracks.

use std::collections::HashMap;

use oxyfin_api::{MediaStream, MediaStreamType};
use oxyfin_player::{Track, TrackId, TrackKind};

use super::{AudioOption, SubtitleOption};

/// Correspondence between the streams of a media source and the tracks of the
/// loaded media.
///
/// Jellyfin lists external streams first, then the file's streams, so an
/// internal track maps to `stream_index + number of external streams`. mpv
/// ids are unique only per kind, hence the kind in every key.
pub(crate) struct TrackMap {
    streams: Vec<MediaStream>,
    tracks: Vec<Track>,
    to_mpv: HashMap<(TrackKind, i32), TrackId>,
    to_jellyfin: HashMap<(TrackKind, TrackId), i32>,
}

fn kind_of(stream: &MediaStream) -> Option<TrackKind> {
    match stream.r#type {
        MediaStreamType::Audio => Some(TrackKind::Audio),
        MediaStreamType::Subtitle => Some(TrackKind::Subtitle),
        _ => None,
    }
}

impl TrackMap {
    /// Builds the map.
    ///
    /// `externals` holds the (Jellyfin index, subtitle track id) pairs returned
    /// when external subtitles were added. `external_urls` holds the
    /// (Jellyfin index, URL) pairs the caller computed, used to recognise
    /// external tracks that `externals` does not cover.
    pub(crate) fn new(
        streams: &[MediaStream],
        tracks: &[Track],
        externals: &[(i32, TrackId)],
        external_urls: &[(i32, String)],
    ) -> TrackMap {
        let offset = streams.iter().filter(|s| s.is_external).count() as i64;
        let kind_at = |index: i32| streams.iter().find(|s| s.index == index).and_then(kind_of);
        let mut to_mpv = HashMap::new();
        let mut to_jellyfin = HashMap::new();
        for track in tracks {
            if !matches!(track.kind, TrackKind::Audio | TrackKind::Subtitle) {
                continue;
            }
            let index = match (track.stream_index, track.is_external) {
                (Some(ff), false) => i32::try_from(ff + offset).ok(),
                _ => externals
                    .iter()
                    .find(|(_, id)| track.kind == TrackKind::Subtitle && *id == track.id)
                    .map(|(index, _)| *index)
                    .or_else(|| {
                        let url = track.external_url.as_deref()?;
                        external_urls
                            .iter()
                            .find(|(_, candidate)| candidate == url)
                            .map(|(index, _)| *index)
                    }),
            };
            let Some(index) = index.filter(|i| kind_at(*i) == Some(track.kind)) else {
                continue;
            };
            to_mpv.insert((track.kind, index), track.id);
            to_jellyfin.insert((track.kind, track.id), index);
        }
        TrackMap {
            streams: streams.to_vec(),
            tracks: tracks.to_vec(),
            to_mpv,
            to_jellyfin,
        }
    }

    /// The mpv audio track for a Jellyfin stream index.
    pub(crate) fn mpv_audio(&self, index: i32) -> Option<TrackId> {
        self.to_mpv.get(&(TrackKind::Audio, index)).copied()
    }

    /// The mpv subtitle track for a Jellyfin stream index.
    pub(crate) fn mpv_subtitle(&self, index: i32) -> Option<TrackId> {
        self.to_mpv.get(&(TrackKind::Subtitle, index)).copied()
    }

    /// The Jellyfin stream index of an mpv track.
    pub(crate) fn jellyfin_index(&self, kind: TrackKind, id: TrackId) -> Option<i32> {
        self.to_jellyfin.get(&(kind, id)).copied()
    }

    fn selected(&self, kind: TrackKind, index: i32) -> bool {
        self.to_mpv
            .get(&(kind, index))
            .and_then(|id| self.tracks.iter().find(|t| t.kind == kind && t.id == *id))
            .is_some_and(|track| track.is_selected)
    }

    /// The audio tracks of the media source, in server order.
    pub(crate) fn audio_options(&self) -> Vec<AudioOption> {
        self.streams
            .iter()
            .filter(|s| s.r#type == MediaStreamType::Audio)
            .map(|s| AudioOption {
                index: s.index,
                label: label(s),
                language: s.language.clone(),
                channels: s.channels,
                selected: self.selected(TrackKind::Audio, s.index),
            })
            .collect()
    }

    /// The subtitle tracks of the media source, in server order.
    pub(crate) fn subtitle_options(&self) -> Vec<SubtitleOption> {
        self.streams
            .iter()
            .filter(|s| s.r#type == MediaStreamType::Subtitle)
            .map(|s| SubtitleOption {
                index: s.index,
                label: label(s),
                language: s.language.clone(),
                forced: s.is_forced,
                external: s.is_external,
                selected: self.selected(TrackKind::Subtitle, s.index),
            })
            .collect()
    }
}

fn label(stream: &MediaStream) -> String {
    let non_empty = |text: &Option<String>| text.clone().filter(|text| !text.is_empty());
    non_empty(&stream.display_title)
        .or_else(|| non_empty(&stream.title))
        .or_else(|| stream.language.clone())
        .unwrap_or_default()
}

/// The file extension mpv and the server use for a subtitle codec.
pub(crate) fn subtitle_format(codec: Option<&str>) -> &str {
    match codec {
        Some("subrip") => "srt",
        Some("webvtt") => "vtt",
        Some("ass") => "ass",
        Some("ssa") => "ssa",
        _ => "srt",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyfin_api::PlaybackInfoResponse;

    fn stream(index: i32, r#type: MediaStreamType, external: bool) -> MediaStream {
        MediaStream {
            index,
            r#type,
            is_external: external,
            ..MediaStream::default()
        }
    }

    fn track(id: i64, kind: TrackKind, ff: Option<i64>) -> Track {
        Track {
            id: TrackId(id),
            kind,
            language: None,
            title: None,
            codec: None,
            channels: None,
            is_default: false,
            is_forced: false,
            is_external: ff.is_none(),
            is_selected: false,
            stream_index: ff,
            external_url: None,
        }
    }

    #[test]
    fn fixture_default_audio_maps_to_english() {
        let info: PlaybackInfoResponse =
            serde_json::from_str(include_str!("../../tests/fixtures/playback_info.json")).unwrap();
        let streams = &info.media_sources[0].media_streams;
        let tracks = [
            track(1, TrackKind::Video, Some(0)),
            track(1, TrackKind::Audio, Some(1)),
            track(2, TrackKind::Audio, Some(2)),
        ];
        let map = TrackMap::new(streams, &tracks, &[], &[]);
        assert_eq!(map.mpv_audio(2), Some(TrackId(1)));
        assert_eq!(map.mpv_audio(3), Some(TrackId(2)));
    }

    #[test]
    fn no_external_streams_keep_indexes() {
        let streams = [
            stream(0, MediaStreamType::Video, false),
            stream(1, MediaStreamType::Audio, false),
        ];
        let tracks = [
            track(1, TrackKind::Video, Some(0)),
            track(1, TrackKind::Audio, Some(1)),
        ];
        let map = TrackMap::new(&streams, &tracks, &[], &[]);
        assert_eq!(map.mpv_audio(1), Some(TrackId(1)));
    }

    #[test]
    fn two_external_streams_shift_by_two() {
        let streams = [
            stream(0, MediaStreamType::Subtitle, true),
            stream(1, MediaStreamType::Subtitle, true),
            stream(2, MediaStreamType::Video, false),
            stream(3, MediaStreamType::Audio, false),
        ];
        let tracks = [
            track(1, TrackKind::Video, Some(0)),
            track(1, TrackKind::Audio, Some(1)),
        ];
        let map = TrackMap::new(&streams, &tracks, &[], &[]);
        assert_eq!(map.mpv_audio(3), Some(TrackId(1)));
        assert_eq!(map.mpv_audio(1), None);
    }

    #[test]
    fn mismatched_kind_is_ignored() {
        let streams = [
            stream(0, MediaStreamType::Video, false),
            stream(1, MediaStreamType::Subtitle, false),
        ];
        let tracks = [track(1, TrackKind::Audio, Some(1))];
        let map = TrackMap::new(&streams, &tracks, &[], &[]);
        assert_eq!(map.mpv_audio(1), None);
        assert_eq!(map.mpv_subtitle(1), None);
        assert_eq!(map.jellyfin_index(TrackKind::Audio, TrackId(1)), None);
    }

    #[test]
    fn external_maps_by_added_id() {
        let streams = [
            stream(0, MediaStreamType::Subtitle, true),
            stream(1, MediaStreamType::Audio, false),
        ];
        let tracks = [
            track(1, TrackKind::Audio, Some(0)),
            track(1, TrackKind::Subtitle, None),
        ];
        let map = TrackMap::new(&streams, &tracks, &[(0, TrackId(1))], &[]);
        assert_eq!(map.mpv_subtitle(0), Some(TrackId(1)));
        assert_eq!(map.mpv_audio(1), Some(TrackId(1)));

        let mut by_url = track(5, TrackKind::Subtitle, None);
        by_url.external_url = Some("http://x/sub.srt".to_owned());
        let map = TrackMap::new(
            &streams,
            &[by_url],
            &[],
            &[(0, "http://x/sub.srt".to_owned())],
        );
        assert_eq!(map.mpv_subtitle(0), Some(TrackId(5)));
    }

    #[test]
    fn reverse_mapping_for_reports() {
        let streams = [
            stream(0, MediaStreamType::Subtitle, true),
            stream(1, MediaStreamType::Video, false),
            stream(2, MediaStreamType::Audio, false),
        ];
        let mut audio = track(1, TrackKind::Audio, Some(1));
        audio.is_selected = true;
        let tracks = [audio, track(1, TrackKind::Subtitle, None)];
        let map = TrackMap::new(&streams, &tracks, &[(0, TrackId(1))], &[]);
        assert_eq!(map.jellyfin_index(TrackKind::Audio, TrackId(1)), Some(2));
        assert_eq!(map.jellyfin_index(TrackKind::Subtitle, TrackId(1)), Some(0));
        let audio = map.audio_options();
        assert_eq!(audio.len(), 1);
        assert!(audio[0].selected);
        let subs = map.subtitle_options();
        assert!(subs[0].external && !subs[0].selected);
    }

    #[test]
    fn label_skips_empty_titles() {
        let mut s = stream(0, MediaStreamType::Audio, false);
        s.display_title = Some(String::new());
        s.title = Some(String::new());
        s.language = Some("fra".to_owned());
        assert_eq!(label(&s), "fra");
        s.title = Some("Commentary".to_owned());
        assert_eq!(label(&s), "Commentary");
    }

    #[test]
    fn subtitle_formats() {
        assert_eq!(subtitle_format(Some("subrip")), "srt");
        assert_eq!(subtitle_format(Some("webvtt")), "vtt");
        assert_eq!(subtitle_format(Some("ass")), "ass");
        assert_eq!(subtitle_format(Some("ssa")), "ssa");
        assert_eq!(subtitle_format(Some("pgs")), "srt");
        assert_eq!(subtitle_format(None), "srt");
    }
}
