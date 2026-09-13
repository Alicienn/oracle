//! The two background loops.
//!
//! Both follow the same shape: sample, emit one event covering everything, sleep. Emitting a
//! single aggregate event rather than one per project keeps the IPC chatter flat as the
//! project count grows, and lets the frontend re-render once per tick instead of N times.
//!
//! Each loop body is wrapped so a panic inside it kills the tick, not the loop.

use crate::config::model::IconSource;
use crate::favicon;
use crate::monitor::{SystemUsage, Usage};
use crate::remote::{self, RemoteReport};
use crate::state::AppState;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub const METRICS_EVENT: &str = "metrics:tick";
pub const REMOTE_EVENT: &str = "remote:tick";
pub const ICONS_EVENT: &str = "icons:resolved";

const SAMPLE_PERIOD: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsTick {
    pub projects: Vec<Usage>,
    pub system: SystemUsage,
    /// What the web app on screen is costing, when one is open.
    pub embed: Option<EmbedUsage>,
}

/// The memory held by the webview currently shown in the central panel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedUsage {
    pub project_id: String,
    pub memory: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTick {
    pub reports: Vec<RemoteReport>,
}

/// Samples every running project once a second.
pub fn spawn_metrics_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(SAMPLE_PERIOD);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;

            let Some(state) = app.try_state::<Arc<AppState>>() else {
                // The app is shutting down.
                return;
            };

            let tick = {
                let roots = state.running_roots();
                let mut monitor = state.monitor.lock();
                let projects = monitor.sample(&roots);
                let system = monitor.system_usage();
                // Dropped before asking about the embed, which takes the same lock.
                drop(monitor);

                MetricsTick {
                    projects,
                    system,
                    embed: crate::embed::visible_memory(&app).map(|(project_id, memory)| {
                        EmbedUsage { project_id, memory }
                    }),
                }
            };

            // A closed window makes emit fail; that is normal and not worth logging.
            let _ = app.emit(METRICS_EVENT, &tick);
        }
    });
}

/// Checks every remote project on the configured interval.
pub fn spawn_health_loop(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let Some(state) = app.try_state::<Arc<AppState>>() else {
                return;
            };

            // Snapshot the work before any await, so the config lock is never held across
            // a network call.
            let (targets, interval) = {
                let config = state.config.read();
                let targets: Vec<_> = config
                    .projects
                    .iter()
                    .filter_map(|project| {
                        project
                            .remote
                            .as_ref()
                            .map(|remote| (project.id.clone(), remote.clone()))
                    })
                    .collect();
                (targets, config.settings.health_interval_secs.max(5))
            };

            if !targets.is_empty() {
                let client = state.http.clone();
                let checks = targets.into_iter().map(|(id, target)| {
                    let client = client.clone();
                    async move {
                        let status = remote::check(&client, &target).await;
                        RemoteReport {
                            project_id: id,
                            status,
                            checked_at: chrono::Utc::now().timestamp_millis(),
                        }
                    }
                });

                // All checks run concurrently: ten dead hosts should cost one timeout, not
                // ten in sequence.
                let reports = futures_join_all(checks).await;

                {
                    let mut cache = state.remote.write();
                    for report in &reports {
                        cache.insert(report.project_id.clone(), report.status.clone());
                    }
                }

                let _ = app.emit(REMOTE_EVENT, &RemoteTick { reports });
            }

            tokio::time::sleep(Duration::from_secs(interval as u64)).await;
        }
    });
}

/// Runs futures concurrently and collects the results.
///
/// Hand-rolled rather than pulling in `futures` for one function: spawning onto Tauri's
/// runtime and joining the handles does the same job with a dependency Oracle already has.
async fn futures_join_all<F, T>(futures: impl Iterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handles: Vec<_> = futures.map(tauri::async_runtime::spawn).collect();

    let mut out = Vec::with_capacity(handles.len());
    for handle in handles {
        // A panicking check drops that one result rather than taking down the loop.
        if let Ok(value) = handle.await {
            out.push(value);
        }
    }
    out
}

