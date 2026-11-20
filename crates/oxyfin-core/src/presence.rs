//! What a chat app such as Discord shows of the item playing.
//!
//! Built only from the titles of the item, its position and an optional
//! public poster: never from the server address, item ids or user names.

use std::time::Duration;

use serde::Serialize;

use crate::playback::PlaybackTitle;

/// Image key of the Oxyfin art uploaded with the Discord application.
pub const PRESENCE_ASSET: &str = "oxyfin";
/// Only posters under this address are shown: a public image, unrelated to the server.
pub const TMDB_IMAGES: &str = "https://image.tmdb.org/";
/// Longest text Discord accepts in an activity field, in characters.
pub const PRESENCE_TEXT_LIMIT: usize = 128;

/// An activity to show, as Discord takes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Presence {
    /// First line: the series name, or the title.
    pub details: String,
    /// Second line: the episode or the year, suffixed while paused. May be empty.
    pub state: String,
    /// When playback started from the beginning, in Unix seconds; `None` while paused.
    pub start: Option<i64>,
    /// When playback will end, in Unix seconds; `None` while paused or when the duration is unknown.
    pub end: Option<i64>,
    /// A public poster URL, or [`PRESENCE_ASSET`].
    pub large_image: String,
    /// Text shown over the image.
    pub large_text: String,
}

/// Builds the activity for `title` played at `position` out of `duration`, at
/// Unix time `now` in seconds. `poster` is used only when it is a public TMDB
/// image, else the Oxyfin art stands for the item.
pub fn presence_for(
    title: &PlaybackTitle,
    position: Duration,
    duration: Option<Duration>,
    paused: bool,
    poster: Option<String>,
    now: i64,
) -> Presence {
    let details = title
        .series_name
        .clone()
        .unwrap_or_else(|| title.title.clone());
    let mut state = match &title.episode_label {
        Some(label) if title.series_name.is_some() => format!("{label} · {}", title.title),
        _ => title.year.map(|year| year.to_string()).unwrap_or_default(),
    };
    if paused {
        state = if state.is_empty() {
            "Paused".to_owned()
        } else {
            format!("{state} · Paused")
        };
    }
    let (start, end) = if paused {
        (None, None)
    } else {
        let start = now.saturating_sub(seconds(position));
        (
            Some(start),
            duration.map(|duration| start.saturating_add(seconds(duration))),
        )
    };
    let poster = poster.filter(|url| url.starts_with(TMDB_IMAGES));
    let large_text = if poster.is_some() {
        details.clone()
    } else {
        "Oxyfin".to_owned()
    };
    Presence {
        details: fit(details),
        state: fit(state),
        start,
        end,
        large_image: poster.unwrap_or_else(|| PRESENCE_ASSET.to_owned()),
        large_text: fit(large_text),
    }
}

fn seconds(duration: Duration) -> i64 {
    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
}

fn fit(text: String) -> String {
    if text.chars().count() <= PRESENCE_TEXT_LIMIT {
        return text;
    }
    let mut cut: String = text.chars().take(PRESENCE_TEXT_LIMIT - 1).collect();
    cut.push('…');
    cut
}
