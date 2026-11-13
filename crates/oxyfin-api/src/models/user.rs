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
}

/// Response of a successful sign-in.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct AuthenticationResult {
    pub(crate) user: UserDto,
    pub(crate) access_token: String,
    pub(crate) server_id: Option<String>,
}
