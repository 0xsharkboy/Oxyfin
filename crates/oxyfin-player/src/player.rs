//! The mpv-backed player.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::{self, JoinHandle};

use libmpv2::Mpv;

use crate::error::describe;
use crate::events::{self, Shared};
use crate::tracks;
use crate::{
    HdrFormat, HdrInfo, MediaSource, PlaybackStats, PlayerError, PlayerEvent, SeekTarget,
    SubtitleAppearance, Track, TrackId, TrackKind,
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
    /// Volume to start at, in percent; mpv starts at full volume without one.
    pub volume: Option<f64>,
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

/// The version of the libmpv in use, such as `mpv 0.40.0`, read from an instance that never
/// plays anything.
pub fn mpv_version() -> Result<String, PlayerError> {
    let mpv = Mpv::with_initializer(|init| {
        init.set_option("vo", "null")?;
        init.set_option("ao", "null")?;
        Ok(())
    })
    .map_err(|e| PlayerError::Create(describe(&e)))?;
    mpv.get_property::<String>("mpv-version")
        .map_err(|e| PlayerError::Property {
            name: "mpv-version".to_owned(),
            message: describe(&e),
        })
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
            // Otherwise rendering blocks the interface thread until each frame's display time.
            init.set_option("video-timing-offset", "0")?;
            if let Some(volume) = options.volume {
                init.set_option("volume", volume.clamp(0.0, 100.0))?;
            }
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

    /// Delays the subtitles by `millis` milliseconds, earlier when negative.
    ///
    /// mpv keeps the delay when the next media loads.
    pub fn set_subtitle_delay(&self, millis: i64) -> Result<(), PlayerError> {
        self.set_property("sub-delay", seconds(millis))
    }

    /// The subtitle delay, in milliseconds.
    pub fn subtitle_delay(&self) -> Result<i64, PlayerError> {
        self.get_millis("sub-delay")
    }

    /// Delays the audio by `millis` milliseconds, earlier when negative.
    ///
    /// mpv keeps the delay when the next media loads.
    pub fn set_audio_delay(&self, millis: i64) -> Result<(), PlayerError> {
        self.set_property("audio-delay", seconds(millis))
    }

    /// The audio delay, in milliseconds.
    pub fn audio_delay(&self) -> Result<i64, PlayerError> {
        self.get_millis("audio-delay")
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

    /// Removes the subtitle track `track`, such as an external file to add
    /// again once it changed on the server.
    pub fn remove_subtitle(&self, track: TrackId) -> Result<(), PlayerError> {
        self.require_loaded()?;
        self.command("sub-remove", &[&track.0.to_string()])
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

    /// Reads the decoding, rendering and network figures of the playing
    /// media. Never fails: what mpv cannot report is `None`.
    pub fn stats(&self) -> PlaybackStats {
        let size = |prefix: &str| match (
            self.int(&format!("{prefix}/w")),
            self.int(&format!("{prefix}/h")),
        ) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
            _ => None,
        };
        // The osd margins are unreliable with the render API: fit the video by its display aspect.
        let output = match (
            self.int("video-params/dw").zip(self.int("video-params/dh")),
            size("osd-dimensions"),
        ) {
            (Some(video), Some(area)) if video.0 > 0 && video.1 > 0 => Some(fit(video, area)),
            _ => None,
        };
        let video_codec = self.text("current-tracks/video/codec").map(|codec| {
            match self.text("current-tracks/video/codec-profile") {
                Some(profile) => format!("{codec} {profile}"),
                None => codec,
            }
        });
        let audio_output = self.text("current-ao").map(|ao| {
            let rate = self
                .int("audio-out-params/samplerate")
                .filter(|rate| *rate > 0)
                .map(|rate| format!("{} kHz", trim_number(rate as f64 / 1000.0, 1)));
            let layout = self.text("audio-out-params/hr-channels");
            let details = [rate, layout].into_iter().flatten().collect::<Vec<_>>();
            if details.is_empty() {
                output_name(&ao)
            } else {
                format!("{} · {}", output_name(&ao), details.join(" "))
            }
        });
        PlaybackStats {
            file_format: self.text("file-format"),
            video_codec,
            hwdec: self.text("hwdec-current").filter(|hwdec| hwdec != "no"),
            source_size: size("video-params"),
            output_size: output.or_else(|| size("video-out-params")),
            fps: self
                .float("estimated-vf-fps")
                .filter(|fps| *fps > 0.0 && fps.is_finite()),
            dropped_decoder: self.count("decoder-frame-drop-count"),
            dropped_output: self.count("frame-drop-count"),
            hdr: self.hdr(),
            audio_codec: self.text("current-tracks/audio/codec"),
            audio_channels: self.text("audio-params/hr-channels"),
            audio_output,
            video_bitrate: self.bitrate("video-bitrate"),
            audio_bitrate: self.bitrate("audio-bitrate"),
            cache_seconds: self
                .float("demuxer-cache-duration")
                .filter(|seconds| *seconds >= 0.0 && seconds.is_finite()),
            cache_speed: self.count("cache-speed"),
        }
    }

    fn hdr(&self) -> Option<HdrInfo> {
        let format = match self.text("video-params/gamma")?.as_str() {
            "pq" => HdrFormat::Hdr10,
            "hlg" => HdrFormat::Hlg,
            _ => return None,
        };
        Some(HdrInfo {
            format,
            tone_mapped: tone_mapped(
                self.text("video-target-params/gamma").as_deref(),
                self.text("target-trc").as_deref(),
            ),
            curve: self.text("tone-mapping").map(|curve| tone_curve(&curve)),
        })
    }

    fn text(&self, name: &str) -> Option<String> {
        self.mpv
            .get_property::<String>(name)
            .ok()
            .filter(|value| !value.is_empty())
    }

    fn int(&self, name: &str) -> Option<u32> {
        self.mpv
            .get_property::<i64>(name)
            .ok()
            .and_then(|value| u32::try_from(value).ok())
    }

    fn count(&self, name: &str) -> Option<u64> {
        self.mpv
            .get_property::<i64>(name)
            .ok()
            .and_then(|value| u64::try_from(value).ok())
    }

    fn float(&self, name: &str) -> Option<f64> {
        self.mpv.get_property::<f64>(name).ok()
    }

    fn bitrate(&self, name: &str) -> Option<u64> {
        self.float(name)
            .filter(|bits| *bits >= 1.0 && bits.is_finite())
            .map(|bits| bits.round() as u64)
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

    fn get_millis(&self, name: &str) -> Result<i64, PlayerError> {
        self.mpv
            .get_property::<f64>(name)
            .map(|secs| (secs * 1000.0).round() as i64)
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

fn seconds(millis: i64) -> f64 {
    millis as f64 / 1000.0
}

fn track_value(track: Option<TrackId>) -> String {
    track.map_or_else(|| "no".to_owned(), |id| id.0.to_string())
}

/// The size of a video of display size `video` scaled to fit in `area`, keeping its aspect.
fn fit((width, height): (u32, u32), (area_width, area_height): (u32, u32)) -> (u32, u32) {
    let scale =
        (f64::from(area_width) / f64::from(width)).min(f64::from(area_height) / f64::from(height));
    (
        (f64::from(width) * scale).round() as u32,
        (f64::from(height) * scale).round() as u32,
    )
}

/// Whether HDR video is tone mapped, from the transfer the output targets as
/// mpv reports it, or else from the `target-trc` option: only a PQ or HLG
/// output shows HDR as it is.
fn tone_mapped(target_gamma: Option<&str>, target_trc: Option<&str>) -> bool {
    // The render API reports no target parameters, and with `target-trc=auto` draws SDR.
    let target = target_gamma.or(target_trc.filter(|trc| *trc != "auto"));
    !matches!(target, Some("pq" | "hlg"))
}

/// The tone mapping curve mpv uses for its `tone-mapping` option.
fn tone_curve(option: &str) -> String {
    // The render API draws with the gpu renderer, where `auto` is bt.2390 (mpv manual).
    match option {
        "auto" => "bt.2390",
        curve => curve,
    }
    .to_owned()
}

/// Display name of an mpv audio output driver.
fn output_name(ao: &str) -> String {
    match ao {
        "pipewire" => "PipeWire",
        "pulse" => "PulseAudio",
        "alsa" => "ALSA",
        "jack" => "JACK",
        "coreaudio" => "Core Audio",
        "wasapi" => "WASAPI",
        other => other,
    }
    .to_owned()
}

/// `value` with at most `decimals` decimals and no trailing zero.
fn trim_number(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless() -> Player {
        let options = PlayerOptions {
            video_output: VideoOutput::None,
            ..PlayerOptions::default()
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
    fn frames_are_handed_to_the_renderer_at_their_display_time() {
        assert_eq!(number(&headless(), "video-timing-offset"), 0.0);
    }

    #[test]
    fn the_mpv_version_names_the_library() {
        let version = mpv_version().unwrap();
        assert!(version.starts_with("mpv "), "{version}");
        assert!(version.chars().any(|c| c.is_ascii_digit()), "{version}");
    }

    #[test]
    fn the_player_starts_at_the_given_volume() {
        let options = PlayerOptions {
            video_output: VideoOutput::None,
            volume: Some(40.0),
        };
        let player = Player::new(options, |_| {}).unwrap();
        assert_eq!(number(&player, "volume"), 40.0);
        assert_eq!(number(&headless(), "volume"), 100.0);
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
    fn audio_outputs_have_display_names() {
        assert_eq!(output_name("pipewire"), "PipeWire");
        assert_eq!(output_name("sdl"), "sdl");
        assert_eq!(trim_number(48.0, 1), "48");
        assert_eq!(trim_number(44.1, 1), "44.1");
        assert_eq!(fit((1920, 1080), (1280, 720)), (1280, 720));
        assert_eq!(fit((1920, 1036), (1280, 720)), (1280, 691));
        assert_eq!(fit((1440, 1080), (1280, 720)), (960, 720));
    }

    #[test]
    fn hdr_is_tone_mapped_unless_the_output_is_hdr() {
        // The render API reports no target parameters and leaves the target transfer on auto.
        assert!(tone_mapped(None, Some("auto")));
        assert!(tone_mapped(None, None));
        assert!(tone_mapped(Some("bt.1886"), Some("auto")));
        assert!(!tone_mapped(Some("pq"), Some("auto")));
        assert!(!tone_mapped(None, Some("pq")));
        assert!(!tone_mapped(None, Some("hlg")));
    }

    #[test]
    fn the_tone_mapping_curve_is_the_one_mpv_picks() {
        assert_eq!(tone_curve("auto"), "bt.2390");
        assert_eq!(tone_curve("hable"), "hable");
        let player = headless();
        assert_eq!(
            player.text("tone-mapping").map(|curve| tone_curve(&curve)),
            Some("bt.2390".to_owned()),
            "the default curve"
        );
    }

    #[test]
    fn stats_without_media_are_empty() {
        let stats = headless().stats();
        assert_eq!(stats.source_size, None);
        assert_eq!(stats.video_codec, None);
        assert_eq!(stats.hdr, None);
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
