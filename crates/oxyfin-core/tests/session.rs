//! Integration tests for the session manager.

mod common;

use std::sync::Arc;

use common::{FailingStore, fixture, manager, memory_vault};
use oxyfin_core::{CoreError, MemoryStore, Paths, SessionManager, TokenStorage, TokenVault};
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mount_public_info(mock: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/System/Info/Public"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("system_info_public")))
        .mount(mock)
        .await;
}

async fn mount_sign_in(mock: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/Users/AuthenticateByName"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("authenticate_by_name")))
        .mount(mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Users/Me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("users_me")))
        .mount(mock)
        .await;
}

#[tokio::test]
async fn connect_reports_name_and_version() {
    let mock = MockServer::start().await;
    mount_public_info(&mock).await;
    let dir = TempDir::new().unwrap();
    let handle = manager(&mock, &dir, memory_vault())
        .connect(&mock.uri())
        .await
        .unwrap();
    assert_eq!(handle.name.as_deref(), Some("49667124a8b5"));
    assert_eq!(handle.version, "12.4.0");
}

#[tokio::test]
async fn connect_rejects_old_server() {
    let mock = MockServer::start().await;
    let mut body = fixture("system_info_public");
    body["Version"] = json!("10.9.0");
    Mock::given(method("GET"))
        .and(path("/System/Info/Public"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&mock)
        .await;
    let dir = TempDir::new().unwrap();
    let error = manager(&mock, &dir, memory_vault())
        .connect(&mock.uri())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CoreError::UnsupportedServer {
            found: "10.9.0".into(),
            minimum: "10.10.0".into()
        }
    );
}

#[tokio::test]
async fn sign_in_persists_and_restores() {
    let mock = MockServer::start().await;
    mount_public_info(&mock).await;
    mount_sign_in(&mock).await;
    let dir = TempDir::new().unwrap();
    let primary = Arc::new(MemoryStore::default());
    let fallback = Arc::new(MemoryStore::default());
    let vault = || TokenVault::new(primary.clone(), fallback.clone());

    let first = manager(&mock, &dir, vault());
    let handle = first.connect(&mock.uri()).await.unwrap();
    let signed = first.sign_in(&handle, "oxyfin", "pw").await.unwrap();
    assert_eq!(signed.user_name, "oxyfin");
    assert_eq!(signed.token_storage, TokenStorage::Keyring);
    assert!(!signed.show_file_token_warning);
    assert!(!format!("{signed:?}").contains(signed.session.token()));

    mock.reset().await;
    let second = manager(&mock, &dir, vault());
    let restored = second.restore().await.unwrap().unwrap();
    assert_eq!(restored.session.token(), signed.session.token());
    assert_eq!(restored.session.user_id(), signed.session.user_id());
    assert_eq!(restored.user_name, "oxyfin");
    assert_eq!(restored.server_url, signed.server_url);
    assert!(mock.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn wrong_password_is_invalid_credentials() {
    let mock = MockServer::start().await;
    mount_public_info(&mock).await;
    Mock::given(method("POST"))
        .and(path("/Users/AuthenticateByName"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock)
        .await;
    let dir = TempDir::new().unwrap();
    let manager = manager(&mock, &dir, memory_vault());
    let handle = manager.connect(&mock.uri()).await.unwrap();
    let error = manager.sign_in(&handle, "oxyfin", "bad").await.unwrap_err();
    assert_eq!(error, CoreError::InvalidCredentials);
}

#[tokio::test]
async fn validate_401_expires_the_session() {
    let mock = MockServer::start().await;
    mount_public_info(&mock).await;
    mount_sign_in(&mock).await;
    let dir = TempDir::new().unwrap();
    let manager = manager(&mock, &dir, memory_vault());
    let handle = manager.connect(&mock.uri()).await.unwrap();
    let signed = manager.sign_in(&handle, "oxyfin", "pw").await.unwrap();

    mock.reset().await;
    Mock::given(method("GET"))
        .and(path("/Users/Me"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock)
        .await;
    let error = manager.validate(&signed.session).await.unwrap_err();
    assert_eq!(error, CoreError::SessionExpired);
    assert!(manager.restore().await.unwrap().is_none());
    assert_eq!(manager.last_server_url(), Some(signed.server_url));
}

#[tokio::test]
async fn sign_out_clears_even_if_logout_fails() {
    let mock = MockServer::start().await;
    mount_public_info(&mock).await;
    mount_sign_in(&mock).await;
    Mock::given(method("POST"))
        .and(path("/Sessions/Logout"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;
    let dir = TempDir::new().unwrap();
    let manager = manager(&mock, &dir, memory_vault());
    let handle = manager.connect(&mock.uri()).await.unwrap();
    let signed = manager.sign_in(&handle, "oxyfin", "pw").await.unwrap();
    manager.sign_out(&signed.session).await.unwrap();
    assert!(manager.restore().await.unwrap().is_none());
}

#[tokio::test]
async fn file_fallback_warns_once() {
    let mock = MockServer::start().await;
    mount_public_info(&mock).await;
    mount_sign_in(&mock).await;
    let dir = TempDir::new().unwrap();
    let fallback = Arc::new(MemoryStore::default());
    let vault = || TokenVault::new(Arc::new(FailingStore), fallback.clone());

    let first = manager(&mock, &dir, vault());
    let handle = first.connect(&mock.uri()).await.unwrap();
    let signed = first.sign_in(&handle, "oxyfin", "pw").await.unwrap();
    assert_eq!(signed.token_storage, TokenStorage::File);
    assert!(signed.show_file_token_warning);

    first.acknowledge_file_token_warning().unwrap();
    let second = SessionManager::new(Paths::under(dir.path()), vault(), "Test Device").unwrap();
    let restored = second.restore().await.unwrap().unwrap();
    assert_eq!(restored.token_storage, TokenStorage::File);
    assert!(!restored.show_file_token_warning);
}
