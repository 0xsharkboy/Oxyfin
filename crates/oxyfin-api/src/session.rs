//! Authenticated access to a Jellyfin server.

use crate::{Result, Server, UserDto, UserId};
use serde::de::IgnoredAny;
use std::fmt;
use std::sync::Arc;

/// A signed-in user on a [`Server`].
///
/// Cheap to clone and shareable across threads. Its `Debug` output never shows the token.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

struct Inner {
    server: Server,
    token: String,
    user_id: UserId,
}

impl Session {
    pub(crate) fn new(server: Server, token: String, user_id: UserId) -> Self {
        Self {
            inner: Arc::new(Inner {
                server,
                token,
                user_id,
            }),
        }
    }

    /// Access token, to store for [`Server::restore_session`].
    pub fn token(&self) -> &str {
        &self.inner.token
    }

    /// Identifier of the signed-in user.
    pub fn user_id(&self) -> &UserId {
        &self.inner.user_id
    }

    /// Server this session belongs to.
    pub fn server(&self) -> &Server {
        &self.inner.server
    }

    /// Fetches the signed-in user (`GET Users/Me`).
    pub async fn me(&self) -> Result<UserDto> {
        self.server()
            .http()
            .get(Some(self.token()), "Users/Me", &[])
            .await
    }

    /// Ends the session on the server (`POST Sessions/Logout`).
    pub async fn logout(&self) -> Result<()> {
        let _: IgnoredAny = self
            .server()
            .http()
            .post_empty(Some(self.token()), "Sessions/Logout", &[])
            .await?;
        Ok(())
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("server", &self.inner.server)
            .field("user_id", &self.inner.user_id)
            .field("token", &"<redacted>")
            .finish()
    }
}
