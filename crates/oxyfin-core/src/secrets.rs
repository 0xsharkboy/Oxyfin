//! Access token storage: system keyring with a private-file fallback.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::{CoreError, Paths, Result};

const SERVICE: &str = "oxyfin";
const FILE_NAME: &str = "tokens.json";

fn storage<E: std::fmt::Display>(error: E) -> CoreError {
    CoreError::Storage(error.to_string())
}

/// Builds the key under which a token is stored for a server and user.
pub fn account_key(server_url: &str, user_id: &str) -> String {
    format!("{user_id}@{server_url}")
}

/// A blocking key-value store for access tokens.
pub trait SecretStore: Send + Sync {
    /// Returns the token stored for `account`, if any.
    fn get(&self, account: &str) -> Result<Option<String>>;
    /// Stores `token` for `account`, replacing any previous value.
    fn set(&self, account: &str, token: &str) -> Result<()>;
    /// Removes the token for `account`. Succeeds when none is stored.
    fn delete(&self, account: &str) -> Result<()>;
}

/// Stores tokens in the system keyring. Calls block.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

impl KeyringStore {
    fn entry(account: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, account).map_err(storage)
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        match Self::entry(account)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(storage(error)),
        }
    }

    fn set(&self, account: &str, token: &str) -> Result<()> {
        Self::entry(account)?.set_password(token).map_err(storage)
    }

    fn delete(&self, account: &str) -> Result<()> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(storage(error)),
        }
    }
}

/// Stores tokens in `data_dir/tokens.json`, readable by the owner only on Unix.
///
/// An unreadable file is moved to `tokens.json.bad` and treated as empty.
#[derive(Debug)]
pub struct FileStore {
    file: PathBuf,
    lock: Mutex<()>,
}

impl FileStore {
    /// Creates a store backed by `tokens.json` in the data directory.
    pub fn new(paths: &Paths) -> Self {
        Self {
            file: paths.data_dir.join(FILE_NAME),
            lock: Mutex::new(()),
        }
    }

    fn guard(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn read(&self) -> Result<HashMap<String, String>> {
        match fs::read(&self.file) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(tokens) => Ok(tokens),
                Err(_) => {
                    fs::rename(&self.file, self.file.with_extension("json.bad"))
                        .map_err(storage)?;
                    Ok(HashMap::new())
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(error) => Err(storage(error)),
        }
    }

    fn write(&self, tokens: &HashMap<String, String>) -> Result<()> {
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir).map_err(storage)?;
        }
        let json = serde_json::to_vec_pretty(tokens).map_err(storage)?;
        let tmp = self.file.with_extension("json.tmp");
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&tmp).map_err(storage)?;
        file.write_all(&json).map_err(storage)?;
        file.sync_all().map_err(storage)?;
        fs::rename(&tmp, &self.file).map_err(storage)
    }
}

impl SecretStore for FileStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        let _guard = self.guard();
        Ok(self.read()?.remove(account))
    }

    fn set(&self, account: &str, token: &str) -> Result<()> {
        let _guard = self.guard();
        let mut tokens = self.read()?;
        tokens.insert(account.to_owned(), token.to_owned());
        self.write(&tokens)
    }

    fn delete(&self, account: &str) -> Result<()> {
        let _guard = self.guard();
        let mut tokens = self.read()?;
        if tokens.remove(account).is_some() {
            self.write(&tokens)?;
        }
        Ok(())
    }
}

/// Keeps tokens in memory, for tests.
#[derive(Default)]
pub struct MemoryStore {
    tokens: Mutex<HashMap<String, String>>,
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore").finish_non_exhaustive()
    }
}

impl MemoryStore {
    fn tokens(&self) -> MutexGuard<'_, HashMap<String, String>> {
        self.tokens.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        Ok(self.tokens().get(account).cloned())
    }

    fn set(&self, account: &str, token: &str) -> Result<()> {
        self.tokens().insert(account.to_owned(), token.to_owned());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        self.tokens().remove(account);
        Ok(())
    }
}

/// Where a token ended up being stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenStorage {
    /// The system keyring.
    Keyring,
    /// The private fallback file.
    File,
}

/// Saves tokens to a primary store and falls back to a secondary one.
pub struct TokenVault {
    primary: Arc<dyn SecretStore>,
    fallback: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for TokenVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenVault").finish_non_exhaustive()
    }
}

impl TokenVault {
    /// Builds a vault from a primary store and a fallback store.
    pub fn new(primary: Arc<dyn SecretStore>, fallback: Arc<dyn SecretStore>) -> Self {
        Self { primary, fallback }
    }

    /// Builds the production vault: system keyring, then a file in the data directory.
    pub fn system(paths: &Paths) -> Self {
        Self::new(Arc::new(KeyringStore), Arc::new(FileStore::new(paths)))
    }

    /// Saves the token to the primary store, or to the fallback when that fails.
    ///
    /// The other store is cleaned on a best-effort basis so a stale token cannot shadow this one.
    pub fn save(&self, account: &str, token: &str) -> Result<TokenStorage> {
        match self.primary.set(account, token) {
            Ok(()) => {
                let _ = self.fallback.delete(account);
                Ok(TokenStorage::Keyring)
            }
            Err(_) => {
                self.fallback.set(account, token)?;
                let _ = self.primary.delete(account);
                Ok(TokenStorage::File)
            }
        }
    }

    /// Loads the token from the primary store, then from the fallback.
    ///
    /// A primary error counts as a missing token so a machine without a keyring still works.
    pub fn load(&self, account: &str) -> Result<Option<(String, TokenStorage)>> {
        if let Ok(Some(token)) = self.primary.get(account) {
            return Ok(Some((token, TokenStorage::Keyring)));
        }
        Ok(self
            .fallback
            .get(account)?
            .map(|token| (token, TokenStorage::File)))
    }