/// Attaches to any project whose port is already in use at startup, so a dev server the user
/// launched from a terminal shows up as running instead of stopped.
pub fn spawn_adoption_sweep(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<Arc<AppState>>() else {
            return;
        };

        let candidates: Vec<_> = {
            let config = state.config.read();
            config
                .projects
                .iter()
                .filter(|p| p.local.as_ref().and_then(|l| l.port).is_some())
                .cloned()
                .collect()
        };

        for project in candidates {
            state.runner.adopt(&project).await;
        }
    });
}

/// Starts the projects flagged to come up with Oracle.
pub fn spawn_project_autostart(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<Arc<AppState>>() else {
            return;
        };

        let wanted: Vec<_> = {
            let config = state.config.read();
            if !config.settings.autostart_projects {
                return;
            }
            config
                .projects
                .iter()
                .filter(|p| {
                    p.local
                        .as_ref()
                        .map(|l| l.autostart_with_oracle)
                        .unwrap_or(false)
                })
                .cloned()
                .collect()
        };

        for project in wanted {
            // Adoption runs first, so a project already serving is left alone.
            if state.runner.is_running(&project.id) {
                continue;
            }
            // One project failing to start must not stop the others — including a project
            // whose port a previous session left occupied, which now fails loudly here
            // rather than dying inside the child process a second later.
            let _ = state.runner.start(&project, None).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    });
}

/// Downloads the icon each remote project serves, once, in the background.
///
/// Runs off the startup path on purpose: it is several HTTP requests to hosts that may be
/// slow or gone, and nothing in the UI is waiting for it. Projects keep their initials until
/// an icon arrives, and the one event at the end tells the frontend to re-read the list.
pub fn spawn_favicon_sweep(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<Arc<AppState>>() else {
            return;
        };

        let targets: Vec<(String, String)> = {
            let config = state.config.read();
            config
                .projects
                .iter()
                // Only where Oracle would otherwise draw initials: a project with a chosen
                // icon must not have it replaced by whatever its site serves.
                .filter(|project| matches!(project.icon, IconSource::Auto))
                .filter_map(|project| {
                    project
                        .remote
                        .as_ref()
                        .map(|remote| (project.id.clone(), remote.url.clone()))
                })
                .collect()
        };

        if targets.is_empty() {
            return;
        }

        let mut found = false;
        for (project_id, url) in targets {
            found |= store_favicon(&state, &project_id, &url).await;
        }

        if found {
            let _ = app.emit(ICONS_EVENT, ());
        }
    });
}

/// Drops a project's cached icon and fetches it again.
///
/// The sweep and the per-project resolve both stop at the cache, which is the point of it.
/// Asking for the icon again is a deliberate act, so it is a deliberate path.
pub fn spawn_favicon_refresh(app: AppHandle, project_id: String, url: String) {
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<Arc<AppState>>() else {
            return;
        };

        favicon::forget(&url);
        state.favicons.write().remove(&project_id);

        // Emitted either way: the icon may legitimately have gone away, and the UI has to
        // stop showing the old one.
        store_favicon(&state, &project_id, &url).await;
        let _ = app.emit(ICONS_EVENT, ());
    });
}

/// Resolves the icon for one project, for a project added or edited after startup.
pub fn spawn_favicon_for(app: AppHandle, project_id: String, url: String) {
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<Arc<AppState>>() else {
            return;
        };

        if store_favicon(&state, &project_id, &url).await {
            let _ = app.emit(ICONS_EVENT, ());
        }
    });
}

/// Downloads and records one project's icon. True when there is something new to show.
async fn store_favicon(state: &Arc<AppState>, project_id: &str, url: &str) -> bool {
    match favicon::resolve(url).await {
        Some(path) => {
            state.favicons.write().insert(project_id.to_string(), path);
            true
        }
        None => false,
    }
}
