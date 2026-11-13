//! The mpv-backed player.

use libmpv2::Mpv;

use crate::PlayerError;

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
pub struct Player {
    _mpv: Mpv,
}

impl Player {
    /// Creates a player with the given options.
    pub fn new(options: PlayerOptions) -> Result<Self, PlayerError> {
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
        Ok(Self { _mpv: mpv })
    }
}
