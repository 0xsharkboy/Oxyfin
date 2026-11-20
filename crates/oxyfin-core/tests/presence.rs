//! What the Discord activity shows for the item playing.

use std::time::Duration;

use oxyfin_core::{PlaybackTitle, Presence, presence_for};

const NOW: i64 = 1_800_000_000;

fn episode() -> PlaybackTitle {
    PlaybackTitle {
        title: "The Pact".into(),
        series_name: Some("Les Veilleurs".into()),
        episode_label: Some("S02E05".into()),
        year: Some(2024),
        artwork: None,
    }
}

fn movie(year: Option<i32>) -> PlaybackTitle {
    PlaybackTitle {
        title: "Night Train".into(),
        series_name: None,
        episode_label: None,
        year,
        artwork: None,
    }
}

fn serialized(presence: &Presence) -> String {
    serde_json::to_string(presence).unwrap_or_else(|error| panic!("cannot serialize: {error}"))
}

#[test]
fn presence_episode_fields() {
    let presence = presence_for(
        &episode(),
        Duration::from_secs(600),
        Some(Duration::from_secs(2400)),
        false,
        None,
        NOW,
    );
    assert_eq!(presence.details, "Les Veilleurs");
    assert_eq!(presence.state, "S02E05 · The Pact");
    assert_eq!(presence.start, Some(NOW - 600));
    assert_eq!(presence.end, Some(NOW - 600 + 2400));
    let (start, end) = (presence.start.unwrap(), presence.end.unwrap());
    assert_eq!(end - start, 2400);
}

#[test]
fn presence_movie_shows_its_year() {
    let presence = presence_for(&movie(Some(2019)), Duration::ZERO, None, false, None, NOW);
    assert_eq!(presence.details, "Night Train");
    assert_eq!(presence.state, "2019");
    assert_eq!(presence.start, Some(NOW));
    assert_eq!(presence.end, None);

    let presence = presence_for(&movie(None), Duration::ZERO, None, false, None, NOW);
    assert_eq!(presence.state, "");
}

#[test]
fn presence_paused_has_no_timestamps() {
    let presence = presence_for(
        &episode(),
        Duration::from_secs(600),
        Some(Duration::from_secs(2400)),
        true,
        None,
        NOW,
    );
    assert!(presence.state.ends_with(" · Paused"), "{}", presence.state);
    assert_eq!(presence.start, None);
    assert_eq!(presence.end, None);

    let presence = presence_for(&movie(None), Duration::ZERO, None, true, None, NOW);
    assert_eq!(presence.state, "Paused");
}

#[test]
fn presence_never_contains_server() {
    let poster = "https://image.tmdb.org/t/p/w500/abc.jpg".to_owned();
    let presence = presence_for(
        &episode(),
        Duration::from_secs(1),
        Some(Duration::from_secs(2)),
        false,
        Some(poster.clone()),
        NOW,
    );
    assert_eq!(presence.large_image, poster);
    let json = serialized(&presence).replace(&poster, "");
    assert!(!json.contains("http"), "{json}");

    let presence = presence_for(
        &episode(),
        Duration::from_secs(1),
        None,
        false,
        Some("https://jellyfin.example.org/Items/abc/Images/Primary".into()),
        NOW,
    );
    assert!(!serialized(&presence).contains("http"));
    assert_eq!(presence.large_image, "oxyfin");
}

#[test]
fn presence_fallback_image() {
    let presence = presence_for(&episode(), Duration::ZERO, None, false, None, NOW);
    assert_eq!(presence.large_image, "oxyfin");
    assert_eq!(presence.large_text, "Oxyfin");
}

#[test]
fn presence_long_texts_are_cut_to_what_discord_accepts() {
    let mut title = movie(None);
    title.title = "é".repeat(300);
    let presence = presence_for(&title, Duration::ZERO, None, false, None, NOW);
    assert_eq!(presence.details.chars().count(), 128);
    assert!(presence.details.ends_with('…'));
}
