//! The player seen by the controller, and the channel carrying its events.

use oxyfin_player::{MediaSource, PlayerError, PlayerEvent, SeekTarget, TrackId};
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
    /// Selects an audio track, or none.
    fn select_audio(&self, track: Option<TrackId>) -> Result<(), PlayerError>;
    /// Selects a subtitle track, or none.
    fn select_subtitle(&self, track: Option<TrackId>) -> Result<(), PlayerError>;
    /// Adds an external subtitle file and returns its subtitle track id.
    fn add_external_subtitle(
        &self,
        url: &str,
        title: Option<&str>,
        language: Option<&str>,
    ) -> Result<TrackId, PlayerError>;
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
    fn select_audio(&self, track: Option<TrackId>) -> Result<(), PlayerError> {
        oxyfin_player::Player::select_audio(self, track)
    }
    fn select_subtitle(&self, track: Option<TrackId>) -> Result<(), PlayerError> {
        oxyfin_player::Player::select_subtitle(self, track)
    }
    fn add_external_subtitle(
        &self,
        url: &str,
        title: Option<&str>,
        language: Option<&str>,
    ) -> Result<TrackId, PlayerError> {
        oxyfin_player::Player::add_external_subtitle(self, url, title, language)
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
