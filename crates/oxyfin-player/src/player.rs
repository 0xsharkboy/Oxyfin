//! The mpv-backed player.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::{self, JoinHandle};

use libmpv2::Mpv;

use crate::error::describe;
use crate::events::{self, Shared};
use crate::tracks;
use crate::{
    MediaSource, PlayerError, PlayerEvent, SeekTarget, SubtitleAppearance, Track, TrackId,
    TrackKind,
};

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
    video_output: VideoOutput,
    shared: Arc<Shared>,
    event_thread: Option<JoinHandle<()>>,
}

impl Player {
    /// Creates a player with the given options, sending its events to
    /// `on_event`.
    ///
    /// `on_event` runs on the event thread, so it must return quickly. It must
    /// not panic, which would stop event delivery. Dropping the [`Player`] from
    /// it is not supported: the drop then skips joining the event thread, which
    /// exits on its own once the callback returns.
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
        .map_err(|e| PlayerError::Create(describe(&e)))?;
        events::observe(&mpv).map_err(|e| PlayerError::Create(describe(&e)))?;

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
            video_output: options.video_output,
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
                message: describe(&e),
            })?;
        // A renderer may have pinned a video track id for the previous media.
        self.set_property("vid", "auto")?;
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

    /// Sets the playback speed; mpv rejects values outside `0.01..=100.0`.
    pub fn set_speed(&self, speed: f64) -> Result<(), PlayerError> {
        self.set_property("speed", speed)
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

    /// Selects the audio track `track` (an id of an audio track), or disables
    /// audio with `None`.
    pub fn select_audio(&self, track: Option<TrackId>) -> Result<(), PlayerError> {
        self.require_loaded()?;
        self.set_property("aid", track_value(track))
    }

    /// Selects the subtitle track `track` (an id of a subtitle track), or
    /// disables subtitles with `None`.
    pub fn select_subtitle(&self, track: Option<TrackId>) -> Result<(), PlayerError> {
        self.require_loaded()?;
        self.set_property("sid", track_value(track))
    }

    /// Adds the subtitle file at `url` (an absolute path or a full URL, matched
    /// against the `external-filename` of mpv) as an external track and returns
    /// the subtitle id of the new track.
    ///
    /// mpv may select it when no subtitle is selected yet.
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
            .filter(|track| {
                track.kind == TrackKind::Subtitle && track.external_url.as_deref() == Some(url)
            })
            .map(|track| track.id)
            .max_by_key(|id| id.0)
            .ok_or_else(|| PlayerError::Command {
                name: "sub-add".to_owned(),
                message: "the added subtitle is not in the track list".to_owned(),
            })
    }

    /// Adds an external subtitle file like [`Player::add_external_subtitle`],
    /// without waiting for it: mpv fetches it in the background and lists it
    /// once loaded, with `url` as its external file name, so a slow server
    /// never holds the caller.
    pub fn add_external_subtitle_later(
        &self,
        url: &str,
        title: Option<&str>,
        language: Option<&str>,
    ) -> Result<(), PlayerError> {
        self.require_loaded()?;
        let args = [
            "sub-add",
            url,
            "auto",
            title.unwrap_or(""),
            language.unwrap_or(""),
        ];
        self.mpv
            .command("async", &args)
            .map_err(|e| PlayerError::Command {
                name: "sub-add".to_owned(),
                message: describe(&e),
            })
    }

    /// Sets how subtitles look, for the current and later media.
    ///
    /// Every option is tried; the first failure is returned. mpv older than
    /// 0.39 has no border style, so that option is skipped there and the box
    /// may not show.
    pub fn set_subtitle_appearance(
        &self,
        appearance: &SubtitleAppearance,
    ) -> Result<(), PlayerError> {
        let [red, green, blue] = appearance.color;
        let (style, back) = if appearance.background_box {
            ("background-box", BOX_COLOR)
        } else {
            ("outline-and-shadow", NO_COLOR)
        };
        let border = match self.mpv.set_property("sub-border-style", style) {
            Err(libmpv2::Error::Raw(code)) if is_not_found(code) => Ok(()),
            other => other.map_err(|e| PlayerError::Property {
                name: "sub-border-style".to_owned(),
                message: describe(&e),
            }),
        };
        let position = if appearance.position.is_nan() {
            100.0
        } else {
            appearance.position.clamp(0.0, 150.0)
        };
        [
            self.set_property("sub-scale", appearance.scale.to_string()),
            self.set_property("sub-color", format!("#{red:02X}{green:02X}{blue:02X}")),
            border,
            self.set_property("sub-back-color", back),
            self.set_property("sub-pos", position.to_string()),
        ]
        .into_iter()
        .collect::<Result<Vec<()>, _>>()
        .map(|_| ())
    }

    /// Returns a one-line summary of decoding and rendering counters
    /// (hardware decoder, video size, dropped and delayed frames, fps).
    ///
    /// Meant for diagnostics only: the format is not stable. A value mpv
    /// cannot report is shown as `-`.
    pub fn stats(&self) -> String {
        fn show<T: std::fmt::Display>(value: Option<T>) -> String {
            value.map_or_else(|| "-".to_owned(), |v| v.to_string())
        }
        let hwdec = self
            .mpv
            .get_property::<String>("hwdec-current")
            .ok()
            .filter(|h| !h.is_empty());
        let video = match (
            self.mpv.get_property::<i64>("video-params/w").ok(),
            self.mpv.get_property::<i64>("video-params/h").ok(),
        ) {
            (Some(w), Some(h)) => Some(format!("{w}x{h}")),
            _ => None,
        };
        let dropped = self.mpv.get_property::<i64>("frame-drop-count").ok();
        let delayed = self.mpv.get_property::<i64>("vo-delayed-frame-count").ok();
        let fps = self
            .mpv
            .get_property::<f64>("estimated-vf-fps")
            .ok()
            .map(|f| format!("{f:.2}"));
        format!(
            "hwdec={} video={} drop={} delayed={} fps={}",
            show(hwdec),
            show(video),
            show(dropped),
            show(delayed),
            show(fps),
        )
    }

    /// Fails with [`PlayerError::NoMediaLoaded`] unless a media is loaded.
    pub(crate) fn require_loaded(&self) -> Result<(), PlayerError> {
        if self.is_loaded() {
            Ok(())
        } else {
            Err(PlayerError::NoMediaLoaded)
        }
    }

    pub(crate) fn is_loaded(&self) -> bool {
        self.shared.loaded.load(Ordering::SeqCst)
    }

    /// Selects the default (or first) video track when the loaded media has
    /// video but none is selected, then seeks exactly to the current position
    /// so that video does not wait for the decoder to catch up with audio.
    ///
    /// The seek is skipped at the end of the media and before the event thread
    /// has seen the file load (video then recovers more slowly), and its
    /// failure is ignored.
    pub(crate) fn restore_video(&self) -> Result<(), PlayerError> {
        // Setting `vid=auto` during playback does not select a track again.
        let tracks = tracks::read(&self.mpv)?;
        let video: Vec<&Track> = tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Video)
            .collect();
        if video.iter().any(|track| track.is_selected) {
            return Ok(());
        }
        match video
            .iter()
            .find(|track| track.is_default)
            .or(video.first())
        {
            Some(track) => {
                // Read before `vid` is set: selecting the track resets it.
                let ended = self
                    .mpv
                    .get_property::<bool>(events::EOF_REACHED)
                    .unwrap_or(false);
                self.set_property("vid", track_value(Some(track.id)))?;
                if self.is_loaded() && !ended {
                    // Best effort: a failed refresh only means a slower recovery.
                    let _ = self.command("seek", &["0", "relative+exact"]);
                }
                Ok(())
            }
            None => Ok(()),
        }
    }

    pub(crate) fn video_output(&self) -> VideoOutput {
        self.video_output
    }

    pub(crate) fn mpv(&self) -> &Arc<Mpv> {
        &self.mpv
    }

    fn set_property(&self, name: &str, value: impl libmpv2::SetData) -> Result<(), PlayerError> {
        self.mpv
            .set_property(name, value)
            .map_err(|e| PlayerError::Property {
                name: name.to_owned(),
                message: describe(&e),
            })
    }

    fn command(&self, name: &str, args: &[&str]) -> Result<(), PlayerError> {
        self.mpv
            .command(name, args)
            .map_err(|e| PlayerError::Command {
                name: name.to_owned(),
                message: describe(&e),
            })
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // The event thread only exits on mpv's shutdown event.
        let _ = self.mpv.command("quit", &[]);
        if let Some(event_thread) = self.event_thread.take()
            && event_thread.thread().id() != thread::current().id()
        {
            let _ = event_thread.join();
        }
    }
}

