//! The audio and subtitle choices made in a series, kept for its next
//! episodes and matched by description, never by stream index.

use std::fs;

use oxyfin_api::{ItemId, MediaStream, MediaStreamType};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;
use crate::{CoreError, Result};

/// Most series remembered; the least recently changed are dropped first.
pub const TRACK_MEMORY_LIMIT: usize = 500;

const FILE_NAME: &str = "track-memory.json";

/// The tracks the user picked in a series.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackChoice {
    /// The audio picked, `None` while the user kept the server's.
    pub audio: Option<AudioPick>,
    /// The subtitle picked.
    pub subtitle: SubtitlePick,
}

/// Description of an audio stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioPick {
    /// Language code.
    pub language: Option<String>,
    /// Number of channels, as a layout hint.
    pub channels: Option<u32>,
}

/// Description of a subtitle choice.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubtitlePick {
    /// The user kept the server's choice.
    #[default]
    Default,
    /// No subtitle.
    Off,
    /// A subtitle stream like this one.
    Track {
        /// Language code.
        language: Option<String>,
        /// Whether it shows only the foreign parts.
        forced: bool,
        /// Whether it is meant for the deaf and hard of hearing.
        hearing_impaired: bool,
        /// Whether it comes from a separate file.
        external: bool,
    },
}

impl AudioPick {
    /// Describes `stream`.
    pub fn of(stream: &MediaStream) -> Self {
        Self {
            language: stream.language.clone(),
            channels: stream.channels,
        }
    }
}

impl SubtitlePick {
    /// Describes `stream`.
    pub fn of(stream: &MediaStream) -> Self {
        Self::Track {
            language: stream.language.clone(),
            forced: stream.is_forced,
            hearing_impaired: stream.is_hearing_impaired,
            external: stream.is_external,
        }
    }
}

/// ISO 639-2 bibliographic codes and their terminology twins.
const TWIN_CODES: [(&str, &str); 20] = [
    ("alb", "sqi"),
    ("arm", "hye"),
    ("baq", "eus"),
    ("bur", "mya"),
    ("chi", "zho"),
    ("cze", "ces"),
    ("dut", "nld"),
    ("fre", "fra"),
    ("geo", "kat"),
    ("ger", "deu"),
    ("gre", "ell"),
    ("ice", "isl"),
    ("mac", "mkd"),
    ("mao", "mri"),
    ("may", "msa"),
    ("per", "fas"),
    ("rum", "ron"),
    ("slo", "slk"),
    ("tib", "bod"),
    ("wel", "cym"),
];

fn canonical(language: &str) -> String {
    let language = language.trim().to_ascii_lowercase();
    TWIN_CODES
        .iter()
        .find(|(bibliographic, _)| *bibliographic == language)
        .map_or(language, |(_, terminology)| (*terminology).to_owned())
}

fn same_language(stream: &MediaStream, wanted: &str) -> bool {
    stream
        .language
        .as_deref()
        .is_some_and(|language| canonical(language) == canonical(wanted))
}

fn is_commentary(stream: &MediaStream) -> bool {
    [&stream.title, &stream.display_title]
        .into_iter()
        .flatten()
        .any(|title| {
            let title = title.to_lowercase();
            title.contains("commentary") || title.contains("commentaire")
        })
}

/// The first stream of `kind` in `language` with the highest score.
fn best<K: Ord>(
    streams: &[MediaStream],
    kind: MediaStreamType,
    language: Option<&str>,
    score: impl Fn(&MediaStream) -> K,
) -> Option<i32> {
    let language = language?;
    streams
        .iter()
        .rev()
        .filter(|stream| stream.r#type == kind && same_language(stream, language))
        .max_by_key(|stream| score(stream))
        .map(|stream| stream.index)
}

/// The audio stream of `streams` closest to `pick`: same language, not a
/// commentary when possible, then the nearest channel count. `None` when no
/// stream has the language.
pub fn best_audio(streams: &[MediaStream], pick: &AudioPick) -> Option<i32> {
    best(
        streams,
        MediaStreamType::Audio,
        pick.language.as_deref(),
        |stream| {
            let distance = match (stream.channels, pick.channels) {
                (Some(have), Some(want)) => have.abs_diff(want),
                _ => u32::MAX,
            };
            (!is_commentary(stream), std::cmp::Reverse(distance))
        },
    )
}

/// The subtitle to show for `pick`: `Some(None)` for none, `Some(Some(index))`
/// for the stream of the same language with the most flags in common,
/// avoiding commentaries. `None` when the server's default applies: the pick
/// is [`SubtitlePick::Default`] or no stream has the language.
pub fn best_subtitle(streams: &[MediaStream], pick: &SubtitlePick) -> Option<Option<i32>> {
    match pick {
        SubtitlePick::Default => None,
        SubtitlePick::Off => Some(None),
        SubtitlePick::Track {
            language,
            forced,
            hearing_impaired,
            external,
        } => best(
            streams,
            MediaStreamType::Subtitle,
            language.as_deref(),
            |stream| {
                (
                    stream.is_forced == *forced,
                    !is_commentary(stream),
                    stream.is_hearing_impaired == *hearing_impaired,
                    stream.is_external == *external,
                )
            },
        )
        .map(Some),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    series: ItemId,
    choice: TrackChoice,
}

/// The choices of the last [`TRACK_MEMORY_LIMIT`] series, saved in
/// `track-memory.json` of the data directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackMemory {
    paths: Paths,
    /// Least recently changed first.
    entries: Vec<Entry>,
}

fn storage<E: std::fmt::Display>(error: E) -> CoreError {
    CoreError::Storage(error.to_string())
}

impl TrackMemory {
    /// Reads the saved choices; a missing or unreadable file gives an empty memory.
    pub fn load(paths: &Paths) -> Self {
        let file = paths.data_dir.join(FILE_NAME);
        let entries = match fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                log::warn!("ignoring the unreadable track memory: {error}");
                Vec::new()
            }),
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("cannot read the track memory: {error}");
                }
                Vec::new()
            }
        };
        Self {
            paths: paths.clone(),
            entries,
        }
    }

    /// The choice saved for `series`.
    pub fn get(&self, series: &ItemId) -> Option<TrackChoice> {
        self.entries
            .iter()
            .find(|entry| entry.series == *series)
            .map(|entry| entry.choice.clone())
    }

    /// Saves `choice` for `series`, dropping the oldest series beyond the limit.
    pub fn set(&mut self, series: &ItemId, choice: TrackChoice) -> Result<()> {
        self.entries.retain(|entry| entry.series != *series);
        self.entries.push(Entry {
            series: series.clone(),
            choice,
        });
        let excess = self.entries.len().saturating_sub(TRACK_MEMORY_LIMIT);
        self.entries.drain(..excess);
        self.save()
    }

    fn save(&self) -> Result<()> {
        let dir = &self.paths.data_dir;
        fs::create_dir_all(dir).map_err(storage)?;
        let json = serde_json::to_vec(&self.entries).map_err(storage)?;
        let tmp = dir.join("track-memory.json.tmp");
        fs::write(&tmp, json).map_err(storage)?;
        fs::rename(&tmp, dir.join(FILE_NAME)).map_err(storage)
    }
}
