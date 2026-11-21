//! Subtitle files the server adds to the playing item, such as a downloaded one.

use std::time::Duration;

use oxyfin_api::{ItemId, MediaSourceInfo, MediaStream, MediaStreamType, PlaybackOptions, Session};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use super::types::SUBTITLE_RETRIES;

/// The source of the playing item once the server lists a new subtitle file
/// in it, or `None` when it did not in time.
pub(super) struct Refreshed {
    pub(super) item: ItemId,
    pub(super) source: Option<MediaSourceInfo>,
    /// Language to select among the new files.
    pub(super) language: String,
}

/// What to ask the server for.
pub(super) struct RefreshRequest {
    pub(super) item: ItemId,
    pub(super) options: PlaybackOptions,
    /// The source the item plays, with its streams as they were loaded.
    pub(super) source: MediaSourceInfo,
    pub(super) language: String,
    pub(super) retry: Duration,
}

/// Asks the server for the streams of the playing source until it lists a
/// subtitle file it did not list before, then sends the source back; sends
/// `None` when the server fails or lists none after every retry.
pub(super) fn spawn(
    session: Session,
    request: RefreshRequest,
    refreshed: UnboundedSender<Refreshed>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        for attempt in 0..=SUBTITLE_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(request.retry).await;
            }
            let info = match session
                .playback_info_with(&request.item, &request.options)
                .await
            {
                Ok(info) => info,
                Err(error) => {
                    log::warn!("cannot fetch the subtitles of the item: {error}");
                    break;
                }
            };
            let source = info
                .media_sources
                .into_iter()
                .find(|source| source.id == request.source.id);
            if let Some(source) = source
                && !added(&request.source.media_streams, &source.media_streams).is_empty()
            {
                let _ = refreshed.send(Refreshed {
                    item: request.item,
                    source: Some(source),
                    language: request.language,
                });
                return;
            }
        }
        log::warn!("no new subtitle of the item came from the server");
        let _ = refreshed.send(Refreshed {
            item: request.item,
            source: None,
            language: request.language,
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

/// The external subtitle streams of `new` that `old` does not have, told apart by their file.
pub(super) fn added<'a>(old: &[MediaStream], new: &'a [MediaStream]) -> Vec<&'a MediaStream> {
    new.iter()
        .filter(|stream| {
            external_subtitle(stream)
                && !old.iter().any(|known| {
                    external_subtitle(known) && known.path.is_some() && known.path == stream.path
                })
        })
        .collect()
}

/// The index in `new` of the stream at `index` in `old`.
///
/// Jellyfin lists external streams first, so a new file moves the streams of
/// the media itself by the number of files added; files are matched by path.
pub(super) fn remapped(old: &[MediaStream], new: &[MediaStream], index: i32) -> Option<i32> {
    let stream = old.iter().find(|stream| stream.index == index)?;
    if stream.is_external {
        return new
            .iter()
            .find(|candidate| {
                candidate.is_external
                    && candidate.r#type == stream.r#type
                    && candidate.path.is_some()
                    && candidate.path == stream.path
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
}
