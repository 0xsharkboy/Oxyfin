//! Reading the track list of the loaded media.

use libmpv2::Mpv;

use crate::{PlayerError, Track, TrackId, TrackKind};

/// Reads the tracks of the loaded media.
pub(crate) fn read(mpv: &Mpv) -> Result<Vec<Track>, PlayerError> {
    let count: i64 = property(mpv, "track-list/count")?;
    let mut tracks = Vec::new();
    for index in 0..count {
        let kind: String = property(mpv, &format!("track-list/{index}/type"))?;
        let kind = match kind.as_str() {
            "video" => TrackKind::Video,
            "audio" => TrackKind::Audio,
            "sub" => TrackKind::Subtitle,
            _ => continue,
        };
        let id: i64 = property(mpv, &format!("track-list/{index}/id"))?;
        tracks.push(Track {
            id: TrackId(id),
            kind,
            language: None,
            title: None,
            codec: None,
            channels: None,
            is_default: false,
            is_forced: false,
            is_external: false,
            is_selected: false,
            stream_index: None,
            external_url: None,
        });
    }
    Ok(tracks)
}

fn property<T: libmpv2::GetData>(mpv: &Mpv, name: &str) -> Result<T, PlayerError> {
    mpv.get_property(name).map_err(|e| PlayerError::Property {
        name: name.to_owned(),
        message: e.to_string(),
    })
}
