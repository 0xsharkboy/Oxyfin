//! Per-user directories.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::{CoreError, Result};

/// Directories used by the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Holds `config.json`.
    pub config_dir: PathBuf,
    /// Holds persistent application data.
    pub data_dir: PathBuf,
    /// Holds disposable cached data.
    pub cache_dir: PathBuf,
}

impl Paths {
    /// Resolves the platform directories of the current user.
    pub fn from_system() -> Result<Self> {
        let dirs = ProjectDirs::from("org", "Oxyfin", "oxyfin")
            .ok_or_else(|| CoreError::Storage("no home directory found".into()))?;
        Ok(Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_dir().to_path_buf(),
            cache_dir: dirs.cache_dir().to_path_buf(),
        })
    }

    /// Places every directory under `root`, for tests and portable setups.
    pub fn under(root: &Path) -> Self {
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_splits_the_root() {
        let paths = Paths::under(Path::new("/tmp/x"));
        assert_eq!(paths.config_dir, Path::new("/tmp/x/config"));
        assert_eq!(paths.data_dir, Path::new("/tmp/x/data"));
        assert_eq!(paths.cache_dir, Path::new("/tmp/x/cache"));
    }
}
