//! Integration tests for the mpv-backed player.

#![cfg(feature = "mpv")]
#![allow(clippy::unwrap_used)]

use oxyfin_player::{Player, PlayerOptions, VideoOutput};

#[test]
fn creates_and_drops_a_headless_player() {
    let player = Player::new(PlayerOptions {
        video_output: VideoOutput::None,
    })
    .unwrap();
    drop(player);
}
