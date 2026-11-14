//! Oxyfin desktop client: Slint user interface wired to `oxyfin-core`.

#[allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]
mod generated {
    slint::include_modules!();
}

pub mod state;

pub use generated::*;
