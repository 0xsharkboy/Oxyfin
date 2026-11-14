//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use oxyfin_api::{ClientInfo, Server, Session, UserId};
use oxyfin_core::{CoreError, MemoryStore, Paths, Result, SecretStore, SessionManager, TokenVault};
use serde_json::Value;
use tempfile::TempDir;
use wiremock::MockServer;

/// Token used by [`session`].
pub const TEST_TOKEN: &str = "test-token";

/// User id used by [`session`].
pub const TEST_USER_ID: &str = "0123456789abcdef0123456789abcdef";

/// Loads `tests/fixtures/<name>.json`.
pub fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("bad JSON in {}: {err}", path.display()))
}

/// A session restored on `mock` with [`TEST_TOKEN`] and [`TEST_USER_ID`].
pub fn session(mock: &MockServer) -> Session {
    Server::new(
        &mock.uri(),
        ClientInfo::new("Test Device", "test-device-id"),
    )
    .unwrap_or_else(|err| panic!("cannot create a server for the mock: {err}"))
    .restore_session(TEST_TOKEN, UserId::from(TEST_USER_ID), None)
}

/// A vault whose primary and fallback stores are fresh in-memory stores.
pub fn memory_vault() -> TokenVault {
    TokenVault::new(
        Arc::new(MemoryStore::default()),
        Arc::new(MemoryStore::default()),
    )
}

/// A manager storing its files under `dir`. The mock is not used: servers come from `connect`.
pub fn manager(_mock: &MockServer, dir: &TempDir, vault: TokenVault) -> SessionManager {
    SessionManager::new(Paths::under(dir.path()), vault, "Test Device")
        .unwrap_or_else(|err| panic!("cannot create the session manager: {err}"))
}

/// A store whose every operation fails.
pub struct FailingStore;

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
