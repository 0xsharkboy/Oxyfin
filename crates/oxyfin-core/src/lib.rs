//! Application logic for Oxyfin: session, use cases, progress sync and caching.

mod config;
mod error;
mod paths;
mod quick_connect;
mod secrets;
mod session;

pub use config::{Config, SavedSession};
pub use error::{CoreError, Result};
pub use paths::Paths;
pub use quick_connect::{QUICK_CONNECT_POLL_INTERVAL, QuickConnect};
pub use secrets::{
    FileStore, KeyringStore, MemoryStore, SecretStore, TokenStorage, TokenVault, account_key,
};
pub use session::{ServerHandle, SessionManager, SignedIn};
