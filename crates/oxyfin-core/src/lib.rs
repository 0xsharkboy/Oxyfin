//! Application logic for Oxyfin: session, use cases, progress sync and caching.
//!
//! # Wiring playback
//!
//! The binary creates the player with a callback that forwards its events to
//! a [`PlayerEventSink`], keeps one `Arc<Player>` to create the video renderer
//! and gives another to the [`PlaybackController`]. The UI then sends
//! [`PlaybackCommand`]s and renders the [`PlaybackEvent`]s; it never talks to
//! mpv or the server directly. [`PlaybackHandle::shutdown`] flushes the last
//! report when the window closes.
//!
//! The controller must run on a multi-thread tokio runtime: adding an external
//! subtitle makes mpv download it on the controller's thread.
//!
//! # Session lifecycle
//!
//! The player, its event sink and the controller live for one signed-in
//! session: a controller is bound to one [`oxyfin_api::Session`] and consumes
//! the player's events. After a sign-out or an expiry, the binary shuts the
//! controller down and creates a new `Player`, sink and controller, and
//! therefore a new `VideoRenderer`, for the next session.
//!
//! On [`CoreError::SessionExpired`] or [`PlaybackEvent::SessionExpired`], the
//! binary must call [`SessionManager::expire`] to forget the token, then
//! rebuild the [`Catalog`], the [`ImageCache`] and the playback controller for
//! the next session.
//!
//! ```no_run
//! # #[cfg(feature = "mpv")]
//! # async fn wire(session: oxyfin_api::Session, item: oxyfin_api::ItemId, paths: oxyfin_core::Paths) -> Result<(), oxyfin_player::PlayerError> {
//! use std::sync::Arc;
//!
//! use oxyfin_core::{
//!     PlayMode, PlaybackCommand, PlaybackController, PlaybackEvent, TrackMemory, account_key,
//!     player_events,
//! };
//! use oxyfin_player::{Player, PlayerOptions};
//!
//! let (sink, events) = player_events();
//! let player = Arc::new(Player::new(PlayerOptions::default(), move |event| sink.send(event))?);
//! // Keep `player` to create the `VideoRenderer` in the UI's rendering notifier.
//! let account = account_key(session.server().base_url().as_str(), session.user_id().as_str());
//! let memory = TrackMemory::load(&paths, &account);
//! let (handle, mut updates) = PlaybackController::spawn(session, player.clone(), events, memory);
//!
//! handle.send(PlaybackCommand::Play { item, mode: PlayMode::Auto });
//! while let Some(update) = updates.recv().await {
//!     if let PlaybackEvent::Stopped = update {
//!         break;
//!     }
//! }
//! handle.shutdown().await;
//! # Ok(())
//! # }
//! ```

mod catalog;
mod config;
mod error;
mod images;
mod models;
mod paths;
mod playback;
mod presence;
mod quick_connect;
mod secrets;
mod session;
mod subtitles;
mod trickplay;

pub use catalog::{BrowseQuery, BrowseSource, Catalog, ContentKind, SortField, WatchFilter};
pub use config::{Config, SavedSession};
pub use error::{CoreError, Result};
pub use images::{
    CacheBudgets, DEFAULT_DISK_BUDGET, DEFAULT_MEMORY_BUDGET, IMAGE_FETCHES, ImageCache, ImageSize,
};
pub use models::{
    CollectionDetails, CollectionLink, Episode, ImageRef, ItemKind, ItemSummary, Library,
    LibraryKind, MovieDetails, Page, Person, PersonDetails, PersonKind, PersonTitle, Season,
    SeriesDetails, UserState,
};
pub use oxyfin_api::DiscoveredServer;
pub use paths::Paths;
pub use playback::{
    AudioOption, AudioPick, CHAPTER_RESTART_GRACE, COUNTDOWNS, Chapter, HdrFormat, HdrInfo,
    MediaPlayer, PROGRESS_INTERVAL, PlayMethod, PlayMode, PlaybackCommand, PlaybackController,
    PlaybackError, PlaybackEvent, PlaybackHandle, PlaybackPrefs, PlaybackStats, PlaybackTimings,
    PlaybackTitle, PlayerEventSink, PlayerEvents, QUALITIES, Quality, SEEK_STEP, SHUTDOWN_TIMEOUT,
    SPEEDS, SegmentKind, SkippableSegment, StatsReport, SubtitleOption, SubtitlePick,
    TRACK_MEMORY_LIMIT, TrackChoice, TrackMemory, UNLOAD_GRACE, best_audio, best_subtitle,
    chapter_at, next_chapter, player_events, previous_chapter, start_at,
};
pub use presence::{PRESENCE_ASSET, PRESENCE_TEXT_LIMIT, Presence, TMDB_IMAGES, presence_for};
pub use quick_connect::{QUICK_CONNECT_POLL_INTERVAL, QuickConnect, quick_connect_code};
pub use secrets::{
    FileStore, KeyringStore, MemoryStore, SecretStore, TokenStorage, TokenVault, account_key,
};
pub use session::{
    DISCOVERY_TIMEOUT, SERVER_VERSION_TIMEOUT, ServerHandle, SessionManager, SignedIn,
};
pub use subtitles::{
    SubtitleBackground, SubtitleColor, SubtitlePosition, SubtitleSize, SubtitleStyle,
};
pub use trickplay::{TRICKPLAY_WIDTH, Trickplay, TrickplaySheet, TrickplayTile};
