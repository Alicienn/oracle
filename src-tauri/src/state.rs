//! The shared state every command reads from.
//!
//! One value, held by Tauri, containing the config and the three subsystems that need to
//! outlive a single call. Locks are deliberately fine-grained and never held across an
//! `await`, so a slow health check cannot block the UI from reading the project list.

use crate::config::{self, model::Config};
use crate::monitor::Monitor;
use crate::remote::RemoteStatus;
use crate::runner::ProcessManager;
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub struct AppState {
    pub config: RwLock<Config>,
    pub runner: Arc<ProcessManager>,
    pub monitor: Mutex<Monitor>,
    /// Last known status per remote project.
    pub remote: RwLock<HashMap<String, RemoteStatus>>,
    /// Cached icon file per project, for the ones that serve one.
    pub favicons: RwLock<HashMap<String, PathBuf>>,
    pub http: reqwest::Client,
    /// Startup notices worth showing once, such as a recovered config file.
    pub warnings: RwLock<Vec<String>>,
}

impl AppState {
    pub fn new(loaded: config::Loaded, runner: Arc<ProcessManager>) -> Self {
        let mut warnings = Vec::new();

        if let Some(path) = &loaded.recovered_from {
            warnings.push(format!(
                "The configuration file could not be read and was moved to {}. Oracle started with defaults.",
                path.display()
            ));
        }

        Self {
            config: RwLock::new(loaded.config),
            runner,
            monitor: Mutex::new(Monitor::new()),
            remote: RwLock::new(HashMap::new()),
            favicons: RwLock::new(HashMap::new()),
            http: crate::remote::client(),
            warnings: RwLock::new(warnings),
        }
    }

    /// Persists the current config.
    ///
    /// Called after every mutation rather than on a timer: the config is small, writes are
    /// atomic, and losing a project because the app closed before a flush would be far
    /// worse than the cost of the write.
    pub fn persist(&self) -> crate::error::Result<()> {
        let config = self.config.read();
        config::save(&config)
    }

    /// The `(project id, root pid)` pairs the monitor needs this tick.
    pub fn running_roots(&self) -> Vec<(String, u32)> {
        self.runner
            .running()
            .into_iter()
            .map(|info| (info.project_id, info.pid))
            .collect()
    }

    /// The icon downloaded for a project, if one was found.
    pub fn favicon(&self, project_id: &str) -> Option<PathBuf> {
        self.favicons.read().get(project_id).cloned()
    }

    pub fn take_warnings(&self) -> Vec<String> {
        std::mem::take(&mut *self.warnings.write())
    }
}
