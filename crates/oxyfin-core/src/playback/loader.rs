//! The network phase of starting an item, run away from the controller so
//! that it keeps handling commands and player events meanwhile.

use std::time::Duration;

use oxyfin_api::{ItemId, MediaSourceInfo, PlaySessionId, Session};
use oxyfin_player::MediaSource;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;

use super::current::Siblings;
use super::queue::neighbours;
use super::types::{PlayMode, PlaybackTitle};
use crate::catalog::Catalog;
use crate::error::CoreError;
use crate::models::{Episode, ItemSummary, UserState};

/// An item ready to be loaded into the player.
pub(super) struct Prepared {
    pub(super) item: ItemId,
    pub(super) source: MediaSourceInfo,
    pub(super) play_session_id: Option<PlaySessionId>,
    pub(super) media: MediaSource,
    pub(super) start: Option<Duration>,
    pub(super) siblings: Siblings,
}

/// Why an item cannot be loaded.
pub(super) enum LoadFailure {
    Request(CoreError),
    NoDirectPlaySource,
}

/// A step of a load, sent back to the controller.
pub(super) enum LoadStage {
    /// The item is known: the controller announces it.
    Loading { item: ItemId, title: PlaybackTitle },
    /// The network phase is over.
    Done(Box<Result<Prepared, LoadFailure>>),
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
            send(LoadStage::Done(Box::new(outcome)));
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
    let dto = session.item(&item).await.map_err(request)?;
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
    })
}
