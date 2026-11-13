//! Helpers shared by the opt-in tests against a real Jellyfin server.
#![allow(dead_code, clippy::expect_used)]

use oxyfin_api::{
    BaseItemDto, BaseItemKind, ClientInfo, ItemsQuery, MediaSourceInfo, Server, Session,
};

/// Connection settings read from the `OXYFIN_TEST_*` environment variables.
pub struct Config {
    pub url: String,
    pub user: String,
    pub password: String,
}

/// Reads the settings, failing with the names of every missing variable.
pub fn config() -> Config {
    let read = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let url = read("OXYFIN_TEST_SERVER");
    let user = read("OXYFIN_TEST_USER");
    let password = read("OXYFIN_TEST_PASSWORD");
    let missing: Vec<&str> = [
        ("OXYFIN_TEST_SERVER", url.is_none()),
        ("OXYFIN_TEST_USER", user.is_none()),
        ("OXYFIN_TEST_PASSWORD", password.is_none()),
    ]
    .into_iter()
    .filter_map(|(name, absent)| absent.then_some(name))
    .collect();
    assert!(
        missing.is_empty(),
        "missing environment variables: {}. Start a test server with \
         `eval \"$(scripts/jellyfin-test-server.sh up 12.4)\"`",
        missing.join(", ")
    );
    match (url, user, password) {
        (Some(url), Some(user), Some(password)) => Config {
            url,
            user,
            password,
        },
        _ => unreachable!("checked above"),
    }
}

/// Fails unless the server is declared disposable.
pub fn require_disposable() {
    assert_eq!(
        std::env::var("OXYFIN_TEST_DISPOSABLE").as_deref(),
        Ok("1"),
        "OXYFIN_TEST_DISPOSABLE must be 1: these tests write to the server and must only \
         target the disposable Docker server"
    );
}

/// Builds a [`Server`] for the configured URL with the given device name.
pub fn server_with_device(config: &Config, device_name: &str) -> Server {
    let info = ClientInfo::new(device_name, "oxyfin-live-test");
    Server::new(&config.url, info).expect("valid server URL")
}

/// Builds a [`Server`] with a default test device name.
pub fn server(config: &Config) -> Server {
    server_with_device(config, "Oxyfin live test")
}

/// Signs in with the configured credentials.
pub async fn login(config: &Config) -> Session {
    server(config)
        .authenticate_by_name(&config.user, &config.password)
        .await
        .expect("sign in")
}

/// Returns the first movie of the library.
pub async fn first_movie(session: &Session) -> Option<BaseItemDto> {
    let query = ItemsQuery {
        include_item_types: vec![BaseItemKind::Movie],
        recursive: true,
        limit: Some(1),
        ..ItemsQuery::default()
    };
    let result = session.items(&query).await.expect("list movies");
    result.items.into_iter().next()
}

/// Returns the first movie, failing when the server has none.
pub async fn movie(session: &Session) -> BaseItemDto {
    first_movie(session)
        .await
        .expect("the test server has no movie")
}

/// Returns the first media source of an item.
pub async fn media_source(session: &Session, item: &BaseItemDto) -> MediaSourceInfo {
    let info = session
        .playback_info(&item.id)
        .await
        .expect("playback info");
    info.media_sources
        .into_iter()
        .next()
        .expect("the item has no media source")
}
