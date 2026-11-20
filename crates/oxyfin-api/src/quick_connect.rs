//! Quick Connect: signing in with a code approved from another signed-in device.

use crate::models::AuthenticationResult;
use crate::{Error, Result, Server, Session};
use serde::Deserialize;
use serde_json::json;
use std::fmt;

/// Secret that identifies one Quick Connect attempt.
///
/// Its `Debug` output hides the value. Pass it back to [`Server::quick_connect_poll`] and
/// [`Server::quick_connect_authenticate`].
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct QuickConnectSecret(String);

impl QuickConnectSecret {
    /// The raw secret, for persisting it between polls; never log it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl From<String> for QuickConnectSecret {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Debug for QuickConnectSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("QuickConnectSecret(..)")
    }
}

/// State of a Quick Connect attempt.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QuickConnectState {
    /// Code to show the user, who enters it on an already signed-in device.
    pub code: String,
    /// Secret identifying this attempt.
    pub secret: QuickConnectSecret,
    /// Whether the code has been approved.
    pub authenticated: bool,
}

impl Server {
    /// Tells whether Quick Connect is enabled on the server (`GET QuickConnect/Enabled`).
    pub async fn quick_connect_enabled(&self) -> Result<bool> {
        self.http().get(None, "QuickConnect/Enabled", &[]).await
    }

    /// Starts a Quick Connect attempt (`POST QuickConnect/Initiate`).
    pub async fn quick_connect_initiate(&self) -> Result<QuickConnectState> {
        self.http()
            .post_empty(None, "QuickConnect/Initiate", &[])
            .await
    }

    /// Checks whether an attempt has been approved (`GET QuickConnect/Connect`), without waiting.
    pub async fn quick_connect_poll(
        &self,
        secret: &QuickConnectSecret,
    ) -> Result<QuickConnectState> {
        self.http()
            .get(
                None,
                "QuickConnect/Connect",
                &[("secret", secret.0.clone())],
            )
            .await
    }

    /// Signs in with an approved attempt (`POST Users/AuthenticateWithQuickConnect`).
    pub async fn quick_connect_authenticate(&self, secret: &QuickConnectSecret) -> Result<Session> {
        let body = json!({ "Secret": secret.0 });
        let result: AuthenticationResult = self
            .http()
            .post_json(None, "Users/AuthenticateWithQuickConnect", &[], &body)
            .await?;
        Ok(self.restore_session(result.access_token, result.user.id, result.server_id))
    }
}

impl Session {
    /// Approves the Quick Connect `code` shown by another device, signing it in as this user
    /// (`POST QuickConnect/Authorize`).
    ///
    /// Returns `Ok(false)` when the server knows no such code, as when it is wrong or expired,
    /// and [`Error::QuickConnectDisabled`] when Quick Connect is turned off on the server.
    pub async fn quick_connect_authorize(&self, code: &str) -> Result<bool> {
        let result = self
            .server()
            .http()
            .post_empty(
                Some(self.token()),
                "QuickConnect/Authorize",
                &[("code", code.to_owned())],
            )
            .await;
        match result {
            Err(Error::Status {
                status: 400 | 404, ..
            }) => Ok(false),
            // Jellyfin answers 401 both to a rejected token and when Quick Connect is off.
            Err(error) if error.is_unauthorized() => {
                match self.server().quick_connect_enabled().await {
                    Ok(false) => Err(Error::QuickConnectDisabled),
                    _ => Err(error),
                }
            }
            other => other,
        }
    }
}
