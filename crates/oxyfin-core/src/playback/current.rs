//! State of the item being played.

use std::time::Duration;

use oxyfin_api::{ItemId, MediaSourceInfo, PlaySessionId, PlaybackReport, Ticks};
use oxyfin_player::{Track, TrackId, TrackKind};

use super::tracks::TrackMap;
use super::{AudioOption, SubtitleOption};
use crate::models::Episode;

/// A last position at most this far from the duration counts as the real end.
const END_MARGIN: Duration = Duration::from_secs(10);
/// A last position past this fraction of the duration counts as the real end.
const END_FRACTION: f64 = 0.95;

/// The items around the current one.
#[derive(Default)]
pub(super) struct Siblings {
    pub(super) previous: Option<Episode>,
    pub(super) next: Option<Episode>,
}

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
    /// Position at which the media was loaded.
    pub(super) start: Duration,
    pub(super) paused: bool,
    pub(super) seek_pending: bool,
    pub(super) duration: Option<Duration>,
    /// The media reached its real end; reset by each load since a load makes a new `Current`.
    pub(super) ended: bool,
    pub(super) siblings: Siblings,
    /// The default selection asked of mpv and not yet seen in a track list:
    /// until it is, track lists show mpv's earlier choice and are ignored.
    pub(super) expected_audio: Option<TrackId>,
    pub(super) expected_subtitle: Option<Option<TrackId>>,
}

impl Current {
    pub(super) fn new(
        item: ItemId,
        source: MediaSourceInfo,
        play_session_id: Option<PlaySessionId>,
        position: Duration,
        siblings: Siblings,
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
            start: position,
            paused: false,
            seek_pending: false,
            duration: None,
            ended: false,
            siblings,
            expected_audio: None,
            expected_subtitle: None,
        }
    }

    /// Whether `tracks` shows the default selection asked of mpv; once it
    /// does, every later track list counts. An expected track missing from
    /// the list is no longer awaited, since mpv will never select it.
    pub(super) fn selection_settled(&mut self, tracks: &[Track]) -> bool {
        let listed = |kind: TrackKind, id: TrackId| {
            tracks
                .iter()
                .any(|track| track.kind == kind && track.id == id)
        };
        let selected = |kind: TrackKind| {
            tracks
                .iter()
                .find(|track| track.kind == kind && track.is_selected)
                .map(|track| track.id)
        };
        self.expected_audio = self
            .expected_audio
            .filter(|id| listed(TrackKind::Audio, *id) && selected(TrackKind::Audio) != Some(*id));
        self.expected_subtitle = self.expected_subtitle.filter(|expected| {
            let possible = expected.is_none_or(|id| listed(TrackKind::Subtitle, id));
            possible && selected(TrackKind::Subtitle) != *expected
        });
        self.expected_audio.is_none() && self.expected_subtitle.is_none()
    }

    /// Stops waiting for the default selection once playback has moved from
    /// where it started: mpv has applied the selection by then, or never will.
    pub(super) fn position_moved(&mut self, position: Duration) {
        if self.loaded && position != self.start {
            self.expected_audio = None;
            self.expected_subtitle = None;
        }
    }

    /// A user pick replaces the default selection of that kind.
    pub(super) fn forget_expected(&mut self, kind: TrackKind) {
        match kind {
            TrackKind::Audio => self.expected_audio = None,
            TrackKind::Subtitle => self.expected_subtitle = None,
            TrackKind::Video => {}
        }
    }

    /// Whether the server session is open: started and not ended.
    pub(super) fn reporting(&self) -> bool {
        self.loaded && !self.ended
    }

    /// Whether the last position is close enough to the duration to count as
    /// the end of the item; true when the duration is unknown.
    pub(super) fn at_real_end(&self) -> bool {
        let Some(duration) = self.duration else {
            return true;
        };
        self.position + END_MARGIN >= duration
            || self.position.as_secs_f64() >= duration.as_secs_f64() * END_FRACTION
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
