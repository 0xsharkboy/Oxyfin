//! Sign-in, session restoration and sign-out.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use oxyfin_api::{ClientInfo, Server, Session, UserId};

use crate::{
    Config, CoreError, Paths, QUICK_CONNECT_POLL_INTERVAL, Result, SavedSession, TokenStorage,
    TokenVault, account_key,
};

/// A server that passed the version check.
#[derive(Debug, Clone)]
pub struct ServerHandle {
    /// Client for the server.
    pub server: Server,
    /// Display name published by the server.
    pub name: Option<String>,
    /// Version published by the server.
    pub version: String,
}

/// A signed-in user. `Debug` never shows the token.
#[derive(Debug, Clone)]
pub struct SignedIn {
    /// The authenticated session.
    pub session: Session,
    /// Display name of the user.
    pub user_name: String,
    /// Base URL of the server.
    pub server_url: String,
    /// Where the token is kept.
    pub token_storage: TokenStorage,
    /// True when the token sits in a plain file and the user was not told yet.
    pub show_file_token_warning: bool,
}

/// Owns the persisted session and drives its lifecycle.
#[derive(Debug)]
pub struct SessionManager {
    paths: Paths,
    vault: Arc<TokenVault>,
    config: Mutex<Config>,
    device_name: String,
    pub(crate) quick_connect_interval: Duration,
}

fn blocking<E: std::fmt::Display>(error: E) -> CoreError {
    CoreError::Storage(error.to_string())
}

impl SessionManager {
    /// Loads the configuration and prepares the manager.
    pub fn new(paths: Paths, vault: TokenVault, device_name: impl Into<String>) -> Result<Self> {
        let config = Config::load_or_create(&paths)?;
        Ok(Self {
            paths,
            vault: Arc::new(vault),
            config: Mutex::new(config),
            device_name: device_name.into(),
            quick_connect_interval: QUICK_CONNECT_POLL_INTERVAL,
        })
    }

    /// Overrides the delay between Quick Connect checks, mainly for tests.
    pub fn with_quick_connect_interval(mut self, interval: Duration) -> Self {
        self.quick_connect_interval = interval;
        self
    }

    fn config(&self) -> MutexGuard<'_, Config> {
        self.config.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn update(&self, change: impl FnOnce(&mut Config)) -> Result<()> {
        let mut config = self.config();
        change(&mut config);
        config.save(&self.paths)
    }

    fn client_info(&self) -> ClientInfo {
        ClientInfo::new(self.device_name.clone(), self.config().device_id.clone())
    }

    async fn vault_call<T: Send + 'static>(
        &self,
        call: impl FnOnce(&TokenVault) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let vault = Arc::clone(&self.vault);
        tokio::task::spawn_blocking(move || call(&vault))
            .await
            .map_err(blocking)?
    }

    /// Returns the last server address a sign-in succeeded on.
    pub fn last_server_url(&self) -> Option<String> {
        self.config().last_server_url.clone()
    }

    /// Reaches the server at `url` and checks that its version is supported.
    pub async fn connect(&self, url: &str) -> Result<ServerHandle> {
        let server = Server::new(url, self.client_info())?;
        let info = server.check_version().await?;
        Ok(ServerHandle {
            server,
            name: info.server_name,
            version: info.version,
        })
    }

    /// Signs in with a user name and password and remembers the session.
    pub async fn sign_in(
        &self,
        server: &ServerHandle,
        user: &str,
        password: &str,
    ) -> Result<SignedIn> {
        let session = server
            .server
            .authenticate_by_name(user, password)
            .await
            .map_err(|error| {
                if error.is_unauthorized() {
                    CoreError::InvalidCredentials
                } else {
                    error.into()
                }
            })?;
        let user_name = session.me().await?.name;
        self.remember(&server.server, session, user_name).await
    }

    pub(crate) async fn remember(
        &self,
        server: &Server,
        session: Session,
        user_name: String,
    ) -> Result<SignedIn> {
        let server_url = server.base_url().to_string();
        let account = account_key(&server_url, session.user_id().as_str());
        let token = session.token().to_owned();
        let previous = self.config().session.clone();

        let saved_account = account.clone();
        let token_storage = self
            .vault_call(move |vault| vault.save(&saved_account, &token))
            .await?;

        if let Some(old) = previous {
            let old_account = account_key(&old.server_url, &old.user_id);
            if old_account != account {
                let _ = self
                    .vault_call(move |vault| vault.clear(&old_account))
                    .await;
            }
        }

        let saved = SavedSession {
            server_url: server_url.clone(),
            user_id: session.user_id().to_string(),
            user_name: user_name.clone(),
            server_id: session.server_id().map(str::to_owned),
        };
        let url = server_url.clone();
        let result = self.update(move |config| {
            config.session = Some(saved);
            config.last_server_url = Some(url);
        });
        if let Err(error) = result {
            let _ = self.vault_call(move |vault| vault.clear(&account)).await;
            return Err(error);
        }
        let show_file_token_warning =
            token_storage == TokenStorage::File && !self.config().file_token_warning_shown;
        Ok(SignedIn {
            session,
            user_name,
            server_url,
            token_storage,
            show_file_token_warning,
        })
    }

    /// Rebuilds the remembered session without any network request.
    pub async fn restore(&self) -> Result<Option<SignedIn>> {
        let Some(saved) = self.config().session.clone() else {
            return Ok(None);
        };
        let server = Server::new(&saved.server_url, self.client_info())?;
        let account = account_key(&saved.server_url, &saved.user_id);
        let Some((token, token_storage)) =
            self.vault_call(move |vault| vault.load(&account)).await?
        else {
            return Ok(None);
        };
        let session = server.restore_session(token, UserId::from(saved.user_id), saved.server_id);
        let show_file_token_warning =
            token_storage == TokenStorage::File && !self.config().file_token_warning_shown;
        Ok(Some(SignedIn {
            session,
            user_name: saved.user_name,
            server_url: saved.server_url,
            token_storage,
            show_file_token_warning,
        }))
    }

    /// Checks the session with the server, expiring it when the server rejects the token.
    pub async fn validate(&self, session: &Session) -> Result<()> {
        match session.me().await {
            Ok(_) => Ok(()),
            Err(error) if error.is_unauthorized() => {
                self.expire().await?;
                Err(CoreError::SessionExpired)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Forgets the token and the remembered session, keeping the last server address.
    pub async fn expire(&self) -> Result<()> {
        let saved = self.config().session.clone();
        if let Some(saved) = saved {
            let account = account_key(&saved.server_url, &saved.user_id);
            self.vault_call(move |vault| vault.clear(&account)).await?;
        }
        self.update(|config| config.session = None)
    }

    /// Ends the session on the server, then forgets it locally even if the server call fails.
    pub async fn sign_out(&self, session: &Session) -> Result<()> {
        let _ = session.logout().await;
        self.expire().await
    }

    /// Records that the user has seen the plain-file token warning.
    pub fn acknowledge_file_token_warning(&self) -> Result<()> {
        self.update(|config| config.file_token_warning_shown = true)
    }
}
