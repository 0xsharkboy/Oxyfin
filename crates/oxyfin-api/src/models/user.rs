//! Users and authentication.

use crate::UserId;
use serde::Deserialize;

/// A Jellyfin user.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserDto {
    /// Identifier of the user.
    pub id: UserId,
    /// User name.
    pub name: String,
    /// Identifier of the server the user belongs to.
    pub server_id: Option<String>,
    /// Tag of the user's profile image, if any.
    pub primary_image_tag: Option<String>,
    /// What the user may do; all denied when the server sends none.
    #[serde(default)]
    pub policy: UserPolicy,
}

/// What a user may do on the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct UserPolicy {
    /// Whether the user may search and download subtitles.
    pub enable_subtitle_management: bool,
    /// Whether the user administers the server.
    pub is_administrator: bool,
}

impl UserPolicy {
    /// Whether the user may search and download subtitles: administrators always may.
    pub fn can_manage_subtitles(&self) -> bool {
        self.enable_subtitle_management || self.is_administrator
    }
}

/// Response of a successful sign-in.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct AuthenticationResult {
    pub(crate) user: UserDto,
    pub(crate) access_token: String,
    pub(crate) server_id: Option<String>,
}
