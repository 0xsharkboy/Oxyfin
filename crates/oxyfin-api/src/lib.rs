//! Typed HTTP client for the Jellyfin API.

mod client_info;
mod error;
mod ids;
// Unused until the HTTP layer lands.
#[allow(dead_code)]
mod json;
mod ticks;
mod version;

pub use client_info::ClientInfo;
pub use error::{Error, Result};
pub use ids::{ItemId, MediaSourceId, PlaySessionId, UserId};
pub use ticks::{TICKS_PER_SECOND, Ticks};
pub use version::{MINIMUM_SERVER_VERSION, ServerVersion};

/// Client name sent in the `Client=` field of the Jellyfin `Authorization` header.
pub const CLIENT_NAME: &str = "Oxyfin";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_name_matches_jellyfin_authorization_header() {
        assert_eq!(CLIENT_NAME, "Oxyfin");
    }
}
