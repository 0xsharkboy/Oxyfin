//! Application logic for Oxyfin: session, use cases, progress sync and caching.

mod config;
mod error;
mod paths;

pub use config::{Config, SavedSession};
pub use error::{CoreError, Result};
pub use paths::Paths;
