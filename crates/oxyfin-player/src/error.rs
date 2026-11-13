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
