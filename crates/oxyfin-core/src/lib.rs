//! Application logic for Oxyfin: session, use cases, progress sync and caching.

mod catalog;
mod config;
mod error;
mod images;
mod models;
mod paths;
pub mod playback;
mod quick_connect;
mod secrets;
mod session;

pub use catalog::{BrowseQuery, BrowseSource, Catalog, SortField};
pub use config::{Config, SavedSession};
pub use error::{CoreError, Result};
pub use images::{ImageCache, ImageSize};
pub use models::{
    Episode, ImageRef, ItemKind, ItemSummary, Library, LibraryKind, MovieDetails, Page, Person,
    Season, SeriesDetails, UserState,
};
pub use paths::Paths;
pub use playback::{AudioOption, SubtitleOption};
pub use quick_connect::{QUICK_CONNECT_POLL_INTERVAL, QuickConnect};
pub use secrets::{
    FileStore, KeyringStore, MemoryStore, SecretStore, TokenStorage, TokenVault, account_key,
};
pub use session::{ServerHandle, SessionManager, SignedIn};
