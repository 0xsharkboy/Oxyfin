//! Trickplay thumbnails: which tile of which sheet shows a given time.

use std::collections::BTreeMap;
use std::time::Duration;

use oxyfin_api::{BaseItemDto, ItemId, MediaSourceId, TrickplayInfoDto};

/// Thumbnail width preferred among the resolutions of an item, in pixels:
/// the smallest one at least this wide, or else the widest.
pub const TRICKPLAY_WIDTH: u32 = 320;

/// The trickplay thumbnails of a media source, at one resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trickplay {
    /// Item the thumbnails belong to.
    pub item: ItemId,
    /// Media source the thumbnails were made from.
    pub media_source: MediaSourceId,
    /// Resolution, as the server names it: the width of a thumbnail in pixels.
    pub resolution: u32,
    /// Width of a thumbnail in a sheet, in pixels.
    pub width: u32,
    /// Height of a thumbnail in a sheet, in pixels.
    pub height: u32,
    /// Thumbnails per row of a sheet.
    pub columns: u32,
    /// Rows of thumbnails per sheet.
    pub rows: u32,
    /// Number of thumbnails of the whole video.
    pub count: u32,
    /// Time between two thumbnails.
    pub interval: Duration,
}

/// One tile sheet of a [`Trickplay`]: a JPEG of `columns` by `rows` thumbnails.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TrickplaySheet {
    /// Item the sheet belongs to.
    pub item: ItemId,
    /// Media source the sheet was made from.
    pub media_source: MediaSourceId,
    /// Resolution of the thumbnails, as the server names it.
    pub resolution: u32,
    /// Index of the sheet, from 0.
    pub index: u32,
    /// Layout of the set the sheet belongs to, so that a set generated again
    /// differently is not mixed up with the old one: interval in
    /// milliseconds, columns and rows.
    pub layout: (u32, u32, u32),
}

/// Where the thumbnail of a time lies: a sheet, and a rectangle in it in pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrickplayTile {
    /// Sheet holding the thumbnail.
    pub sheet: TrickplaySheet,
    /// Left edge of the thumbnail in the sheet.
    pub x: u32,
    /// Top edge of the thumbnail in the sheet.
    pub y: u32,
    /// Width of the thumbnail.
    pub width: u32,
    /// Height of the thumbnail.
    pub height: u32,
}

impl Trickplay {
    /// The thumbnails of `media_source` of `dto` at the resolution closest
    /// to [`TRICKPLAY_WIDTH`]; `None` when the server has no usable ones.
    pub fn from_dto(dto: &BaseItemDto, media_source: &MediaSourceId) -> Option<Self> {
        let (resolution, info) = pick(dto.trickplay_for(media_source)?)?;
        Some(Self {
            item: dto.id.clone(),
            media_source: media_source.clone(),
            resolution,
            width: info.width,
            height: info.height,
            columns: info.tile_width,
            rows: info.tile_height,
            count: info
                .thumbnail_count
                .max(thumbnails_over(dto, info.interval)),
            interval: Duration::from_millis(u64::from(info.interval)),
        })
    }

    /// The thumbnail to show for `position`: the last one taken at or
    /// before it, and the last one of all past the end.
    pub fn tile_at(&self, position: Duration) -> TrickplayTile {
        let interval = self.interval.as_millis().max(1);
        let taken = position.as_millis() / interval;
        let last = self.count.saturating_sub(1);
        let number = u32::try_from(taken).unwrap_or(u32::MAX).min(last);
        let per_sheet = (self.columns * self.rows).max(1);
        let within = number % per_sheet;
        let columns = self.columns.max(1);
        TrickplayTile {
            sheet: self.sheet(number / per_sheet),
            x: within % columns * self.width,
            y: within / columns * self.height,
            width: self.width,
            height: self.height,
        }
    }

    /// Sheet `index` of these thumbnails.
    pub fn sheet(&self, index: u32) -> TrickplaySheet {
        let interval = u32::try_from(self.interval.as_millis()).unwrap_or(u32::MAX);
        TrickplaySheet {
            item: self.item.clone(),
            media_source: self.media_source.clone(),
            resolution: self.resolution,
            index,
            layout: (interval, self.columns, self.rows),
        }
    }
}

/// Whole intervals in the run time of `dto`: thumbnails the server surely
/// took, one every `interval` milliseconds from the start; 0 when the run
/// time is unknown. Some servers report the number of sheets as the
/// thumbnail count, so the count is checked against it.
fn thumbnails_over(dto: &BaseItemDto, interval: u32) -> u32 {
    const TICKS_PER_MILLISECOND: u64 = 10_000;
    let Some(ticks) = dto
        .run_time_ticks
        .and_then(|ticks| u64::try_from(ticks.0).ok())
    else {
        return 0;
    };
    let interval = u64::from(interval.max(1)) * TICKS_PER_MILLISECOND;
    u32::try_from(ticks / interval).unwrap_or(u32::MAX)
}

fn usable(info: &TrickplayInfoDto) -> bool {
    [
        info.width,
        info.height,
        info.tile_width,
        info.tile_height,
        info.thumbnail_count,
        info.interval,
    ]
    .iter()
    .all(|value| *value > 0)
}

