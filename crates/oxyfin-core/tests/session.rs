//! Integration tests for the session manager.

mod common;

use std::sync::Arc;

use common::{FailingStore, fixture, manager, memory_vault};
use oxyfin_core::{
    CoreError, MemoryStore, Paths, SecretStore, SessionManager, SignedIn, TokenStorage, TokenVault,
    account_key,
};
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

fn assert_token_gone(primary: &MemoryStore, fallback: &MemoryStore, signed: &SignedIn) {
    let account = account_key(&signed.server_url, signed.session.user_id().as_str());
    assert_eq!(primary.get(&account), Ok(None));
    assert_eq!(fallback.get(&account), Ok(None));
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
    let primary = Arc::new(MemoryStore::default());
    let fallback = Arc::new(MemoryStore::default());
    let manager = manager(
        &mock,
        &dir,
        TokenVault::new(primary.clone(), fallback.clone()),
    );
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
    assert_eq!(manager.last_server_url(), Some(signed.server_url.clone()));
    assert_token_gone(&primary, &fallback, &signed);
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
    let primary = Arc::new(MemoryStore::default());
    let fallback = Arc::new(MemoryStore::default());
    let manager = manager(
        &mock,
        &dir,
        TokenVault::new(primary.clone(), fallback.clone()),
    );
    let handle = manager.connect(&mock.uri()).await.unwrap();
    let signed = manager.sign_in(&handle, "oxyfin", "pw").await.unwrap();
    manager.sign_out(&signed.session).await.unwrap();
    assert!(manager.restore().await.unwrap().is_none());
    assert_token_gone(&primary, &fallback, &signed);
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

mod quick_connect {
    use std::time::Duration;

    use super::*;
    use wiremock::matchers::query_param;

    async fn mount_enabled(mock: &MockServer, enabled: bool) {
        Mock::given(method("GET"))
            .and(path("/QuickConnect/Enabled"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!(enabled)))
            .mount(mock)
            .await;
    }

    async fn mount_initiate(mock: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/QuickConnect/Initiate"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("quick_connect_initiate")),
            )
            .mount(mock)
            .await;
    }

    fn state(authenticated: bool) -> serde_json::Value {
        let mut body = fixture("quick_connect_connect");
        body["Authenticated"] = json!(authenticated);
        body
    }

    async fn poll_count(mock: &MockServer) -> usize {
        mock.received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|request| request.url.path() == "/QuickConnect/Connect")
            .count()
    }

    #[tokio::test]
    async fn quick_connect_waits_for_approval() {
        let mock = MockServer::start().await;
        mount_public_info(&mock).await;
        mount_sign_in(&mock).await;
        mount_enabled(&mock, true).await;
        mount_initiate(&mock).await;
        for (body, times) in [(state(false), 2), (state(true), 1)] {
            Mock::given(method("GET"))
                .and(path("/QuickConnect/Connect"))
                .and(query_param(
                    "secret",
                    fixture("quick_connect_connect")["Secret"].as_str().unwrap(),
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .up_to_n_times(times)
                .expect(times)
                .mount(&mock)
                .await;
        }
        Mock::given(method("POST"))
            .and(path("/Users/AuthenticateWithQuickConnect"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("authenticate_by_name")))
            .mount(&mock)
            .await;
        let dir = TempDir::new().unwrap();
        let manager = manager(&mock, &dir, memory_vault());
        let handle = manager.connect(&mock.uri()).await.unwrap();
        let flow = manager.start_quick_connect(&handle).await.unwrap();
        assert_eq!(flow.code, "668908");
        let signed = manager.finish_quick_connect(&handle, flow).await.unwrap();
        assert_eq!(signed.user_name, "oxyfin");
        assert_eq!(poll_count(&mock).await, 3);
        assert!(manager.restore().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn quick_connect_disabled() {
        let mock = MockServer::start().await;
        mount_public_info(&mock).await;
        mount_enabled(&mock, false).await;
        let dir = TempDir::new().unwrap();
        let manager = manager(&mock, &dir, memory_vault());
        let handle = manager.connect(&mock.uri()).await.unwrap();
        let error = manager.start_quick_connect(&handle).await.unwrap_err();
        assert_eq!(error, CoreError::QuickConnectDisabled);
    }

    #[tokio::test]
    async fn quick_connect_initiate_401_is_disabled() {
        let mock = MockServer::start().await;
        mount_public_info(&mock).await;
        mount_enabled(&mock, true).await;
        Mock::given(method("POST"))
            .and(path("/QuickConnect/Initiate"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock)
            .await;
        let dir = TempDir::new().unwrap();
        let manager = manager(&mock, &dir, memory_vault());
        let handle = manager.connect(&mock.uri()).await.unwrap();
        let error = manager.start_quick_connect(&handle).await.unwrap_err();
        assert_eq!(error, CoreError::QuickConnectDisabled);
    }

    #[tokio::test]
    async fn quick_connect_expired() {
        for status in [401, 404] {
            let mock = MockServer::start().await;
            mount_public_info(&mock).await;
            mount_enabled(&mock, true).await;
            mount_initiate(&mock).await;
            Mock::given(method("GET"))
                .and(path("/QuickConnect/Connect"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&mock)
                .await;
            let dir = TempDir::new().unwrap();
            let manager = manager(&mock, &dir, memory_vault());
            let handle = manager.connect(&mock.uri()).await.unwrap();
            let flow = manager.start_quick_connect(&handle).await.unwrap();
            let error = manager
                .finish_quick_connect(&handle, flow)
                .await
                .unwrap_err();
            assert_eq!(error, CoreError::QuickConnectExpired, "status {status}");
        }
    }

    #[tokio::test]
    async fn dropping_the_flow_stops_polling() {
        let mock = MockServer::start().await;
        mount_public_info(&mock).await;
        mount_enabled(&mock, true).await;
        mount_initiate(&mock).await;
        Mock::given(method("GET"))
            .and(path("/QuickConnect/Connect"))
            .respond_with(ResponseTemplate::new(200).set_body_json(state(false)))
            .mount(&mock)
            .await;
        let dir = TempDir::new().unwrap();
        let manager = manager(&mock, &dir, memory_vault())
            .with_quick_connect_interval(Duration::from_secs(1));
        let handle = manager.connect(&mock.uri()).await.unwrap();
        let flow = manager.start_quick_connect(&handle).await.unwrap();
        let first_poll_seen = async {
            while poll_count(&mock).await < 1 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        tokio::select! {
            _ = manager.finish_quick_connect(&handle, flow) => panic!("the flow ended"),
            seen = tokio::time::timeout(Duration::from_secs(10), first_poll_seen) => {
                assert!(seen.is_ok(), "no poll reached the server");
            }
        }
        tokio::time::sleep(Duration::from_millis(3500)).await;
        assert_eq!(poll_count(&mock).await, 1);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn failed_config_save_keeps_the_previous_token() {
    use std::os::unix::fs::PermissionsExt;

    let mock = MockServer::start().await;
    mount_public_info(&mock).await;
    mount_sign_in(&mock).await;
    let dir = TempDir::new().unwrap();
    let primary = Arc::new(MemoryStore::default());
    let fallback = Arc::new(MemoryStore::default());
    let manager = manager(&mock, &dir, TokenVault::new(primary.clone(), fallback));
    let handle = manager.connect(&mock.uri()).await.unwrap();
    let first = manager.sign_in(&handle, "oxyfin", "pw").await.unwrap();
    let old_account = account_key(&first.server_url, first.session.user_id().as_str());

    let mut other = fixture("authenticate_by_name");
    other["User"]["Id"] = json!("ffffffffffffffffffffffffffffffff");
    mock.reset().await;
    Mock::given(method("POST"))
        .and(path("/Users/AuthenticateByName"))
        .respond_with(ResponseTemplate::new(200).set_body_json(other))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/Users/Me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("users_me")))
        .mount(&mock)
        .await;
    let config_dir = Paths::under(dir.path()).config_dir;
    std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = manager.sign_in(&handle, "other", "pw").await;
    std::fs::set_permissions(&config_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

    assert!(matches!(result, Err(CoreError::Storage(_))));
    let restored = manager.restore().await.unwrap().unwrap();
    assert_eq!(restored.session.user_id(), first.session.user_id());
    assert!(primary.get(&old_account).unwrap().is_some());
}
