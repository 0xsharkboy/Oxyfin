//! Oxyfin desktop client: Slint user interface wired to `oxyfin-core`.

#[allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]
mod generated {
    slint::include_modules!();
}

pub mod app;
mod back_cache;
pub mod discord;
pub mod generation;
mod hover;
pub mod images;
#[cfg(target_os = "linux")]
pub mod inhibit;
pub mod launch;
pub mod locale;
mod menu;
#[cfg(target_os = "linux")]
pub mod mpris;
pub mod present;
pub mod screens;
pub mod state;
#[cfg(feature = "mpv")]
pub mod video;

/// Application ID: the Wayland app ID and X11 class of the window, and the name of the
/// desktop entry, icon and AppStream metainfo under `data/`.
pub const APP_ID: &str = "io.github._0xsharkboy.Oxyfin";

pub use app::App;
pub use generated::*;
