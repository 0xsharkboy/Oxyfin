//! Client identity sent with every request.

use crate::CLIENT_NAME;
use std::fmt::Write;

/// Identifies this installation to the Jellyfin server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientInfo {
    /// Human-readable device name shown in the server dashboard.
    pub device_name: String,
    /// Stable unique identifier of this installation.
    pub device_id: String,
}

impl ClientInfo {
    /// Creates a client identity from a device name and a stable device id.
    pub fn new(device_name: impl Into<String>, device_id: impl Into<String>) -> Self {
        Self {
            device_name: device_name.into(),
            device_id: device_id.into(),
        }
    }
}

/// Percent-encodes every byte outside the RFC 3986 unreserved set.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Builds the `Authorization` header value, with the token once the user is signed in.
pub(crate) fn authorization_header(info: &ClientInfo, token: Option<&str>) -> String {
    let mut header = format!(
        r#"MediaBrowser Client="{}", Device="{}", DeviceId="{}", Version="{}""#,
        percent_encode(CLIENT_NAME),
        percent_encode(&info.device_name),
        percent_encode(&info.device_id),
        env!("CARGO_PKG_VERSION"),
    );
    if let Some(token) = token {
        let _ = write!(header, r#", Token="{}""#, percent_encode(token));
    }
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_without_token() {
        let info = ClientInfo::new("Living Room", "abc123");
        assert_eq!(
            authorization_header(&info, None),
            r#"MediaBrowser Client="Oxyfin", Device="Living%20Room", DeviceId="abc123", Version="0.1.0""#
        );
    }

    #[test]
    fn header_with_token_appends_token() {
        let info = ClientInfo::new("Living Room", "abc123");
        assert!(authorization_header(&info, Some("tok")).ends_with(r#", Token="tok""#));
    }

    #[test]
    fn header_escapes_quotes_and_commas() {
        let info = ClientInfo::new("Bob's \"PC\", été", "abc123");
        assert!(
            authorization_header(&info, None)
                .contains(r#"Device="Bob%27s%20%22PC%22%2C%20%C3%A9t%C3%A9""#)
        );
    }
}
