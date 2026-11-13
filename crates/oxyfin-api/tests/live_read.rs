//! Read-only tests against a real Jellyfin server.
//!
//! Run with `cargo test -p oxyfin-api --test live_read -- --ignored` after starting
//! `scripts/jellyfin-test-server.sh`.

mod live_common;

use live_common::{config, first_movie, login, server, server_with_device};
use oxyfin_api::{ImageRequest, ImageType, ItemsQuery, MediaStreamType};
use serde_json::Value;

const DEVICE_NAME: &str = r#"Bob's "PC", été"#;
const DEVICE_NAME_ENCODED: &str = "Bob%27s%20%22PC%22%2C%20%C3%A9t%C3%A9";

#[tokio::test]
#[ignore = "needs a Jellyfin server, see CONTRIBUTING.md"]
async fn browse_and_read() {
    let config = config();
    server(&config)
        .check_version()
        .await
        .expect("version check");

    let session = login(&config).await;
    let me = session.me().await.expect("me");
    assert_eq!(me.name, config.user);

    let views = session.user_views().await.expect("user views");
    assert!(!views.items.is_empty(), "no library view");

    let query = ItemsQuery {
        recursive: true,
        limit: Some(5),
        ..ItemsQuery::default()
    };
    let listed = session.items(&query).await.expect("items");
    assert!(!listed.items.is_empty() && listed.items.len() <= 5);
    let first = &listed.items[0];
    let fetched = session.item(&first.id).await.expect("item");
    assert_eq!(fetched.id, first.id);

    session.resume(12).await.expect("resume");
    session.next_up(None, 12).await.expect("next up");
    session.latest(None, 12).await.expect("latest");

    session.logout().await.expect("logout");
}

#[tokio::test]
#[ignore = "needs a Jellyfin server, see CONTRIBUTING.md"]
async fn playback_info_and_image() {
    let config = config();
    let session = login(&config).await;

    let movie = first_movie(&session)
        .await
        .expect("the server has no movie");
    let info = session
        .playback_info(&movie.id)
        .await
        .expect("playback info");
    let source = info.media_sources.first().expect("no media source");
    assert!(
        source
            .media_streams
            .iter()
            .any(|stream| stream.r#type == MediaStreamType::Video)
    );
    assert!(
        source
            .media_streams
            .iter()
            .any(|stream| stream.r#type == MediaStreamType::Audio)
    );

    assert!(
        movie.image_tags.contains_key(&ImageType::Primary),
        "the movie has no Primary image"
    );
    let mut request = ImageRequest::new(movie.id.clone(), ImageType::Primary);
    request.max_width = Some(100);
    let bytes = session.fetch_image(&request).await.expect("fetch image");
    assert!(bytes.len() > 100, "image too small: {} bytes", bytes.len());

    session.logout().await.expect("logout");
}

#[tokio::test]
#[ignore = "needs a Jellyfin server, see CONTRIBUTING.md"]
async fn device_name_round_trips() {
    let config = config();
    let authorization = format!(
        r#"MediaBrowser Client="Oxyfin", Device="{DEVICE_NAME_ENCODED}", DeviceId="oxyfin-live-raw", Version="0.0.0""#
    );
    let base = config.url.trim_end_matches('/');
    let http = reqwest::Client::new();
    let response: Value = http
        .post(format!("{base}/Users/AuthenticateByName"))
        .header("Authorization", &authorization)
        .header("Accept", "application/json")
        .json(&serde_json::json!({ "Username": config.user, "Pw": config.password }))
        .send()
        .await
        .expect("raw sign in")
        .error_for_status()
        .expect("raw sign in status")
        .json()
        .await
        .expect("raw sign in body");

    let field = |value: &Value, name: &str| {
        let lower = name.to_ascii_lowercase();
        value.as_object().and_then(|object| {
            object
                .iter()
                .find(|(key, _)| key.to_ascii_lowercase() == lower)
                .map(|(_, value)| value.clone())
        })
    };
    let session_info = field(&response, "SessionInfo").expect("no SessionInfo in the response");
    let device_name = field(&session_info, "DeviceName").expect("no DeviceName in SessionInfo");
    assert_eq!(device_name.as_str(), Some(DEVICE_NAME));

    let token = field(&response, "AccessToken").expect("no access token");
    let token = token.as_str().expect("access token is a string");
    http.post(format!("{base}/Sessions/Logout"))
        .header(
            "Authorization",
            format!(r#"{authorization}, Token="{token}""#),
        )
        .send()
        .await
        .expect("raw logout")
        .error_for_status()
        .expect("raw logout status");

    let session = server_with_device(&config, DEVICE_NAME)
        .authenticate_by_name(&config.user, &config.password)
        .await
        .expect("sign in with the awkward device name");
    session.me().await.expect("me");
    session.logout().await.expect("logout");
}
