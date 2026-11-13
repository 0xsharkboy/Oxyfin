//! The mpv-backed player.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::{self, JoinHandle};

use libmpv2::Mpv;

use crate::events::{self, Shared};
use crate::tracks;
use crate::{MediaSource, PlayerError, PlayerEvent, SeekTarget, TrackId};

/// Where mpv sends decoded video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoOutput {
    /// Frames are rendered through an embedded render context.
    #[default]
    Embedded,
    /// No video and no audio output, for headless use and tests.
    None,
}

/// Options used to create a [`Player`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerOptions {
    /// Where mpv sends decoded video.
    pub video_output: VideoOutput,
}

/// A libmpv player instance.
///
/// Events are delivered to the callback given to [`Player::new`], on a
/// dedicated thread.
pub struct Player {
    mpv: Arc<Mpv>,
    shared: Arc<Shared>,
    event_thread: Option<JoinHandle<()>>,
}

impl Player {
    /// Creates a player with the given options, sending its events to
    /// `on_event`.
    ///
    /// `on_event` runs on the event thread, so it must return quickly. It must
    /// not panic, which would stop event delivery, and it must not drop the
    /// [`Player`], which would deadlock joining that thread.
    pub fn new(
        options: PlayerOptions,
        on_event: impl Fn(PlayerEvent) + Send + 'static,
    ) -> Result<Self, PlayerError> {
        let mpv = Mpv::with_initializer(|init| {
            match options.video_output {
                VideoOutput::Embedded => init.set_option("vo", "libmpv")?,
                VideoOutput::None => {
                    init.set_option("vo", "null")?;
                    init.set_option("ao", "null")?;
                }
            }
            init.set_option("hwdec", "auto-safe")?;
            init.set_option("keep-open", "yes")?;
            init.set_option("keep-open-pause", "no")?;
            Ok(())
        })
        .map_err(|e| PlayerError::Create(e.to_string()))?;
        events::observe(&mpv).map_err(|e| PlayerError::Create(e.to_string()))?;

        let mpv = Arc::new(mpv);
        let shared = Arc::new(Shared::default());
        let event_thread = {
            let mpv = Arc::clone(&mpv);
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name("oxyfin-mpv-events".to_owned())
                .spawn(move || events::run(&mpv, &shared, &on_event))
                .map_err(|e| PlayerError::Create(e.to_string()))?
        };
        Ok(Self {
            mpv,
            shared,
            event_thread: Some(event_thread),
        })
    }

    /// Loads `source`, replacing the current media.
    ///
    /// The outcome is reported through events: [`PlayerEvent::FileLoaded`]
    /// on success, [`PlayerEvent::Unloaded`] with an error otherwise.
    pub fn load(&self, source: &MediaSource) -> Result<(), PlayerError> {
        let start = source.start.map_or_else(
            || "none".to_owned(),
            |start| format!("+{}", start.as_secs_f64()),
        );
        self.mpv
            .set_property("start", start)
            .map_err(|e| PlayerError::Property {
                name: "start".to_owned(),
                message: e.to_string(),
            })?;
        self.command("change-list", &["http-header-fields", "clr", ""])?;
        for (name, value) in &source.headers {
            let header = format!("{name}: {value}");
            self.command("change-list", &["http-header-fields", "append", &header])?;
        }
        self.shared.stopping.store(false, Ordering::SeqCst);
        self.command("loadfile", &[&source.location, "replace"])
    }

    /// Stops playback and unloads the current media.
    pub fn stop(&self) -> Result<(), PlayerError> {
        self.shared.stopping.store(true, Ordering::SeqCst);
        let result = self.command("stop", &[]);
        if result.is_err() {
            self.shared.stopping.store(false, Ordering::SeqCst);
        }
        result
    }

    /// Pauses or resumes playback.
    pub fn set_paused(&self, paused: bool) -> Result<(), PlayerError> {
        self.set_property("pause", paused)
    }

    /// Seeks to `target`. Fails with [`PlayerError::NoMediaLoaded`] when no
    /// media is loaded.
    pub fn seek(&self, target: SeekTarget) -> Result<(), PlayerError> {
        self.require_loaded()?;
        let (secs, mode) = match target {
            SeekTarget::Absolute(at) => (at.as_secs_f64(), "absolute"),
            SeekTarget::Forward(by) => (by.as_secs_f64(), "relative+exact"),
            SeekTarget::Backward(by) => (-by.as_secs_f64(), "relative+exact"),
        };
        self.command("seek", &[&format!("{secs:.3}"), mode])
    }

    /// Sets the volume, clamped to `0.0..=100.0`.
    pub fn set_volume(&self, volume: f64) -> Result<(), PlayerError> {
        let volume = if volume.is_nan() {
            0.0
        } else {
            volume.clamp(0.0, 100.0)
        };
        self.set_property("volume", volume)
    }

    /// Mutes or unmutes the audio.
    pub fn set_muted(&self, muted: bool) -> Result<(), PlayerError> {
        self.set_property("mute", muted)
    }

    /// Selects the audio track `track`, or disables audio with `None`.
    pub fn select_audio(&self, track: Option<TrackId>) -> Result<(), PlayerError> {
        self.require_loaded()?;
        self.set_property("aid", track_value(track))
    }

    /// Selects the subtitle track `track`, or disables subtitles with `None`.
    pub fn select_subtitle(&self, track: Option<TrackId>) -> Result<(), PlayerError> {
        self.require_loaded()?;
        self.set_property("sid", track_value(track))
    }

    /// Adds the subtitle file at `url` as an external track and returns its
    /// identifier. mpv may select it when no subtitle is selected yet.
    pub fn add_external_subtitle(
        &self,
        url: &str,
        title: Option<&str>,
        language: Option<&str>,
    ) -> Result<TrackId, PlayerError> {
        self.require_loaded()?;
        self.command(
            "sub-add",
            &[url, "auto", title.unwrap_or(""), language.unwrap_or("")],
        )?;
        tracks::read(&self.mpv)?
            .into_iter()
            .filter(|track| track.external_url.as_deref() == Some(url))
            .map(|track| track.id)
            .max_by_key(|id| id.0)
            .ok_or_else(|| PlayerError::Command {
                name: "sub-add".to_owned(),
                message: "the added subtitle is not in the track list".to_owned(),
            })
    }

    /// Fails with [`PlayerError::NoMediaLoaded`] unless a media is loaded.
    pub(crate) fn require_loaded(&self) -> Result<(), PlayerError> {
        if self.shared.loaded.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(PlayerError::NoMediaLoaded)
        }
    }

    fn set_property(&self, name: &str, value: impl libmpv2::SetData) -> Result<(), PlayerError> {
        self.mpv
            .set_property(name, value)
            .map_err(|e| PlayerError::Property {
                name: name.to_owned(),
                message: e.to_string(),
            })
    }

    fn command(&self, name: &str, args: &[&str]) -> Result<(), PlayerError> {
        self.mpv
            .command(name, args)
            .map_err(|e| PlayerError::Command {
                name: name.to_owned(),
                message: e.to_string(),
            })
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // The event thread only exits on mpv's shutdown event.
        let _ = self.mpv.command("quit", &[]);
        if let Some(event_thread) = self.event_thread.take() {
            let _ = event_thread.join();
        }
    }
}

fn track_value(track: Option<TrackId>) -> String {
    track.map_or_else(|| "no".to_owned(), |id| id.0.to_string())
}
