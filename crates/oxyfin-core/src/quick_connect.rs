//! Quick Connect: sign in with a code approved from another device.

use std::time::Duration;

use oxyfin_api::{Error as ApiError, QuickConnectSecret};

use crate::{CoreError, Result, ServerHandle, SessionManager, SignedIn};

/// Delay between two checks of a pending Quick Connect code.
pub const QUICK_CONNECT_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// The Quick Connect code typed by the user: its digits only, without the spaces or dashes
/// that group them.
pub fn quick_connect_code(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

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
        self.remember(server, session, user_name).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_keeps_only_its_digits() {
        assert_eq!(quick_connect_code("482 913"), "482913");
        assert_eq!(quick_connect_code(" 48\t2913\n"), "482913");
        assert_eq!(quick_connect_code("   "), "");
        assert_eq!(quick_connect_code("482-913"), "482913");
        assert_eq!(quick_connect_code(" - -"), "");
    }
}
