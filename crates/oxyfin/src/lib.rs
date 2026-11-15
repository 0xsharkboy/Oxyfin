//! Oxyfin desktop client: Slint user interface wired to `oxyfin-core`.

#[allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]
mod generated {
    slint::include_modules!();
}

pub mod app;
pub mod generation;
pub mod images;
pub mod locale;
pub mod present;
pub mod screens;
pub mod state;
#[cfg(feature = "mpv")]
pub mod video;

pub use app::App;
pub use generated::*;
