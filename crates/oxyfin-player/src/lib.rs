//! Safe libmpv wrapper and OpenGL video rendering for Oxyfin.

mod error;
#[cfg(feature = "mpv")]
mod player;

pub use error::PlayerError;
#[cfg(feature = "mpv")]
pub use player::{Player, PlayerOptions, VideoOutput};
