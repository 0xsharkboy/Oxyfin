//! Tests that change server state, against the disposable Jellyfin server only.
//!
//! Run with `cargo test -p oxyfin-api --test live_write -- --ignored --test-threads=1`.
//! They fail at once unless `OXYFIN_TEST_DISPOSABLE=1`, which
//! `scripts/jellyfin-test-server.sh up` sets.

mod live_common;

use live_common::{config, login, media_source, movie, require_disposable, server};
use oxyfin_api::{ItemsQuery, MediaStreamType, PlaybackReport, Ticks};
use std::time::Duration;

#[tokio::test]
#[ignore = "writes to the server, see CONTRIBUTING.md"]
async fn played_and_favorite_round_trip() {
    require_disposable();
    let session = login(&config()).await;
    let movie = movie(&session).await;

    let data = session.set_played(&movie.id, true).await.expect("played");
    assert!(data.played);
    let data = session
        .set_played(&movie.id, false)
        .await
        .expect("unplayed");
    assert!(!data.played);

    let data = session
        .set_favorite(&movie.id, true)
        .await
        .expect("favorite");
    assert!(data.is_favorite);
    let data = session
        .set_favorite(&movie.id, false)
        .await
        .expect("unfavorite");
    assert!(!data.is_favorite);

    session.logout().await.expect("logout");
}

#[tokio::test]
#[ignore = "writes to the server, see CONTRIBUTING.md"]
async fn progress_reports_create_a_resume_entry() {
    require_disposable();
    let session = login(&config()).await;
    let movie = movie(&session).await;
    session.set_played(&movie.id, false).await.expect("reset");

    let info = session
        .playback_info(&movie.id)
        .await
        .expect("playback info");
    let source = info.media_sources.first().expect("no media source");
    let mut report = PlaybackReport {
        item_id: movie.id.clone(),
        media_source_id: source.id.clone(),
        play_session_id: info.play_session_id.clone(),
        position: Ticks(0),
        is_paused: false,
        audio_stream_index: source.default_audio_stream_index,
        subtitle_stream_index: None,
    };
    session.report_playback_start(&report).await.expect("start");
    report.position = Ticks::from_duration(Duration::from_secs(4));
    session
        .report_playback_progress(&report)
        .await
        .expect("progress");
    session
        .report_playback_stopped(&report)
        .await
        .expect("stopped");

    let resume = session.resume(50).await.expect("resume");
    // On 12.4 the list also holds the Season and Series items.
    let entry = resume
        .items
        .iter()
        .find(|item| item.id == movie.id)
        .expect("the movie is not in the resume list");
    let data = entry.user_data.as_ref().expect("no user data");
    assert!(data.playback_position_ticks > Ticks(0));

    session.set_played(&movie.id, false).await.expect("cleanup");
    session.logout().await.expect("logout");
}

#[tokio::test]
#[ignore = "writes to the server, see CONTRIBUTING.md"]
async fn quick_connect_is_pending_until_approved() {
    require_disposable();
    let config = config();
    let server = server(&config);
    assert!(server.quick_connect_enabled().await.expect("enabled"));
    let state = server.quick_connect_initiate().await.expect("initiate");
    assert!(!state.code.is_empty());
    assert!(!state.authenticated);
    let polled = server
        .quick_connect_poll(&state.secret)
        .await
        .expect("poll");
    assert!(!polled.authenticated);
}

#[tokio::test]
#[ignore = "writes to the server, see CONTRIBUTING.md"]
async fn external_subtitle_is_downloadable() {
    require_disposable();
    let session = login(&config()).await;
    let movie = movie(&session).await;
    let source = media_source(&session, &movie).await;
    let subtitle = source
        .media_streams
        .iter()
        .find(|stream| stream.r#type == MediaStreamType::Subtitle && stream.is_external)
        .expect("no external subtitle");

    let url = session.subtitle_url(&movie.id, &source.id, subtitle.index, "srt");
    let authorization = session.stream_source(&movie.id, &source.id).authorization;
    let body = reqwest::Client::new()
        .get(url)
        .header("Authorization", authorization)
        .send()
        .await
        .expect("download")
        .error_for_status()
        .expect("download status")
        .text()
        .await
        .expect("body");
    assert!(body.contains("Hello"), "unexpected subtitle: {body:?}");

    session.logout().await.expect("logout");
}

#[tokio::test]
#[ignore = "writes to the server, see CONTRIBUTING.md"]
async fn logout_invalidates_the_token() {
    require_disposable();
    let session = login(&config()).await;
    session.me().await.expect("me before logout");
    session.logout().await.expect("logout");
    let err = session.me().await.expect_err("me after logout");
    assert!(err.is_unauthorized(), "unexpected error: {err}");
}

#[tokio::test]
#[ignore = "writes to the server, see CONTRIBUTING.md"]
async fn audio_streams_are_listed() {
    require_disposable();
    let session = login(&config()).await;
    let movie = movie(&session).await;
    let source = media_source(&session, &movie).await;
    let audio: Vec<_> = source
        .media_streams
        .iter()
        .filter(|stream| stream.r#type == MediaStreamType::Audio)
        .collect();
    assert_eq!(audio.len(), 2);
    let french = audio
        .iter()
        .any(|stream| matches!(stream.language.as_deref(), Some("fre" | "fra")));
    assert!(french, "no French audio track: {audio:?}");
    session.logout().await.expect("logout");
}

#[tokio::test]
#[ignore = "writes to the server, see CONTRIBUTING.md"]
async fn genres_filter_accepts_pipe_delimited_values() {
    require_disposable();
    let session = login(&config()).await;
    let movie = movie(&session).await;
    let page = session
        .items(&ItemsQuery {
            recursive: true,
            genres: vec!["Drama".into(), "Comedy".into()],
            ..ItemsQuery::default()
        })
        .await
        .expect("items");
    assert!(page.items.iter().any(|item| item.id == movie.id));
    session.logout().await.expect("logout");
}
