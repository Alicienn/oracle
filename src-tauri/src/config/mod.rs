//! Loading and saving the config file.
//!
//! Two guarantees matter here. First, a write either fully lands or does not happen at all —
//! a crash mid-save must never leave a truncated config. Second, a config Oracle cannot
//! parse is never silently discarded: it is moved aside with a timestamp so the user can
//! recover it by hand, and the fact is surfaced in the UI.

pub mod model;

use crate::error::{OracleError, Result};
use model::Config;
use std::fs;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "config.json";

/// Outcome of a load, so the caller can tell the user what happened.
#[derive(Debug)]
pub struct Loaded {
    pub config: Config,
    /// Set when the previous file could not be parsed and was moved aside.
    pub recovered_from: Option<PathBuf>,
    /// Set when no config existed yet — a first run.
    pub is_first_run: bool,
}

/// `%APPDATA%\com.alicien.oracle` on Windows, the platform equivalent elsewhere.
///
/// Named after the bundle identifier rather than "Oracle" so that it is exactly what Tauri
/// resolves `$APPCONFIG` to. That lets the asset protocol scope be this one directory
/// instead of a wildcard over the user's home, which is what serving user-chosen icons
/// would otherwise require.
pub fn data_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.alicien.oracle")
}

pub fn config_path() -> PathBuf {
    data_dir().join(FILE_NAME)
}

/// Where user-supplied project icons are copied to, so the config never points at a file
/// the user might later move or delete.
pub fn icons_dir() -> PathBuf {
    data_dir().join("icons")
}

pub fn load() -> Loaded {
    let path = config_path();

    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Loaded {
                config: Config::default(),
                recovered_from: None,
                is_first_run: true,
            };
        }
        Err(_) => {
            // Unreadable for some other reason (permissions, a locked file). Start clean
            // rather than refusing to launch, but do not touch what is on disk.
            return Loaded {
                config: Config::default(),
                recovered_from: None,
                is_first_run: false,
            };
        }
    };

    match serde_json::from_str::<Config>(&raw) {
        Ok(mut config) => {
            migrate(&mut config);
            config.normalise_order();
            Loaded {
                config,
                recovered_from: None,
                is_first_run: false,
            }
        }
        Err(_) => {
            let quarantined = quarantine(&path);
            Loaded {
                config: Config::default(),
                recovered_from: quarantined,
                is_first_run: false,
            }
        }
    }
}

/// Writes to a sibling temp file and renames over the target, so readers never observe a
/// partially written config.
pub fn save(config: &Config) -> Result<()> {
    let path = config_path();
    let dir = path.parent().unwrap_or_else(|| Path::new("."));

    fs::create_dir_all(dir).map_err(|source| OracleError::Io {
        path: dir.to_path_buf(),
        source,
    })?;

    let json = serde_json::to_string_pretty(config).map_err(OracleError::ConfigParse)?;

    let temp = path.with_extension("json.tmp");
    fs::write(&temp, json).map_err(OracleError::Config)?;

    // Windows rejects a rename onto an existing file, so clear the way first. The window
    // between these two calls is the one case where a crash loses the file; the temp file
    // still holds the new content, which is why it is not deleted on failure.
    if path.exists() {
        let _ = fs::remove_file(&path);
    }
    fs::rename(&temp, &path).map_err(OracleError::Config)?;

    Ok(())
}

/// Moves an unparseable config out of the way, returning where it went.
fn quarantine(path: &Path) -> Option<PathBuf> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let target = path.with_file_name(format!("config.corrupt-{stamp}.json"));
    fs::rename(path, &target).ok().map(|_| target)
}

/// Applies any transformation needed to bring an older document up to `CONFIG_VERSION`.
/// Currently a no-op beyond stamping the version, but the hook exists so the first real
/// migration does not have to invent the mechanism.
fn migrate(config: &mut Config) {
    if config.version < model::CONFIG_VERSION {
        config.version = model::CONFIG_VERSION;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Project, CONFIG_VERSION};

    #[test]
    fn migration_stamps_the_current_version() {
        let mut config = Config {
            version: 0,
            ..Config::default()
        };
        migrate(&mut config);
        assert_eq!(config.version, CONFIG_VERSION);
    }

    #[test]
    fn quarantine_moves_the_file_aside() {
        let dir = std::env::temp_dir().join(format!("oracle-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        fs::write(&path, "{ not json").unwrap();

        let moved = quarantine(&path).expect("file should have been moved");

        assert!(!path.exists(), "the corrupt file should no longer be in place");
        assert!(moved.exists(), "the quarantined copy should exist");
        assert!(moved
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("config.corrupt-"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_config_survives_a_serialisation_round_trip() {
        let mut config = Config::default();
        let mut project = Project::new("Oracle");
        project.tags = vec!["tool".into()];
        config.projects.push(project);

        let json = serde_json::to_string_pretty(&config).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(back.projects.len(), 1);
        assert_eq!(back.projects[0].name, "Oracle");
        assert_eq!(back.projects[0].tags, vec!["tool".to_string()]);
    }
}
