//! Unauthenticated access to a Jellyfin server.

use crate::http::Http;
use crate::models::AuthenticationResult;
use crate::{ClientInfo, Error, MINIMUM_SERVER_VERSION, PublicSystemInfo, Result, Session, UserId};
use serde_json::json;
use std::fmt;
use std::sync::Arc;
use url::Url;

/// A Jellyfin server, before signing in.
///
/// Cheap to clone: clones share one connection pool.
#[derive(Clone)]
pub struct Server {
    http: Arc<Http>,
}

impl Server {
    /// Creates a client for the server at `base_url`.
    ///
    /// The URL is trimmed, must use `http` or `https`, must not carry a user name or password,
    /// and may include a subpath such as `/jellyfin`.
    pub fn new(base_url: &str, info: ClientInfo) -> Result<Server> {
        let mut url = Url::parse(base_url.trim())
            .map_err(|err| Error::InvalidUrl(format!("cannot parse the server URL: {err}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(Error::InvalidUrl(
                "the server URL must start with http:// or https://".to_owned(),
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(Error::InvalidUrl(
                "the server URL must not contain a user name or password".to_owned(),
            ));
        }
        url.set_query(None);
        url.set_fragment(None);
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Ok(Server {
            http: Arc::new(Http::new(url, info)?),
        })
    }

    /// Base URL of the server, always ending with `/`.
    pub fn base_url(&self) -> &Url {
        self.http.base_url()
    }

    pub(crate) fn http(&self) -> &Http {
        &self.http
    }

    /// Fetches the public server information (`GET System/Info/Public`).
    pub async fn public_info(&self) -> Result<PublicSystemInfo> {
        self.http.get(None, "System/Info/Public", &[]).await
    }

    /// Fetches the public server information and checks the server is recent enough.
    ///
    /// Fails with [`Error::UnsupportedServer`] below [`MINIMUM_SERVER_VERSION`].
    pub async fn check_version(&self) -> Result<PublicSystemInfo> {
        let info = self.public_info().await?;
        let found = info.server_version()?;
        if found < MINIMUM_SERVER_VERSION {
            return Err(Error::UnsupportedServer {
                found,
                minimum: MINIMUM_SERVER_VERSION,
            });
        }
        Ok(info)
    }

    /// Signs in with a user name and password (`POST Users/AuthenticateByName`).
    pub async fn authenticate_by_name(&self, username: &str, password: &str) -> Result<Session> {
        let body = json!({ "Username": username, "Pw": password });
        let result: AuthenticationResult = self
            .http
            .post_json(None, "Users/AuthenticateByName", &[], &body)
            .await?;
        Ok(self.restore_session(result.access_token, result.user.id, result.server_id))
    }

    /// Resumes a session from a stored token and server id, without any request.
    pub fn restore_session(
        &self,
        token: impl Into<String>,
        user_id: UserId,
        server_id: Option<String>,
    ) -> Session {
        Session::new(self.clone(), token.into(), user_id, server_id)
    }
}

impl fmt::Debug for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Server")
            .field("base_url", &self.base_url().as_str())
            .finish_non_exhaustive()
    }
}