/// Dark translucent box behind subtitles, as `#AARRGGBB`.
const BOX_COLOR: &str = "#AF000000";
/// Fully transparent, as `#AARRGGBB`.
const NO_COLOR: &str = "#00000000";

fn is_not_found(code: libmpv2::MpvError) -> bool {
    use libmpv2::mpv_error;
    code == mpv_error::PropertyNotFound || code == mpv_error::OptionNotFound
}

fn track_value(track: Option<TrackId>) -> String {
    track.map_or_else(|| "no".to_owned(), |id| id.0.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless() -> Player {
        let options = PlayerOptions {
            video_output: VideoOutput::None,
        };
        Player::new(options, |_| {}).unwrap()
    }

    fn read(player: &Player, name: &str) -> Option<String> {
        player.mpv.get_property::<String>(name).ok()
    }

    fn number(player: &Player, name: &str) -> f64 {
        read(player, name).unwrap().parse().unwrap()
    }

    #[test]
    fn subtitle_appearance_sets_the_mpv_options() {
        let player = headless();
        player
            .set_subtitle_appearance(&SubtitleAppearance {
                scale: 1.25,
                color: [0xFF, 0xE0, 0x00],
                background_box: true,
                position: 90.0,
            })
            .unwrap();

        assert_eq!(number(&player, "sub-scale"), 1.25);
        assert_eq!(number(&player, "sub-pos"), 90.0);
        assert_eq!(read(&player, "sub-color").unwrap(), "#FFFFE000");
        assert_eq!(read(&player, "sub-back-color").unwrap(), BOX_COLOR);
        // Older mpv versions have no border style.
        if let Some(style) = read(&player, "sub-border-style") {
            assert_eq!(style, "background-box");
        }
    }

    #[test]
    fn default_subtitle_appearance_restores_the_outline() {
        let player = headless();
        let boxed = SubtitleAppearance {
            background_box: true,
            ..SubtitleAppearance::default()
        };
        player.set_subtitle_appearance(&boxed).unwrap();
        player
            .set_subtitle_appearance(&SubtitleAppearance::default())
            .unwrap();

        assert_eq!(number(&player, "sub-scale"), 1.0);
        assert_eq!(number(&player, "sub-pos"), 100.0);
        assert_eq!(read(&player, "sub-color").unwrap(), "#FFFFFFFF");
        assert_eq!(read(&player, "sub-back-color").unwrap(), NO_COLOR);
        if let Some(style) = read(&player, "sub-border-style") {
            assert_eq!(style, "outline-and-shadow");
        }
    }

    #[test]
    fn subtitle_position_is_kept_in_range() {
        let player = headless();
        let appearance = SubtitleAppearance {
            position: 400.0,
            ..SubtitleAppearance::default()
        };
        player.set_subtitle_appearance(&appearance).unwrap();
        assert_eq!(number(&player, "sub-pos"), 150.0);
    }
}
