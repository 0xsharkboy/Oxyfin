//! System information.

use crate::{Result, ServerVersion};
use serde::Deserialize;

/// Information a server publishes without authentication (`System/Info/Public`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublicSystemInfo {
    /// Display name of the server.
    pub server_name: Option<String>,
    /// Server version, such as `10.11.11`.
    pub version: String,
    /// Unique identifier of the server.
    pub id: String,
    /// Product name, normally `Jellyfin Server`.
    pub product_name: Option<String>,
    /// Whether the initial setup wizard has been completed.
    pub startup_wizard_completed: Option<bool>,
}

impl PublicSystemInfo {
    /// Parses [`PublicSystemInfo::version`].
    pub fn server_version(&self) -> Result<ServerVersion> {
        self.version.parse()
    }
}
