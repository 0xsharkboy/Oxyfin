//! Safe libmpv wrapper and OpenGL video rendering for Oxyfin.

mod error;
#[cfg(feature = "mpv")]
mod events;
#[cfg(feature = "mpv")]
mod player;
#[cfg(feature = "mpv")]
mod tracks;
mod types;

pub use error::PlayerError;
#[cfg(feature = "mpv")]
pub use player::{Player, PlayerOptions, VideoOutput};
pub use types::{MediaSource, PlayerEvent, SeekTarget, Track, TrackId, TrackKind, UnloadReason};
