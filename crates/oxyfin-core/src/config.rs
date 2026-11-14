//! Persisted configuration: device id and saved session. Never holds a token.

use std::fs;

use serde::{Deserialize, Serialize};

use crate::{CoreError, Paths, Result};

const FILE_NAME: &str = "config.json";

/// A session remembered between launches. The token lives in the keyring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSession {
    /// Base URL of the server.
    pub server_url: String,
    /// Jellyfin user id.
    pub user_id: String,
    /// Display name of the user.
    pub user_name: String,
    /// Server id, when known.
    #[serde(default)]
    pub server_id: Option<String>,
}

/// Application configuration stored in `config_dir/config.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Stable identifier of this installation.
    pub device_id: String,
    /// The remembered session, if any.
    #[serde(default)]
    pub session: Option<SavedSession>,
    /// Last server address typed by the user.
    #[serde(default)]
    pub last_server_url: Option<String>,
    /// Whether the plain-file token fallback warning was already shown.
    #[serde(default)]
    pub file_token_warning_shown: bool,
}

fn storage<E: std::fmt::Display>(error: E) -> CoreError {
    CoreError::Storage(error.to_string())
}

impl Config {
    fn fresh() -> Self {
        Self {
            device_id: uuid::Uuid::new_v4().to_string(),
            session: None,
            last_server_url: None,
            file_token_warning_shown: false,
        }
    }

    /// Loads the configuration, creating it with a new device id when absent.
    ///
    /// An unreadable file is moved to `config.json.bad` and replaced.
    pub fn load_or_create(paths: &Paths) -> Result<Self> {
        let file = paths.config_dir.join(FILE_NAME);
        match fs::read(&file) {
            Ok(bytes) => match serde_json::from_slice::<Self>(&bytes) {
                Ok(config) if !config.device_id.is_empty() => return Ok(config),
                _ => {
                    fs::rename(&file, paths.config_dir.join("config.json.bad")).map_err(storage)?
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage(error)),
        }
        let config = Self::fresh();
        config.save(paths)?;
        Ok(config)
    }

    /// Writes the configuration atomically.
    pub fn save(&self, paths: &Paths) -> Result<()> {
        fs::create_dir_all(&paths.config_dir).map_err(storage)?;
        let json = serde_json::to_vec_pretty(self).map_err(storage)?;
        let tmp = paths.config_dir.join("config.json.tmp");
        fs::write(&tmp, json).map_err(storage)?;
        fs::rename(&tmp, paths.config_dir.join(FILE_NAME)).map_err(storage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        (dir, paths)
    }

    #[test]
    fn creates_a_device_id_once() {
        let (_dir, paths) = temp();
        let first = Config::load_or_create(&paths).unwrap();
        let second = Config::load_or_create(&paths).unwrap();
        assert!(!first.device_id.is_empty());
        assert_eq!(first.device_id, second.device_id);
    }

    #[test]
    fn save_round_trips() {
        let (_dir, paths) = temp();
        let mut config = Config::load_or_create(&paths).unwrap();
        config.last_server_url = Some("https://jf.example".into());
        config.file_token_warning_shown = true;
        config.session = Some(SavedSession {
            server_url: "https://jf.example".into(),
            user_id: "u1".into(),
            user_name: "Ana".into(),
            server_id: Some("s1".into()),
        });
        config.save(&paths).unwrap();
        assert_eq!(Config::load_or_create(&paths).unwrap(), config);
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let (_dir, paths) = temp();
        fs::create_dir_all(&paths.config_dir).unwrap();
        fs::write(paths.config_dir.join("config.json"), "{\"device").unwrap();
        let config = Config::load_or_create(&paths).unwrap();
        assert!(paths.config_dir.join("config.json.bad").exists());
        assert!(config.session.is_none());
        assert!(!config.device_id.is_empty());
    }

    #[test]
    fn older_files_without_new_fields_still_load() {
        let (_dir, paths) = temp();
        fs::create_dir_all(&paths.config_dir).unwrap();
        fs::write(
            paths.config_dir.join("config.json"),
            r#"{"device_id":"abc"}"#,
        )
        .unwrap();
        let config = Config::load_or_create(&paths).unwrap();
        assert_eq!(config.device_id, "abc");
        assert!(!config.file_token_warning_shown);
    }

    #[test]
    fn config_never_contains_a_token() {
        let (_dir, paths) = temp();
        let mut config = Config::load_or_create(&paths).unwrap();
        config.session = Some(SavedSession {
            server_url: "https://jf.example".into(),
            user_id: "u1".into(),
            user_name: "Ana".into(),
            server_id: None,
        });
        let value = serde_json::to_value(&config).unwrap();
        for object in [&value, &value["session"]] {
            assert!(object.as_object().unwrap().keys().all(|key| key != "token"));
        }
    }
}
