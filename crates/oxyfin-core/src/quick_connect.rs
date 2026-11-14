//! Quick Connect: sign in with a code approved from another device.

use std::time::Duration;

use oxyfin_api::{Error as ApiError, QuickConnectSecret};

use crate::{CoreError, Result, ServerHandle, SessionManager, SignedIn};

/// Delay between two checks of a pending Quick Connect code.
pub const QUICK_CONNECT_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// A pending Quick Connect attempt.
#[derive(Debug)]
pub struct QuickConnect {
    /// Code to show the user, who approves it on a signed-in device.
    pub code: String,
    secret: QuickConnectSecret,
}

fn expired(error: ApiError) -> CoreError {
    match error {
        ApiError::Status {
            status: 401 | 404, ..
        } => CoreError::QuickConnectExpired,
        other => other.into(),
    }
}

impl SessionManager {
    /// Starts a Quick Connect attempt on `server`.
    pub async fn start_quick_connect(&self, server: &ServerHandle) -> Result<QuickConnect> {
        if !server.server.quick_connect_enabled().await? {
            return Err(CoreError::QuickConnectDisabled);
        }
        // Jellyfin answers 401 to Initiate when Quick Connect is disabled.
        let state = server
            .server
            .quick_connect_initiate()
            .await
            .map_err(|error| match error {
                ApiError::Status { status: 401, .. } => CoreError::QuickConnectDisabled,
                other => other.into(),
            })?;
        Ok(QuickConnect {
            code: state.code,
            secret: state.secret,
        })
    }

    /// Waits for the code to be approved, then signs in. Drop the future to cancel.
    pub async fn finish_quick_connect(
        &self,
        server: &ServerHandle,
        flow: QuickConnect,
    ) -> Result<SignedIn> {
        loop {
            let state = server
                .server
                .quick_connect_poll(&flow.secret)
                .await
                .map_err(expired)?;
            if state.authenticated {
                break;
            }
            tokio::time::sleep(self.quick_connect_interval).await;
        }
        let session = server
            .server
            .quick_connect_authenticate(&flow.secret)
            .await
            .map_err(expired)?;
        let user_name = session.me().await?.name;
        self.remember(&server.server, session, user_name).await
    }
}
