//! Media segments: intros, recaps, credits and the like.

use super::enums::MediaSegmentType;
use crate::{ItemId, Ticks};
use serde::Deserialize;

/// A typed span of an item, such as its intro.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSegmentDto {
    /// Identifier of the segment.
    pub id: String,
    /// Item the segment belongs to.
    pub item_id: ItemId,
    /// Kind of content the segment spans.
    pub r#type: MediaSegmentType,
    /// Start of the segment.
    pub start_ticks: Ticks,
    /// End of the segment.
    pub end_ticks: Ticks,
}
