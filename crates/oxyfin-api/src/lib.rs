//! Typed HTTP client for the Jellyfin API.

mod client_info;
mod error;
mod http;
mod ids;
mod images;
mod items;
mod json;
mod models;
mod playback;
mod quick_connect;
mod server;
mod session;
mod ticks;
mod user_data;
mod version;

pub use client_info::ClientInfo;
pub use error::{Error, Result};
pub use ids::{ItemId, MediaSourceId, PlaySessionId, UserId};
pub use images::ImageRequest;
pub use models::{
    BaseItemDto, BaseItemKind, BaseItemPerson, ImageType, ItemFields, ItemSortBy, ItemsQuery,
    MediaSourceInfo, MediaStream, MediaStreamType, PlaybackInfoResponse, PublicSystemInfo,
    QueryResult, SortOrder, UserDto, UserItemDataDto,
};
pub use playback::{PlaybackReport, StreamSource};
pub use quick_connect::{QuickConnectSecret, QuickConnectState};
pub use server::Server;
pub use session::Session;
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