fn pick(widths: &BTreeMap<u32, TrickplayInfoDto>) -> Option<(u32, &TrickplayInfoDto)> {
    let mut candidates = widths
        .iter()
        .filter(|(_, info)| usable(info))
        .map(|(resolution, info)| (*resolution, info));
    candidates
        .clone()
        .find(|(resolution, _)| *resolution >= TRICKPLAY_WIDTH)
        .or_else(|| candidates.next_back())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ITEM: &str = "af282b1c408f374beeee50519e9157a4";

    fn info(width: u32) -> serde_json::Value {
        json!({"Width": width, "Height": width * 9 / 16, "TileWidth": 10, "TileHeight": 10,
               "ThumbnailCount": 250, "Interval": 10000, "Bandwidth": 1})
    }

    fn dto(widths: serde_json::Value) -> BaseItemDto {
        serde_json::from_value(json!({"Id": ITEM, "Type": "Movie", "Trickplay": {ITEM: widths}}))
            .unwrap()
    }

    fn source() -> MediaSourceId {
        MediaSourceId::from(ITEM)
    }

    fn resolution(widths: serde_json::Value) -> Option<u32> {
        Trickplay::from_dto(&dto(widths), &source()).map(|trickplay| trickplay.resolution)
    }

    fn movie() -> Trickplay {
        Trickplay {
            item: ItemId::from(ITEM),
            media_source: source(),
            resolution: 320,
            width: 320,
            height: 180,
            columns: 10,
            rows: 10,
            count: 250,
            interval: Duration::from_secs(10),
        }
    }

    #[test]
    fn picks_the_smallest_resolution_at_least_the_preferred_width() {
        assert_eq!(
            resolution(json!({"160": info(160), "320": info(320), "640": info(640)})),
            Some(320)
        );
        assert_eq!(
            resolution(json!({"480": info(480), "640": info(640)})),
            Some(480)
        );
    }

    #[test]
    fn falls_back_on_the_widest_smaller_resolution() {
        assert_eq!(
            resolution(json!({"120": info(120), "160": info(160)})),
            Some(160)
        );
    }

    #[test]
    fn unusable_or_missing_thumbnails_give_none() {
        let mut empty = info(320);
        empty["ThumbnailCount"] = json!(0);
        assert_eq!(resolution(json!({"320": empty})), None);
        let mut instant = info(320);
        instant["Interval"] = json!(0);
        assert_eq!(
            resolution(json!({"320": instant, "160": info(160)})),
            Some(160)
        );
        assert_eq!(resolution(json!({})), None);
        let other = dto(json!({"320": info(320)}));
        assert!(Trickplay::from_dto(&other, &MediaSourceId::from("other")).is_none());
    }

    #[test]
    fn reads_the_layout_of_the_picked_resolution() {
        let trickplay = Trickplay::from_dto(&dto(json!({"320": info(320)})), &source()).unwrap();
        assert_eq!(trickplay, movie());
    }

    #[test]
    fn tiles_go_row_by_row_then_sheet_by_sheet() {
        let trickplay = movie();
        let at = |secs: u64| {
            let tile = trickplay.tile_at(Duration::from_secs(secs));
            (tile.sheet.index, tile.x, tile.y)
        };
        assert_eq!(at(0), (0, 0, 0));
        assert_eq!(at(9), (0, 0, 0));
        assert_eq!(at(10), (0, 320, 0));
        assert_eq!(at(95), (0, 2880, 0));
        assert_eq!(at(100), (0, 0, 180));
        assert_eq!(at(999), (0, 2880, 1620));
        assert_eq!(at(1000), (1, 0, 0));
        assert_eq!(at(1234), (1, 960, 360));
    }

    #[test]
    fn past_the_last_thumbnail_shows_the_last_one() {
        let trickplay = movie();
        let last = trickplay.tile_at(Duration::from_secs(2490));
        assert_eq!((last.sheet.index, last.x, last.y), (2, 2880, 720));
        assert_eq!(trickplay.tile_at(Duration::from_secs(100_000)), last);
    }

    fn with_count(count: u32, run_time: Option<u64>) -> Trickplay {
        let mut info = info(320);
        info["ThumbnailCount"] = json!(count);
        let mut item = json!({"Id": ITEM, "Type": "Movie", "Trickplay": {ITEM: {"320": info}}});
        if let Some(seconds) = run_time {
            item["RunTimeTicks"] = json!(seconds * 10_000_000);
        }
        let dto: BaseItemDto = serde_json::from_value(item).unwrap();
        Trickplay::from_dto(&dto, &source()).unwrap()
    }

    #[test]
    fn run_time_overrides_a_thumbnail_count_that_counts_sheets() {
        let trickplay = with_count(5, Some(4920));
        assert_eq!(trickplay.count, 492);
        let tile = trickplay.tile_at(Duration::from_secs(45 * 60));
        assert_eq!((tile.sheet.index, tile.x, tile.y), (2, 0, 7 * 180));
    }

    #[test]
    fn unknown_run_time_keeps_the_server_count() {
        assert_eq!(with_count(5, None).count, 5);
    }

    #[test]
    fn a_larger_server_count_is_kept() {
        assert_eq!(with_count(742, Some(7000)).count, 742);
    }

    #[test]
    fn a_tile_names_its_sheet_and_size() {
        let tile = movie().tile_at(Duration::from_millis(20_500));
        assert_eq!((tile.width, tile.height), (320, 180));
        assert_eq!(
            tile.sheet,
            TrickplaySheet {
                item: ItemId::from(ITEM),
                media_source: source(),
                resolution: 320,
                index: 0,
                layout: (10_000, 10, 10),
            }
        );
    }
}
