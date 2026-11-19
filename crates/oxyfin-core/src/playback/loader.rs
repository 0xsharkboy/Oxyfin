//! The network phase of starting an item, run away from the controller so
//! that it keeps handling commands and player events meanwhile.

use std::time::Duration;

use oxyfin_api::{BaseItemKind, ItemId, MediaSourceInfo, PlaySessionId, Session};
use oxyfin_player::MediaSource;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;

use super::current::Siblings;
use super::queue::neighbours;
use super::segments::{SkippableSegment, skippable};
use super::types::{PlayMode, PlaybackTitle};
use crate::catalog::Catalog;
use crate::error::CoreError;
use crate::models::{Episode, ItemSummary, UserState};
use crate::trickplay::Trickplay;

/// An item ready to be loaded into the player.
pub(super) struct Prepared {
    pub(super) item: ItemId,
    pub(super) source: MediaSourceInfo,
    pub(super) play_session_id: Option<PlaySessionId>,
    pub(super) media: MediaSource,
    pub(super) start: Option<Duration>,
    pub(super) siblings: Siblings,
    /// Seek bar thumbnails of the source, when the server has some.
    pub(super) trickplay: Option<Trickplay>,
    /// Segments that arrived while the item waited to load.
    pub(super) segments: Vec<SkippableSegment>,
}

/// Why an item cannot be loaded.
pub(super) enum LoadFailure {
    Request(CoreError),
    NoDirectPlaySource,
    NothingToPlay,
}

/// A step of a load, sent back to the controller.
pub(super) enum LoadStage {
    /// The item is known: the controller announces it.
    Loading { item: ItemId, title: PlaybackTitle },
    /// The network phase is over.
    Done(Box<Result<Prepared, LoadFailure>>),
    /// The skippable segments of the prepared item, fetched after `Done` so
    /// they never delay playback. Not sent when fetching them failed.
    Segments(Vec<SkippableSegment>),
}

/// A [`LoadStage`] tagged with the load it belongs to.
pub(super) struct LoadMessage {
    pub(super) generation: u64,
    pub(super) stage: LoadStage,
}

/// Starts loads, one at a time: a new one abandons the previous one.
pub(super) struct Loader {
    generation: u64,
    task: Option<JoinHandle<()>>,
    messages: UnboundedSender<LoadMessage>,
}

impl Loader {
    /// Returns the loader and the receiver of its messages.
    pub(super) fn new() -> (Self, UnboundedReceiver<LoadMessage>) {
        let (messages, receiver) = unbounded_channel();
        let loader = Self {
            generation: 0,
            task: None,
            messages,
        };
        (loader, receiver)
    }

    /// Abandons the running load: its messages no longer count.
    pub(super) fn cancel(&mut self) {
        self.generation += 1;
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }

    /// Abandons the running load and starts fetching `item`.
    pub(super) fn start(&mut self, session: Session, item: ItemId, mode: PlayMode) {
        self.cancel();
        let generation = self.generation;
        let messages = self.messages.clone();
        self.task = Some(tokio::spawn(async move {
            let send = |stage| {
                let _ = messages.send(LoadMessage { generation, stage });
            };
            let outcome = fetch(&session, item, mode, &send).await;
            let prepared = outcome.as_ref().ok().map(|prepared| prepared.item.clone());
            send(LoadStage::Done(Box::new(outcome)));
            if let Some(item) = prepared {
                match session.media_segments(&item).await {
                    Ok(segments) => send(LoadStage::Segments(skippable(segments))),
                    Err(error) => log::warn!("cannot fetch the media segments: {error}"),
                }
            }
        }));
    }

    /// Whether `message` belongs to the latest load.
    pub(super) fn is_current(&self, message: &LoadMessage) -> bool {
        message.generation == self.generation
    }
}

async fn fetch(
    session: &Session,
    item: ItemId,
    mode: PlayMode,
    send: &impl Fn(LoadStage),
) -> Result<Prepared, LoadFailure> {
    let request = |error: oxyfin_api::Error| LoadFailure::Request(error.into());
    let mut item = item;
    let mut mode = mode;
    let mut dto = session.item(&item).await.map_err(request)?;
    if dto.r#type == BaseItemKind::Series {
        let episode = Catalog::new(session.clone())
            .next_episode_to_watch(&item)
            .await
            .map_err(LoadFailure::Request)?
            .ok_or(LoadFailure::NothingToPlay)?;
        item = episode.id;
        mode = PlayMode::Auto;
        dto = session.item(&item).await.map_err(request)?;
    }
    let title = match ItemSummary::from_dto(&dto) {
        Some(summary) => PlaybackTitle {
            title: summary.title,
            series_name: summary.series_name,
            episode_label: summary.episode_label,
        },
        None => PlaybackTitle {
            title: dto.name.clone().unwrap_or_default(),
            series_name: None,
            episode_label: None,
        },
    };
    let saved = UserState::from_dto(dto.user_data.as_ref()).position;
    send(LoadStage::Loading {
        item: item.clone(),
        title,
    });

    let episodes = async {
        match Episode::from_dto(&dto) {
            None => Ok(Siblings::default()),
            Some(episode) => {
                let catalog = Catalog::new(session.clone());
                match catalog.all_episodes(&episode.series_id).await {
                    Ok(list) => {
                        let (previous, next) = neighbours(&list, &item);
                        Ok(Siblings { previous, next })
                    }
                    Err(CoreError::SessionExpired) => Err(CoreError::SessionExpired),
                    Err(_) => Ok(Siblings::default()),
                }
            }
        }
    };
    let (siblings, info) = tokio::join!(episodes, session.playback_info(&item));
    let siblings = siblings.map_err(LoadFailure::Request)?;
    let info = info.map_err(request)?;
    let source = info
        .media_sources
        .into_iter()
        .find(|source| source.supports_direct_play)
        .ok_or(LoadFailure::NoDirectPlaySource)?;
    let trickplay = Trickplay::from_dto(&dto, &source.id);
    let stream = session.stream_source(&item, &source.id);
    let mut media =
        MediaSource::new(stream.url.as_str()).with_header("Authorization", stream.authorization);
    let start = match mode {
        PlayMode::Auto if !saved.is_zero() => Some(saved),
        _ => None,
    };
    if let Some(start) = start {
        media = media.with_start(start);
    }
    Ok(Prepared {
        item,
        source,
        play_session_id: info.play_session_id,
        media,
        start,
        siblings,
        trickplay,
        segments: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loading(generation: u64) -> LoadMessage {
        LoadMessage {
            generation,
            stage: LoadStage::Loading {
                item: ItemId::from("0000000000000000000000000000000a"),
                title: PlaybackTitle {
                    title: "Item".into(),
                    series_name: None,
                    episode_label: None,
                },
            },
        }
    }

    #[test]
    fn a_cancelled_load_is_stale() {
        let (mut loader, _receiver) = Loader::new();
        let before = loading(loader.generation);
        assert!(loader.is_current(&before));
        loader.cancel();
        assert!(!loader.is_current(&before));
        assert!(loader.is_current(&loading(loader.generation)));
    }
}
