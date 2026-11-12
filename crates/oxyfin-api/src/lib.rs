//! Typed HTTP client for the Jellyfin API.

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
