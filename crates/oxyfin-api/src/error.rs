//! Error type.

use crate::ServerVersion;

/// Largest response body, in bytes, kept in [`Error::Status`].
const MAX_BODY_BYTES: usize = 1024;

/// Errors returned by the Jellyfin client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server URL could not be parsed or is not an HTTP(S) URL.
    #[error("invalid server URL: {0}")]
    InvalidUrl(String),
    /// The request could not be sent or the response could not be read.
    #[error("request to {route} failed: {source}")]
    Transport {
        /// Route of the failed request, relative to the server root.
        route: String,
        /// Underlying HTTP client error.
        source: reqwest::Error,
    },
    /// The server answered with a non-success HTTP status.
    #[error("{route} answered with HTTP {status}")]
    Status {
        /// Route of the failed request, relative to the server root.
        route: String,
        /// HTTP status code.
        status: u16,
        /// Start of the response body, truncated to 1024 bytes.
        body: String,
    },
    /// The response body was not the expected JSON.
    #[error("could not decode the response of {route}: {source}")]
    Decode {
        /// Route of the request, relative to the server root.
        route: String,
        /// Underlying JSON error.
        source: serde_json::Error,
    },
    /// A server version string could not be parsed.
    #[error("invalid server version: {0:?}")]
    InvalidVersion(String),
    /// The server is older than the minimum supported version.
    #[error("unsupported server version {found}, {minimum} or later is required")]
    UnsupportedServer {
        /// Version reported by the server.
        found: ServerVersion,
        /// Oldest supported version.
        minimum: ServerVersion,
    },
}

impl Error {
    /// Returns true when the server rejected the credentials (HTTP 401).
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Status { status: 401, .. })
    }

    /// Builds an [`Error::Status`], truncating `body` to 1024 bytes on a char boundary.
    pub(crate) fn status(route: &str, status: u16, body: &str) -> Self {
        let mut end = body.len().min(MAX_BODY_BYTES);
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        Self::Status {
            route: route.to_owned(),
            status,
            body: body[..end].to_owned(),
        }
    }
}

/// Result type of the Jellyfin client.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    fn status(code: u16) -> Error {
        Error::Status {
            route: "Users/Me".into(),
            status: code,
            body: String::new(),
        }
    }

    #[test]
    fn is_unauthorized_only_for_401() {
        assert!(status(401).is_unauthorized());
        assert!(!status(403).is_unauthorized());
        assert!(!status(500).is_unauthorized());
    }

    #[test]
    fn status_body_is_truncated_on_a_char_boundary() {
        let body = "é".repeat(2000);
        match Error::status("Users/Me", 500, &body) {
            Error::Status { body, .. } => {
                assert_eq!(body.len(), 1024);
                assert!(body.chars().all(|c| c == 'é'));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
