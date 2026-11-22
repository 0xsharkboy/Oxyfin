//! Sign-in, session restoration and sign-out.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use oxyfin_api::{ClientInfo, DiscoveredServer, Server, Session, UserId};

use crate::{
    Config, CoreError, Paths, PlaybackPrefs, QUICK_CONNECT_POLL_INTERVAL, Quality, Result,
    SavedSession, SubtitleStyle, TokenStorage, TokenVault, account_key,
};

/// How long a discovery waits for servers to answer.
pub const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a discovered server has to tell its version.
pub const SERVER_VERSION_TIMEOUT: Duration = Duration::from_secs(1);

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
    /// Display name of the server, when it published one.
    pub server_name: Option<String>,
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
    discovery_targets: Mutex<Option<Vec<SocketAddr>>>,
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
            discovery_targets: Mutex::new(None),
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
        let mut next = config.clone();
        change(&mut next);
        next.save(&self.paths)?;
        *config = next;
        Ok(())
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

    /// Sends discovery requests to these addresses instead of the local broadcast, for tests.
    pub fn set_discovery_targets(&self, targets: Vec<SocketAddr>) {
        *self
            .discovery_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(targets);
    }

    /// Lists the Jellyfin servers that answer on the local network within [`DISCOVERY_TIMEOUT`].
    ///
    /// Finds nothing, rather than failing, when the network refuses the request.
    pub async fn discover(&self) -> Vec<DiscoveredServer> {
        let targets = self
            .discovery_targets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(oxyfin_api::default_targets);
        oxyfin_api::discover(DISCOVERY_TIMEOUT, &targets).await
    }

    /// Version a discovered server publishes, or `None` when it does not tell within
    /// [`SERVER_VERSION_TIMEOUT`]. The request carries no client identity and follows no
    /// redirect.
    pub async fn discovered_version(&self, server: &DiscoveredServer) -> Option<String> {
        oxyfin_api::discovered_version(server, SERVER_VERSION_TIMEOUT).await
    }

    /// Returns how subtitles look, as last saved.
    pub fn subtitle_style(&self) -> SubtitleStyle {
        self.config().subtitles
    }

    /// Saves how subtitles look. The saved style is unchanged on failure.
    pub fn set_subtitle_style(&self, style: SubtitleStyle) -> Result<()> {
        self.update(|config| config.subtitles = style)
    }

    /// Returns the highest streaming quality, as last saved.
    pub fn quality(&self) -> Quality {
        self.config().quality
    }

    /// Saves the highest streaming quality. The saved quality is unchanged on failure.
    pub fn set_quality(&self, quality: Quality) -> Result<()> {
        self.update(|config| config.quality = quality)
    }

    /// Returns whether the window opens fullscreen, as last saved.
    pub fn start_fullscreen(&self) -> bool {
        self.config().start_fullscreen
    }

    /// Saves whether the window opens fullscreen. The saved choice is unchanged on failure.
    pub fn set_start_fullscreen(&self, on: bool) -> Result<()> {
        self.update(|config| config.start_fullscreen = on)
    }

    /// Returns whether Discord shows what plays, as last saved.
    pub fn discord(&self) -> bool {
        self.config().discord
    }

    /// Saves whether Discord shows what plays. The saved choice is unchanged on failure.
    pub fn set_discord(&self, on: bool) -> Result<()> {
        self.update(|config| config.discord = on)
    }

    /// Returns whether the player shows the time left, as last saved.
    pub fn time_remaining(&self) -> bool {
        self.config().time_remaining
    }

    /// Saves whether the player shows the time left. The saved choice is unchanged on failure.
    pub fn set_time_remaining(&self, on: bool) -> Result<()> {
        self.update(|config| config.time_remaining = on)
    }

    /// Returns the playback choices, as last saved.
    pub fn playback_prefs(&self) -> PlaybackPrefs {
        self.config().playback
    }

    /// Saves the playback choices. The saved choices are unchanged on failure.
    pub fn set_playback_prefs(&self, prefs: PlaybackPrefs) -> Result<()> {
        self.update(|config| config.playback = prefs)
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
        self.remember(server, session, user_name).await
    }

    pub(crate) async fn remember(
        &self,
        server: &ServerHandle,
        session: Session,
        user_name: String,
    ) -> Result<SignedIn> {
        let server_url = server.server.base_url().to_string();
        let account = account_key(&server_url, session.user_id().as_str());
        let token = session.token().to_owned();
        let previous = self.config().session.clone();

        let saved_account = account.clone();
        let token_storage = self
            .vault_call(move |vault| vault.save(&saved_account, &token))
            .await?;

        let saved = SavedSession {
            server_url: server_url.clone(),
            user_id: session.user_id().to_string(),
            user_name: user_name.clone(),
            server_id: session.server_id().map(str::to_owned),
            server_name: server.name.clone(),
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
        if let Some(old) = previous {
            let old_account = account_key(&old.server_url, &old.user_id);
            if old_account != account {
                let _ = self
                    .vault_call(move |vault| vault.clear(&old_account))
                    .await;
            }
        }
        let show_file_token_warning =
            token_storage == TokenStorage::File && !self.config().file_token_warning_shown;
        Ok(SignedIn {
            session,
            user_name,
            server_url,
            server_name: server.name.clone(),
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
            server_name: saved.server_name,
            token_storage,
            show_file_token_warning,
        }))
    }

    /// Checks the session with the server, expiring it when the server rejects the token.
    pub async fn validate(&self, session: &Session) -> Result<()> {
        match session.me().await {
            Ok(_) => Ok(()),
            Err(error) if error.is_unauthorized() => {
                self.expire(session).await?;
                Err(CoreError::SessionExpired)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Forgets `session` after the server rejected its token, keeping the last server address.
    ///
    /// Like `sign_out`, it leaves alone a later sign-in on the same account.
    pub async fn expire(&self, session: &Session) -> Result<()> {
        self.forget(session).await
    }

    /// Forgets `session` locally, then ends it on the server, ignoring any server error.
    ///
    /// Only the account of `session` is forgotten, and only while it still holds
    /// the token of `session`, so a later sign-in survives a late sign-out.
    pub async fn sign_out(&self, session: &Session) -> Result<()> {
        let forgotten = self.forget(session).await;
        let _ = session.logout().await;
        forgotten
    }

    async fn forget(&self, session: &Session) -> Result<()> {
        let server_url = session.server().base_url().to_string();
        let user_id = session.user_id().to_string();
        let account = account_key(&server_url, &user_id);
        let token = session.token().to_owned();
        let cleared = self
            .vault_call(move |vault| match vault.load(&account) {
                Ok(Some((stored, _))) if stored != token => Ok(false),
                _ => vault.clear(&account).map(|()| true),
            })
            .await?;
        if !cleared {
            return Ok(());
        }
        self.update(|config| {
            if config
                .session
                .as_ref()
                .is_some_and(|saved| saved.server_url == server_url && saved.user_id == user_id)
            {
                config.session = None;
            }
        })
    }

    /// Records that the user has seen the plain-file token warning.
    pub fn acknowledge_file_token_warning(&self) -> Result<()> {
        self.update(|config| config.file_token_warning_shown = true)
    }
}
