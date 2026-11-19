//! Chapters of the playing item and the chapter jumps of the player keys.

use std::time::Duration;

use oxyfin_api::{BaseItemDto, ImageType, Ticks};

use crate::models::ImageRef;

/// How long after a chapter start a jump back restarts the previous chapter
/// instead of the current one.
pub const CHAPTER_RESTART_GRACE: Duration = Duration::from_secs(3);

/// A chapter of the playing item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    /// Start of the chapter.
    pub start: Duration,
    /// Name of the chapter, empty when the media has none.
    pub name: String,
    /// Image of the chapter, when the server extracted one.
    pub image: Option<ImageRef>,
}

/// The chapters of `dto`, ordered by start; empty when it has fewer than
/// two, since a single chapter splits nothing.
pub(super) fn chapters(dto: &BaseItemDto) -> Vec<Chapter> {
    if dto.chapters.len() < 2 {
        return Vec::new();
    }
    let mut chapters: Vec<Chapter> = dto
        .chapters
        .iter()
        .enumerate()
        .map(|(index, info)| Chapter {
            start: Ticks(info.start_position_ticks).to_duration(),
            name: info.name.clone().unwrap_or_default(),
            image: info
                .image_tag
                .clone()
                .zip(u32::try_from(index).ok())
                .map(|(tag, index)| ImageRef {
                    item_id: dto.id.clone(),
                    image_type: ImageType::Chapter,
                    tag,
                    index: Some(index),
                }),
        })
        .collect();
    chapters.sort_by_key(|chapter| chapter.start);
    chapters
}

/// Index of the chapter playing at `position`: the last one started, `None`
/// before the first one.
pub fn chapter_at(chapters: &[Chapter], position: Duration) -> Option<usize> {
    chapters
        .iter()
        .rposition(|chapter| chapter.start <= position)
}

/// Index of the chapter playing at `position`, given the chapter starts in
/// order: like [`chapter_at`] without the rest of each chapter.
pub fn start_at(starts: &[Duration], position: Duration) -> Option<usize> {
    starts.iter().rposition(|start| *start <= position)
}

/// Index of the chapter after the one at `position`, given the chapter
/// starts in order.
pub fn next_chapter(starts: &[Duration], position: Duration) -> Option<usize> {
    starts.iter().position(|start| *start > position)
}

/// Index of the chapter a jump back from `position` goes to, like mpv: the
/// current chapter, or the previous one within [`CHAPTER_RESTART_GRACE`] of
/// the current start.
pub fn previous_chapter(starts: &[Duration], position: Duration) -> Option<usize> {
    let current = start_at(starts, position)?;
    let restarted = starts
        .get(current)
        .is_some_and(|start| position < *start + CHAPTER_RESTART_GRACE);
    Some(if restarted {
        current.saturating_sub(1)
    } else {
        current
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyfin_api::ChapterInfo;

    fn info(ticks: i64, tag: Option<&str>) -> ChapterInfo {
        ChapterInfo {
            start_position_ticks: ticks,
            name: None,
            image_tag: tag.map(str::to_owned),
        }
    }

    #[test]
    fn image_index_is_the_server_position() {
        let dto = BaseItemDto {
            id: "abc".into(),
            chapters: vec![info(0, None), info(10, Some("t"))],
            ..BaseItemDto::default()
        };
        let chapters = chapters(&dto);
        assert_eq!(chapters[0].image, None);
        assert_eq!(chapters[1].image.as_ref().and_then(|i| i.index), Some(1));
        assert_eq!(chapters[1].start, Duration::from_micros(1));
    }
}
