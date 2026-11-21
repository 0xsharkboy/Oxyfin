//! The player seen by the controller, and the channel carrying its events.

use oxyfin_player::{
    MediaSource, PlaybackStats, PlayerError, PlayerEvent, SeekTarget, SubtitleAppearance, TrackId,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// The player commands the playback controller uses.
///
/// Implemented for `oxyfin_player::Player` with the `mpv` feature; tests use
/// a fake that records the calls.
pub trait MediaPlayer: Send + Sync + 'static {
    /// Loads a media, replacing the current one.
    fn load(&self, source: &MediaSource) -> Result<(), PlayerError>;
    /// Stops playback and unloads the media.
    fn stop(&self) -> Result<(), PlayerError>;
    /// Pauses or resumes playback.
    fn set_paused(&self, paused: bool) -> Result<(), PlayerError>;
    /// Seeks in the media.
    fn seek(&self, target: SeekTarget) -> Result<(), PlayerError>;
    /// Sets the volume, in percent.
    fn set_volume(&self, volume: f64) -> Result<(), PlayerError>;
    /// Mutes or unmutes the audio.
    fn set_muted(&self, muted: bool) -> Result<(), PlayerError>;
    /// Sets the playback speed.
    fn set_speed(&self, speed: f64) -> Result<(), PlayerError>;
    /// Delays the subtitles by this many milliseconds, earlier when negative.
    fn set_subtitle_delay(&self, millis: i64) -> Result<(), PlayerError>;
    /// Delays the audio by this many milliseconds, earlier when negative.
    fn set_audio_delay(&self, millis: i64) -> Result<(), PlayerError>;
    /// Selects an audio track, or none.
    fn select_audio(&self, track: Option<TrackId>) -> Result<(), PlayerError>;
    /// Selects a subtitle track, or none.
    fn select_subtitle(&self, track: Option<TrackId>) -> Result<(), PlayerError>;
    /// Removes a subtitle track.
    fn remove_subtitle(&self, track: TrackId) -> Result<(), PlayerError>;
    /// Adds an external subtitle file and returns its subtitle track id.
    fn add_external_subtitle(
        &self,
        url: &str,
        title: Option<&str>,
        language: Option<&str>,
    ) -> Result<TrackId, PlayerError>;
    /// Adds an external subtitle file without waiting for it; it is listed
    /// in a later track list, with its URL, once loaded.
    fn add_external_subtitle_later(
        &self,
        url: &str,
        title: Option<&str>,
        language: Option<&str>,
    ) -> Result<(), PlayerError>;
    /// Sets how subtitles look, for the current and later media.
    fn set_subtitle_appearance(&self, appearance: &SubtitleAppearance) -> Result<(), PlayerError>;
    /// Reads the decoding, rendering and network figures of the media.
    fn stats(&self) -> PlaybackStats;
}

#[cfg(feature = "mpv")]
impl MediaPlayer for oxyfin_player::Player {
    fn load(&self, source: &MediaSource) -> Result<(), PlayerError> {
        oxyfin_player::Player::load(self, source)
    }
    fn stop(&self) -> Result<(), PlayerError> {
        oxyfin_player::Player::stop(self)
    }
    fn set_paused(&self, paused: bool) -> Result<(), PlayerError> {
        oxyfin_player::Player::set_paused(self, paused)
    }
    fn seek(&self, target: SeekTarget) -> Result<(), PlayerError> {
        oxyfin_player::Player::seek(self, target)
    }
    fn set_volume(&self, volume: f64) -> Result<(), PlayerError> {
        oxyfin_player::Player::set_volume(self, volume)
    }
    fn set_muted(&self, muted: bool) -> Result<(), PlayerError> {
        oxyfin_player::Player::set_muted(self, muted)
    }
    fn set_speed(&self, speed: f64) -> Result<(), PlayerError> {
        oxyfin_player::Player::set_speed(self, speed)
    }
    fn set_subtitle_delay(&self, millis: i64) -> Result<(), PlayerError> {
        oxyfin_player::Player::set_subtitle_delay(self, millis)
    }
    fn set_audio_delay(&self, millis: i64) -> Result<(), PlayerError> {
        oxyfin_player::Player::set_audio_delay(self, millis)
    }
    fn select_audio(&self, track: Option<TrackId>) -> Result<(), PlayerError> {
        oxyfin_player::Player::select_audio(self, track)
    }
    fn select_subtitle(&self, track: Option<TrackId>) -> Result<(), PlayerError> {
        oxyfin_player::Player::select_subtitle(self, track)
    }
    fn remove_subtitle(&self, track: TrackId) -> Result<(), PlayerError> {
        oxyfin_player::Player::remove_subtitle(self, track)
    }
    fn add_external_subtitle(
        &self,
        url: &str,
        title: Option<&str>,
        language: Option<&str>,
    ) -> Result<TrackId, PlayerError> {
        oxyfin_player::Player::add_external_subtitle(self, url, title, language)
    }
    fn add_external_subtitle_later(
        &self,
        url: &str,
        title: Option<&str>,
        language: Option<&str>,
    ) -> Result<(), PlayerError> {
        oxyfin_player::Player::add_external_subtitle_later(self, url, title, language)
    }
    fn set_subtitle_appearance(&self, appearance: &SubtitleAppearance) -> Result<(), PlayerError> {
        oxyfin_player::Player::set_subtitle_appearance(self, appearance)
    }
    fn stats(&self) -> PlaybackStats {
        oxyfin_player::Player::stats(self)
    }
}

/// Sending side of the player events, given to the player's event callback.
///
/// Sending never blocks, so it is safe on the mpv event thread.
#[derive(Clone, Debug)]
pub struct PlayerEventSink(UnboundedSender<PlayerEvent>);

impl PlayerEventSink {
    /// Forwards an event to the controller; dropped when the controller is gone.
    pub fn send(&self, event: PlayerEvent) {
        let _ = self.0.send(event);
    }
}

/// Receiving side of the player events, given to the playback controller.
#[derive(Debug)]
pub struct PlayerEvents(pub(crate) UnboundedReceiver<PlayerEvent>);

/// Creates the channel that carries player events to the playback controller.
pub fn player_events() -> (PlayerEventSink, PlayerEvents) {
    let (sender, receiver) = unbounded_channel();
    (PlayerEventSink(sender), PlayerEvents(receiver))
}
