//! Error type of the player crate.

/// Errors returned by the player.
#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    /// mpv could not be created or initialised.
    #[error("cannot create the mpv instance: {0}")]
    Create(String),
    /// mpv rejected a command.
    #[error("mpv command `{name}` failed: {message}")]
    Command {
        /// Name of the command.
        name: String,
        /// Reason reported by mpv.
        message: String,
    },
    /// A property could not be read or written.
    #[error("mpv property `{name}` failed: {message}")]
    Property {
        /// Name of the property.
        name: String,
        /// Reason reported by mpv.
        message: String,
    },
    /// The render context failed.
    #[error("mpv render context error: {0}")]
    Render(String),
    /// The command needs a loaded media.
    #[error("no media is loaded")]
    NoMediaLoaded,
}

/// Turns a libmpv2 error into readable text, keeping the numeric mpv code.
#[cfg(feature = "mpv")]
pub(crate) fn describe(error: &libmpv2::Error) -> String {
    use libmpv2::mpv_error;

    let libmpv2::Error::Raw(code) = error else {
        return error.to_string();
    };
    let text = match *code {
        mpv_error::LoadingFailed => "loading failed",
        mpv_error::UnknownFormat => "unrecognized file format",
        mpv_error::NothingToPlay => "nothing to play",
        mpv_error::Command => "invalid command or command failed",
        mpv_error::PropertyError => "error reading or writing the property",
        mpv_error::PropertyFormat => "unsupported property format",
        mpv_error::PropertyNotFound => "property not found",
        mpv_error::PropertyUnavailable => "property currently unavailable",
        mpv_error::OptionError => "error setting the option",
        mpv_error::OptionFormat => "invalid option format",
        mpv_error::OptionNotFound => "option not found",
        mpv_error::InvalidParameter => "invalid parameter",
        mpv_error::Uninitialized => "mpv is not initialized",
        mpv_error::Unsupported => "unsupported operation",
        mpv_error::NotImplemented => "feature not implemented",
        mpv_error::NoMem => "out of memory",
        mpv_error::VoInitFailed => "video output initialization failed",
        mpv_error::AoInitFailed => "audio output initialization failed",
        mpv_error::Generic => "unspecified mpv error",
        _ => return error.to_string(),
    };
    format!("{text} (mpv error {code})")
}

#[cfg(all(test, feature = "mpv"))]
mod tests {
    use super::describe;
    use libmpv2::{Error, mpv_error};

    #[test]
    fn known_codes_read_as_text_with_the_code() {
        let text = describe(&Error::Raw(mpv_error::LoadingFailed));
        assert_eq!(text, "loading failed (mpv error -13)");
    }

    #[test]
    fn unknown_codes_fall_back_to_display() {
        assert_eq!(describe(&Error::Raw(-9999)), "Raw(-9999)");
        assert_eq!(describe(&Error::Null), "Null");
    }
}
