//! Subtitle files the server adds to the playing item, such as a downloaded one.

use std::time::Duration;

use oxyfin_api::{
    ItemId, MediaSourceId, MediaSourceInfo, MediaStream, MediaStreamType, PlaybackOptions, Session,
};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use super::types::SUBTITLE_RETRIES;

/// A subtitle file asked for with `ReloadSubtitles`, until it is answered.
#[derive(Clone, Debug)]
pub(super) struct Wanted {
    pub(super) item: ItemId,
    /// Language to select among the new files.
    pub(super) language: String,
    /// The streams of the item before the download.
    pub(super) before: Vec<MediaStream>,
}

/// The source of the playing item once the server lists a new subtitle file
/// in it, or `None` when it did not in time.
pub(super) struct Refreshed {
    /// The request answered: only the latest one counts.
    pub(super) serial: u64,
    pub(super) wanted: Wanted,
    pub(super) source: Option<MediaSourceInfo>,
}

/// What to ask the server for.
pub(super) struct RefreshRequest {
    pub(super) serial: u64,
    pub(super) options: PlaybackOptions,
    /// The source the item plays.
    pub(super) source: MediaSourceId,
    pub(super) wanted: Wanted,
    pub(super) retry: Duration,
}

/// Asks the server for the streams of the playing source until it lists a
/// subtitle file it did not list before the download, then sends the source
/// back; sends `None` when the server fails or lists none after every retry.
pub(super) fn spawn(
    session: Session,
    request: RefreshRequest,
    refreshed: UnboundedSender<Refreshed>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let RefreshRequest {
            serial,
            options,
            source: source_id,
            wanted,
            retry,
        } = request;
        for attempt in 0..=SUBTITLE_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(retry).await;
            }
            let info = match session.playback_info_with(&wanted.item, &options).await {
                Ok(info) => info,
                Err(error) => {
                    log::warn!("cannot fetch the subtitles of the item: {error}");
                    break;
                }
            };
            let source = info
                .media_sources
                .into_iter()
                .find(|source| source.id == source_id);
            if let Some(source) = source
                && !added(&wanted.before, &source.media_streams).is_empty()
            {
                let _ = refreshed.send(Refreshed {
                    serial,
                    wanted,
                    source: Some(source),
                });
                return;
            }
        }
        log::warn!("no new subtitle of the item came from the server");
        let _ = refreshed.send(Refreshed {
            serial,
            wanted,
            source: None,
        });
    })
}

fn external_subtitle(stream: &MediaStream) -> bool {
    stream.r#type == MediaStreamType::Subtitle && stream.is_external
}

fn external_count(streams: &[MediaStream]) -> i32 {
    let count = streams.iter().filter(|stream| stream.is_external).count();
    i32::try_from(count).unwrap_or(i32::MAX)
}

/// Whether two external streams are the same file: by path, or by language,
/// codec and title when the server gives no path.
fn same_file(known: &MediaStream, stream: &MediaStream) -> bool {
    match (&known.path, &stream.path) {
        (Some(known), Some(path)) => known == path,
        _ => {
            known.language == stream.language
                && known.codec == stream.codec
                && known.title == stream.title
        }
    }
}

/// The external subtitle streams of `new` that `old` does not have, told apart by their file.
pub(super) fn added<'a>(old: &[MediaStream], new: &'a [MediaStream]) -> Vec<&'a MediaStream> {
    new.iter()
        .filter(|stream| {
            external_subtitle(stream)
                && !old
                    .iter()
                    .any(|known| external_subtitle(known) && same_file(known, stream))
        })
        .collect()
}

/// The index in `new` of the stream at `index` in `old`.
///
/// Jellyfin lists external streams first, so a new file moves the streams of
/// the media itself by the number of files added; files are matched as
/// [`added`] does.
pub(super) fn remapped(old: &[MediaStream], new: &[MediaStream], index: i32) -> Option<i32> {
    let stream = old.iter().find(|stream| stream.index == index)?;
    if stream.is_external {
        return new
            .iter()
            .find(|candidate| {
                candidate.is_external
                    && candidate.r#type == stream.r#type
                    && same_file(stream, candidate)
            })
            .map(|candidate| candidate.index);
    }
    let moved = index.checked_add(external_count(new) - external_count(old))?;
    new.iter()
        .find(|candidate| {
            candidate.index == moved && !candidate.is_external && candidate.r#type == stream.r#type
        })
        .map(|candidate| candidate.index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(index: i32, r#type: MediaStreamType, path: Option<&str>) -> MediaStream {
        MediaStream {
            index,
            r#type,
            is_external: path.is_some(),
            path: path.map(str::to_owned),
            ..MediaStream::default()
        }
    }

    fn before() -> Vec<MediaStream> {
        vec![
            stream(0, MediaStreamType::Subtitle, Some("/m/film.en.srt")),
            stream(1, MediaStreamType::Video, None),
            stream(2, MediaStreamType::Audio, None),
            stream(3, MediaStreamType::Subtitle, None),
        ]
    }

    fn after() -> Vec<MediaStream> {
        vec![
            stream(0, MediaStreamType::Subtitle, Some("/m/film.en.srt")),
            stream(1, MediaStreamType::Subtitle, Some("/m/film.fr.srt")),
            stream(2, MediaStreamType::Video, None),
            stream(3, MediaStreamType::Audio, None),
            stream(4, MediaStreamType::Subtitle, None),
        ]
    }

    #[test]
    fn a_new_file_moves_the_streams_of_the_media() {
        let (old, new) = (before(), after());
        assert_eq!(remapped(&old, &new, 0), Some(0));
        assert_eq!(remapped(&old, &new, 2), Some(3));
        assert_eq!(remapped(&old, &new, 3), Some(4));
        assert_eq!(remapped(&old, &new, 9), None);
    }

    #[test]
    fn only_files_not_listed_before_are_added() {
        let (old, new) = (before(), after());
        let added: Vec<i32> = added(&old, &new).iter().map(|s| s.index).collect();
        assert_eq!(added, vec![1]);
        assert!(super::added(&new, &new).is_empty());
    }

    fn unnamed(index: i32, language: &str) -> MediaStream {
        MediaStream {
            index,
            r#type: MediaStreamType::Subtitle,
            is_external: true,
            language: Some(language.to_owned()),
            codec: Some("subrip".to_owned()),
            title: Some("Downloaded".to_owned()),
            ..MediaStream::default()
        }
    }

    #[test]
    fn files_without_a_path_are_told_apart_by_language_codec_and_title() {
        let old = vec![unnamed(0, "eng"), stream(1, MediaStreamType::Video, None)];
        let new = vec![
            unnamed(0, "eng"),
            unnamed(1, "fre"),
            stream(2, MediaStreamType::Video, None),
        ];
        let added: Vec<i32> = added(&old, &new).iter().map(|s| s.index).collect();
        assert_eq!(added, vec![1]);
        assert!(super::added(&new, &new).is_empty());
        assert_eq!(remapped(&old, &new, 0), Some(0));
        assert_eq!(remapped(&old, &new, 1), Some(2));
    }
}
