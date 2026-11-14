//! State of the item being played.

use std::time::Duration;

use oxyfin_api::{ItemId, MediaSourceInfo, PlaySessionId, PlaybackReport, Ticks};
use oxyfin_player::{Track, TrackId, TrackKind};

use super::tracks::TrackMap;
use super::{AudioOption, SubtitleOption};

/// State of the item being played.
pub(super) struct Current {
    pub(super) item: ItemId,
    pub(super) source: MediaSourceInfo,
    pub(super) play_session_id: Option<PlaySessionId>,
    pub(super) loaded: bool,
    pub(super) map: Option<TrackMap>,
    pub(super) externals: Vec<(i32, TrackId)>,
    pub(super) external_urls: Vec<(i32, String)>,
    pub(super) audio: Option<i32>,
    pub(super) subtitle: Option<i32>,
    pub(super) options: (Vec<AudioOption>, Vec<SubtitleOption>),
    pub(super) position: Duration,
    pub(super) paused: bool,
    pub(super) seek_pending: bool,
}

impl Current {
    pub(super) fn new(
        item: ItemId,
        source: MediaSourceInfo,
        play_session_id: Option<PlaySessionId>,
        position: Duration,
    ) -> Self {
        Self {
            item,
            source,
            play_session_id,
            loaded: false,
            map: None,
            externals: Vec::new(),
            external_urls: Vec::new(),
            audio: None,
            subtitle: None,
            options: (Vec::new(), Vec::new()),
            position,
            paused: false,
            seek_pending: false,
        }
    }

    pub(super) fn report(&self) -> PlaybackReport {
        PlaybackReport {
            item_id: self.item.clone(),
            media_source_id: self.source.id.clone(),
            play_session_id: self.play_session_id.clone(),
            position: Ticks::from_duration(self.position),
            is_paused: self.paused,
            audio_stream_index: self.audio,
            subtitle_stream_index: self.subtitle,
        }
    }

    /// Rebuilds the track map from `tracks`; returns whether the selection changed.
    pub(super) fn update_tracks(&mut self, tracks: &[Track]) -> bool {
        let map = TrackMap::new(
            &self.source.media_streams,
            tracks,
            &self.externals,
            &self.external_urls,
        );
        let selected = |kind: TrackKind| {
            tracks
                .iter()
                .find(|track| track.kind == kind && track.is_selected)
                .and_then(|track| map.jellyfin_index(kind, track.id))
        };
        let selection = (selected(TrackKind::Audio), selected(TrackKind::Subtitle));
        let changed = selection != (self.audio, self.subtitle);
        (self.audio, self.subtitle) = selection;
        self.map = Some(map);
        changed
    }

    /// Updates the track options; returns them when they changed.
    pub(super) fn refresh_options(&mut self) -> Option<(Vec<AudioOption>, Vec<SubtitleOption>)> {
        let map = self.map.as_ref()?;
        let options = (map.audio_options(), map.subtitle_options());
        if options == self.options {
            return None;
        }
        self.options = options.clone();
        Some(options)
    }
}

pub(super) fn mark_selected(tracks: &mut [Track], kind: TrackKind, id: Option<TrackId>) {
    for track in tracks.iter_mut().filter(|track| track.kind == kind) {
        track.is_selected = Some(track.id) == id;
    }
}
