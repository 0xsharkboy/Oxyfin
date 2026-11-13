//! The mpv-backed player.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::{self, JoinHandle};

use libmpv2::Mpv;

use crate::events::{self, Shared};
use crate::{MediaSource, PlayerError, PlayerEvent};

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
        if self.shared.active.load(Ordering::SeqCst) {
            self.shared.replacing.store(true, Ordering::SeqCst);
        }
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
        self.command("loadfile", &[&source.location, "replace"])
    }

    /// Stops playback and unloads the current media.
    pub fn stop(&self) -> Result<(), PlayerError> {
        self.command("stop", &[])
    }

    /// Fails with [`PlayerError::NoMediaLoaded`] unless a media is loaded.
    #[allow(dead_code)]
    pub(crate) fn require_loaded(&self) -> Result<(), PlayerError> {
        if self.shared.loaded.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(PlayerError::NoMediaLoaded)
        }
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
