//! Reading the track list of the loaded media.

use libmpv2::Mpv;

use crate::{PlayerError, Track, TrackId, TrackKind};

/// Reads the tracks of the loaded media.
///
/// Only the track count is required: a missing per-track property yields
/// `None` or `false`.
pub(crate) fn read(mpv: &Mpv) -> Result<Vec<Track>, PlayerError> {
    let count: i64 = mpv
        .get_property("track-list/count")
        .map_err(|e| PlayerError::Property {
            name: "track-list/count".to_owned(),
            message: e.to_string(),
        })?;
    Ok((0..count)
        .filter_map(|index| read_track(mpv, index))
        .collect())
}

fn read_track(mpv: &Mpv, index: i64) -> Option<Track> {
    let get = |field: &str| format!("track-list/{index}/{field}");
    let text = |field: &str| {
        mpv.get_property::<String>(&get(field))
            .ok()
            .filter(|value| !value.is_empty())
    };
    let flag = |field: &str| mpv.get_property::<bool>(&get(field)).unwrap_or(false);
    let number = |field: &str| mpv.get_property::<i64>(&get(field)).ok();

    let kind = match text("type")?.as_str() {
        "video" => TrackKind::Video,
        "audio" => TrackKind::Audio,
        "sub" => TrackKind::Subtitle,
        _ => return None,
    };
    Some(Track {
        id: TrackId(number("id")?),
        kind,
        language: text("lang"),
        title: text("title"),
        codec: text("codec"),
        channels: number("demux-channel-count").and_then(|n| u32::try_from(n).ok()),
        is_default: flag("default"),
        is_forced: flag("forced"),
        is_external: flag("external"),
        is_selected: flag("selected"),
        stream_index: number("ff-index"),
        external_url: text("external-filename"),
    })
}
