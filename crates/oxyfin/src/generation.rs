//! Resources owned by one signed-in session.

use std::sync::Arc;

use oxyfin_api::Session;
use oxyfin_core::{
    CacheBudgets, Catalog, ImageCache, Paths, PlaybackEvent, PlaybackHandle, SessionManager,
};
#[cfg(feature = "mpv")]
use oxyfin_player::Player;

use crate::state::{GenerationId, Screen, SessionEnd};

/// The playback part of a generation, created at its first playback.
pub struct Playback {
    /// Sends commands to the playback controller.
    pub handle: PlaybackHandle,
    /// The player the controller drives and the video renderer draws.
    #[cfg(feature = "mpv")]
    pub player: Arc<Player>,
}

/// What lives as long as one signed-in session: the session and the services bound to it.
pub struct Generation {
    /// Identifier given by the state machine.
    pub id: GenerationId,
    /// The authenticated session.
    pub session: Session,
    /// Library browsing for this session.
    pub catalog: Catalog,
    /// Image cache for this session.
    pub images: Arc<ImageCache>,
    /// Playback, once something was played in this session.
    pub playback: Option<Playback>,
}

impl Generation {
    /// Creates the services of a session. Must run inside the Tokio runtime,
    /// where the image cache starts its background trim.
    pub fn new(id: GenerationId, session: Session, paths: &Paths) -> Self {
        Self {
            id,
            catalog: Catalog::new(session.clone()),
            images: Arc::new(ImageCache::new(
                session.clone(),
                paths,
                CacheBudgets::default(),
            )),
            session,
            playback: None,
        }
    }

    /// Shuts the generation down for the given reason, flushing the last
    /// playback report first.
    pub async fn end(mut self, end: SessionEnd, manager: &SessionManager) {
        if let Some(playback) = self.playback.take() {
            playback.handle.shutdown().await;
        }
        let result = match end {
            SessionEnd::Expire => manager.expire(&self.session).await,
            SessionEnd::SignOut => manager.sign_out(&self.session).await,
            SessionEnd::Close => Ok(()),
        };
        if let Err(error) = result {
            log::warn!("ending session generation {:?} failed: {error}", self.id);
        }
    }
}

/// What to do with an event of the playback controller of generation `from`.
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    /// The event belongs to a generation that is no longer current.
    Ignore,
    /// The server rejected the session of the current generation.
    Expire(GenerationId),
    /// The player screen shows the event.
    Show(PlaybackEvent),
}

/// Routes `event` from generation `from` while `current` is live.
pub fn route(current: Option<GenerationId>, from: GenerationId, event: PlaybackEvent) -> Route {
    if current != Some(from) {
        return Route::Ignore;
    }
    match event {
        PlaybackEvent::SessionExpired => Route::Expire(from),
        event => Route::Show(event),
    }
}

/// What to do with a player built off the Slint thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Built {
    /// Its generation ended while it was built.
    Release,
    /// It becomes the player of its generation, without playing: the user left the home screen.
    Keep,
    /// It becomes the player of its generation and plays the item clicked.
    Play,
}

/// Decides the fate of a player built for generation `built_for` while
/// `current` is live and `screen` is shown.
pub fn built(built_for: GenerationId, current: Option<GenerationId>, screen: Screen) -> Built {
    if current != Some(built_for) {
        Built::Release
    } else if screen == Screen::Home {
        Built::Play
    } else {
        Built::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G1: GenerationId = GenerationId(1);
    const G2: GenerationId = GenerationId(2);

    #[test]
    fn events_of_another_generation_are_ignored() {
        assert_eq!(route(Some(G2), G1, PlaybackEvent::Ended), Route::Ignore);
        assert_eq!(
            route(None, G1, PlaybackEvent::SessionExpired),
            Route::Ignore
        );
    }

    #[test]
    fn session_expiry_expires_the_generation() {
        assert_eq!(
            route(Some(G1), G1, PlaybackEvent::SessionExpired),
            Route::Expire(G1)
        );
    }

    #[test]
    fn a_player_built_for_another_generation_is_released() {
        assert_eq!(built(G1, Some(G2), Screen::Home), Built::Release);
        assert_eq!(built(G1, None, Screen::SignIn), Built::Release);
    }

    #[test]
    fn a_player_built_for_the_current_generation_plays_from_home_only() {
        assert_eq!(built(G1, Some(G1), Screen::Home), Built::Play);
        assert_eq!(built(G1, Some(G1), Screen::Settings), Built::Keep);
        assert_eq!(built(G1, Some(G1), Screen::Player), Built::Keep);
    }

    #[test]
    fn other_events_go_to_the_player_screen() {
        assert_eq!(
            route(Some(G1), G1, PlaybackEvent::Paused(true)),
            Route::Show(PlaybackEvent::Paused(true))
        );
    }
}