    /// Removes the token from both stores.
    ///
    /// A primary failure is ignored once the fallback is clean, since the keyring may be absent.
    pub fn clear(&self, account: &str) -> Result<()> {
        let _ = self.primary.delete(account);
        self.fallback.delete(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingStore;

    impl SecretStore for FailingStore {
        fn get(&self, _: &str) -> Result<Option<String>> {
            Err(CoreError::Storage("unavailable".into()))
        }
        fn set(&self, _: &str, _: &str) -> Result<()> {
            Err(CoreError::Storage("unavailable".into()))
        }
        fn delete(&self, _: &str) -> Result<()> {
            Err(CoreError::Storage("unavailable".into()))
        }
    }

    #[derive(Default)]
    struct SetFailsStore(MemoryStore);

    impl SecretStore for SetFailsStore {
        fn get(&self, account: &str) -> Result<Option<String>> {
            self.0.get(account)
        }
        fn set(&self, _: &str, _: &str) -> Result<()> {
            Err(CoreError::Storage("read only".into()))
        }
        fn delete(&self, account: &str) -> Result<()> {
            self.0.delete(account)
        }
    }

    #[test]
    fn clear_succeeds_when_only_the_primary_fails() {
        let vault = TokenVault::new(Arc::new(FailingStore), Arc::new(MemoryStore::default()));
        vault.save("a", "t").unwrap();
        vault.clear("a").unwrap();
        assert_eq!(vault.load("a").unwrap(), None);
    }

    #[test]
    fn primary_save_removes_the_stale_fallback_token() {
        let primary = Arc::new(MemoryStore::default());
        let fallback = Arc::new(MemoryStore::default());
        fallback.set("a", "old").unwrap();
        let vault = TokenVault::new(primary, fallback.clone());
        vault.save("a", "new").unwrap();
        assert_eq!(fallback.get("a").unwrap(), None);
    }

    #[test]
    fn fallback_save_removes_the_stale_primary_token() {
        let primary = Arc::new(SetFailsStore::default());
        primary.0.set("a", "old").unwrap();
        let vault = TokenVault::new(primary, Arc::new(MemoryStore::default()));
        assert_eq!(vault.save("a", "new").unwrap(), TokenStorage::File);
        assert_eq!(
            vault.load("a").unwrap(),
            Some(("new".into(), TokenStorage::File))
        );
    }

    fn temp() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        (dir, paths)
    }

    #[test]
    fn account_key_joins_user_and_server() {
        assert_eq!(
            account_key("https://jf.example", "u1"),
            "u1@https://jf.example"
        );
    }

    #[test]
    fn vault_prefers_the_primary() {
        let primary = Arc::new(MemoryStore::default());
        let fallback = Arc::new(MemoryStore::default());
        let vault = TokenVault::new(primary.clone(), fallback.clone());
        assert_eq!(vault.save("a", "t").unwrap(), TokenStorage::Keyring);
        assert_eq!(primary.get("a").unwrap().as_deref(), Some("t"));
        assert_eq!(fallback.get("a").unwrap(), None);
        assert_eq!(
            vault.load("a").unwrap(),
            Some(("t".into(), TokenStorage::Keyring))
        );
    }

    #[test]
    fn vault_falls_back_when_the_primary_fails() {
        let vault = TokenVault::new(Arc::new(FailingStore), Arc::new(MemoryStore::default()));
        assert_eq!(vault.save("a", "t").unwrap(), TokenStorage::File);
        assert_eq!(
            vault.load("a").unwrap(),
            Some(("t".into(), TokenStorage::File))
        );
    }

    #[test]
    fn clear_removes_both() {
        let primary = Arc::new(MemoryStore::default());
        let fallback = Arc::new(MemoryStore::default());
        primary.set("a", "t").unwrap();
        fallback.set("a", "t").unwrap();
        let vault = TokenVault::new(primary.clone(), fallback.clone());
        vault.clear("a").unwrap();
        assert_eq!(primary.get("a").unwrap(), None);
        assert_eq!(fallback.get("a").unwrap(), None);
        assert_eq!(vault.load("a").unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn file_store_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, paths) = temp();
        let store = FileStore::new(&paths);
        store.set("a", "t").unwrap();
        let mode = fs::metadata(paths.data_dir.join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn file_store_keeps_other_accounts() {
        let (_dir, paths) = temp();
        let store = FileStore::new(&paths);
        store.set("a", "ta").unwrap();
        store.set("b", "tb").unwrap();
        store.delete("a").unwrap();
        assert_eq!(store.get("a").unwrap(), None);
        assert_eq!(store.get("b").unwrap().as_deref(), Some("tb"));
        store.delete("missing").unwrap();
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let (_dir, paths) = temp();
        fs::create_dir_all(&paths.data_dir).unwrap();
        fs::write(paths.data_dir.join(FILE_NAME), b"{not json").unwrap();
        let store = FileStore::new(&paths);
        assert_eq!(store.get("a").unwrap(), None);
        assert!(paths.data_dir.join("tokens.json.bad").exists());
        store.set("a", "t").unwrap();
        assert_eq!(store.get("a").unwrap().as_deref(), Some("t"));
    }

    #[test]
    fn debug_output_hides_tokens() {
        let store = MemoryStore::default();
        store.set("a", "secret-token").unwrap();
        assert!(!format!("{store:?}").contains("secret-token"));
    }
}
