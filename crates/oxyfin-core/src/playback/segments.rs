//! The parts of an item the user can skip, from its media segments.

use std::time::Duration;

use oxyfin_api::{ItemId, MediaSegmentDto, MediaSegmentType};

/// A segment stops being offered this long before its end, so a seek to the
/// end that lands a frame short does not offer it again.
const SEGMENT_END_MARGIN: Duration = Duration::from_secs(1);

/// What a skippable segment holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    /// The opening sequence.
    Intro,
    /// A recap of the previous episodes.
    Recap,
    /// A preview of the next episode or of the item.
    Preview,
    /// The end credits.
    Credits,
}

/// A part of the playing item the user can skip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkippableSegment {
    /// What the segment holds.
    pub kind: SegmentKind,
    /// Start of the segment.
    pub start: Duration,
    /// End of the segment, where skipping it seeks to. Never past the
    /// duration of the item, when known.
    pub end: Duration,
}

/// The segments of one item already skipped without asking: each is skipped
/// once, and only offered afterwards.
#[derive(Debug, Default)]
pub(super) struct SkipMemory {
    item: Option<ItemId>,
    /// Segments skipped, or whose skip was undone.
    done: Vec<(SegmentKind, Duration)>,
    /// Where the last skip happened, until undone.
    last: Option<Duration>,
}

impl SkipMemory {
    /// Forgets every skip unless `item` is the item remembered and `keep` is set.
    pub(super) fn start(&mut self, item: &ItemId, keep: bool) {
        if !keep || self.item.as_ref() != Some(item) {
            *self = Self {
                item: Some(item.clone()),
                ..Self::default()
            };
        }
    }

    /// Whether `segment` was not skipped yet.
    pub(super) fn fresh(&self, segment: &SkippableSegment) -> bool {
        !self.done.contains(&(segment.kind, segment.start))
    }

    /// Records that `segment` was skipped from `from`.
    pub(super) fn skipped(&mut self, segment: &SkippableSegment, from: Duration) {
        self.done.push((segment.kind, segment.start));
        self.last = Some(from);
    }

    /// Where the last skip happened, forgotten once returned.
    pub(super) fn undo(&mut self) -> Option<Duration> {
        self.last.take()
    }

    /// The user moved to `target`: the segments around it are theirs to
    /// watch, and the last skip can no longer be undone.
    pub(super) fn sought(&mut self, segments: &[SkippableSegment], target: Duration) {
        self.last = None;
        for segment in segments {
            let key = (segment.kind, segment.start);
            if segment.start <= target && target < segment.end && !self.done.contains(&key) {
                self.done.push(key);
            }
        }
    }
}

/// The segments of `segments` worth skipping, ordered by start. Unknown
/// kinds, commercials and empty segments are left out.
pub(super) fn skippable(segments: Vec<MediaSegmentDto>) -> Vec<SkippableSegment> {
    let mut found: Vec<SkippableSegment> = segments
        .into_iter()
        .filter_map(|segment| {
            let kind = match segment.r#type {
                MediaSegmentType::Intro => SegmentKind::Intro,
                MediaSegmentType::Recap => SegmentKind::Recap,
                MediaSegmentType::Preview => SegmentKind::Preview,
                MediaSegmentType::Outro => SegmentKind::Credits,
                _ => return None,
            };
            let start = segment.start_ticks.to_duration();
            let end = segment.end_ticks.to_duration();
            (end > start).then_some(SkippableSegment { kind, start, end })
        })
        .collect();
    found.sort_by_key(|segment| segment.start);
    found
}

/// The segment to offer at `position`: the first one that contains it,
/// ignoring the last [`SEGMENT_END_MARGIN`] of each. Its end is clamped to
/// `duration`.
pub(super) fn active(
    segments: &[SkippableSegment],
    position: Duration,
    duration: Option<Duration>,
) -> Option<SkippableSegment> {
    segments.iter().find_map(|segment| {
        let end = duration.map_or(segment.end, |duration| segment.end.min(duration));
        let offered = segment.start <= position && position + SEGMENT_END_MARGIN < end;
        offered.then_some(SkippableSegment { end, ..*segment })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyfin_api::Ticks;

    fn dto(kind: &str, start: u64, end: u64) -> MediaSegmentDto {
        MediaSegmentDto {
            id: String::new(),
            item_id: "item".into(),
            r#type: MediaSegmentType::from(kind.to_owned()),
            start_ticks: Ticks::from_duration(Duration::from_secs(start)),
            end_ticks: Ticks::from_duration(Duration::from_secs(end)),
        }
    }

    fn segment(kind: SegmentKind, start: u64, end: u64) -> SkippableSegment {
        SkippableSegment {
            kind,
            start: Duration::from_secs(start),
            end: Duration::from_secs(end),
        }
    }

    #[test]
    fn keeps_the_kinds_worth_skipping_in_order() {
        let found = skippable(vec![
            dto("Outro", 1300, 1400),
            dto("Commercial", 600, 700),
            dto("Unknown", 700, 800),
            dto("Sponsor", 800, 900),
            dto("Intro", 60, 150),
            dto("Recap", 0, 60),
            dto("Preview", 1400, 1440),
            dto("Intro", 200, 200),
        ]);
        assert_eq!(
            found,
            vec![
                segment(SegmentKind::Recap, 0, 60),
                segment(SegmentKind::Intro, 60, 150),
                segment(SegmentKind::Credits, 1300, 1400),
                segment(SegmentKind::Preview, 1400, 1440),
            ]
        );
    }

    #[test]
    fn offers_a_segment_from_its_start_until_shortly_before_its_end() {
        let segments = [segment(SegmentKind::Intro, 60, 150)];
        let at = |secs: f64| active(&segments, Duration::from_secs_f64(secs), None);
        assert_eq!(at(59.9), None);
        assert_eq!(at(60.0), Some(segments[0]));
        assert_eq!(at(148.9), Some(segments[0]));
        assert_eq!(at(149.0), None);
        assert_eq!(at(149.96), None);
        assert_eq!(at(150.0), None);
    }

    #[test]
    fn back_to_back_segments_hand_over() {
        let segments = [
            segment(SegmentKind::Recap, 0, 60),
            segment(SegmentKind::Intro, 60, 150),
        ];
        let at = |secs: u64| active(&segments, Duration::from_secs(secs), None);
        assert_eq!(at(10).map(|s| s.kind), Some(SegmentKind::Recap));
        assert_eq!(at(59), None);
        assert_eq!(at(60).map(|s| s.kind), Some(SegmentKind::Intro));
    }

    #[test]
    fn credits_past_the_end_stop_at_the_duration() {
        let segments = [segment(SegmentKind::Credits, 1300, 1500)];
        let duration = Some(Duration::from_secs(1400));
        let at = |secs: u64| active(&segments, Duration::from_secs(secs), duration);
        assert_eq!(at(1350), Some(segment(SegmentKind::Credits, 1300, 1400)));
        assert_eq!(at(1399), None);
        assert_eq!(at(1400), None);
    }
}
