//! Error type.

use oxyfin_api::Error as ApiError;
use oxyfin_player::PlayerError;

/// Errors surfaced by the application logic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    /// The stored session is no longer accepted by the server.
    #[error("the session has expired")]
    SessionExpired,
    /// The server could not be reached.
    #[error("network error: {0}")]
    Network(String),
    /// The server is older than the minimum supported version.
    #[error("unsupported server version {found}, {minimum} or later is required")]
    UnsupportedServer {
        /// Version reported by the server.
        found: String,
        /// Oldest supported version.
        minimum: String,
    },
    /// The server address is not a valid HTTP(S) URL.
    #[error("invalid server URL: {0}")]
    InvalidServerUrl(String),
    /// The server rejected the username or password.
    #[error("invalid credentials")]
    InvalidCredentials,
    /// Quick Connect is disabled on the server.
    #[error("Quick Connect is disabled on this server")]
    QuickConnectDisabled,
    /// The Quick Connect code expired before it was approved.
    #[error("the Quick Connect code has expired")]
    QuickConnectExpired,
    /// The server answered with an unexpected HTTP status.
    #[error("the server answered with HTTP {status}")]
    Server {
        /// HTTP status code.
        status: u16,
    },
    /// A server response could not be understood.
    #[error("unexpected server response: {0}")]
    Decode(String),
    /// Local storage or the system keyring failed.
    #[error("storage error: {0}")]
    Storage(String),
    /// The media player failed.
    #[error("player error: {0}")]
    Player(String),
}

/// Result type of the application logic.
pub type Result<T> = std::result::Result<T, CoreError>;

impl From<ApiError> for CoreError {
    fn from(error: ApiError) -> Self {
        match error {
            ApiError::Status { status: 401, .. } => Self::SessionExpired,
            ApiError::Status { status, .. } => Self::Server { status },
            ApiError::Transport { source, .. } => Self::Network(source.to_string()),
            ApiError::InvalidUrl(url) => Self::InvalidServerUrl(url),
            ApiError::UnsupportedServer { found, minimum } => Self::UnsupportedServer {
                found: found.to_string(),
                minimum: minimum.to_string(),
            },
            ApiError::Decode { .. } | ApiError::InvalidVersion(_) => {
                Self::Decode(error.to_string())
            }
        }
    }
}

impl From<PlayerError> for CoreError {
    fn from(error: PlayerError) -> Self {
        Self::Player(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyfin_api::{ClientInfo, Server, ServerVersion};

    fn status(code: u16) -> ApiError {
        ApiError::Status {
            route: "Users/Me".into(),
            status: code,
            body: String::new(),
        }
    }

    #[test]
    fn api_401_becomes_session_expired() {
        assert_eq!(CoreError::from(status(401)), CoreError::SessionExpired);
    }

    #[tokio::test]
    async fn api_transport_becomes_network() {
        let server = Server::new("http://127.0.0.1:1", ClientInfo::new("test", "id")).unwrap();
        let error = server.public_info().await.unwrap_err();
        assert!(matches!(error, ApiError::Transport { .. }));
        assert!(matches!(CoreError::from(error), CoreError::Network(_)));
    }

    #[tokio::test]
    async fn network_message_never_carries_the_url_query() {
        let server = Server::new("http://127.0.0.1:1", ClientInfo::new("test", "id")).unwrap();
        let error = server
            .quick_connect_poll(&"abc-secret".to_owned().into())
            .await
            .unwrap_err();
        let CoreError::Network(message) = CoreError::from(error) else {
            panic!("expected a network error");
        };
        assert!(!message.contains("abc-secret"), "{message}");
    }

    #[test]
    fn api_unsupported_keeps_versions() {
        let error = ApiError::UnsupportedServer {
            found: "10.9.0".parse::<ServerVersion>().unwrap(),
            minimum: "10.10.0".parse::<ServerVersion>().unwrap(),
        };
        match CoreError::from(error) {
            CoreError::UnsupportedServer { found, minimum } => {
                assert_eq!(found, "10.9.0");
                assert_eq!(minimum, "10.10.0");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn api_500_becomes_server_status() {
        assert_eq!(
            CoreError::from(status(500)),
            CoreError::Server { status: 500 }
        );
    }
}
