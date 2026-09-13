//! The two background loops.
//!
//! Both follow the same shape: sample, emit one event covering everything, sleep. Emitting a
//! single aggregate event rather than one per project keeps the IPC chatter flat as the
//! project count grows, and lets the frontend re-render once per tick instead of N times.
//!
//! Each loop body is wrapped so a panic inside it kills the tick, not the loop.

use crate::monitor::{SystemUsage, Usage};
use crate::remote::{self, RemoteReport};
use crate::state::AppState;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub const METRICS_EVENT: &str = "metrics:tick";
pub const REMOTE_EVENT: &str = "remote:tick";

const SAMPLE_PERIOD: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsTick {
    pub projects: Vec<Usage>,
    pub system: SystemUsage,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTick {
    pub reports: Vec<RemoteReport>,
}

/// Samples every running project once a second.
pub fn spawn_metrics_loop(app: AppHandle) {
    tokio::spawn(async move {
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
                MetricsTick {
                    projects: monitor.sample(&roots),
                    system: monitor.system_usage(),
                }
            };

            // A closed window makes emit fail; that is normal and not worth logging.
            let _ = app.emit(METRICS_EVENT, &tick);
        }
    });
}

/// Checks every remote project on the configured interval.
pub fn spawn_health_loop(app: AppHandle) {
    tokio::spawn(async move {
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
/// Hand-rolled rather than pulling in `futures` for one function: spawning onto the current
/// runtime and joining the handles does the same job with a dependency Oracle already has.
async fn futures_join_all<F, T>(futures: impl Iterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handles: Vec<_> = futures.map(tokio::spawn).collect();

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
    tokio::spawn(async move {
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
    tokio::spawn(async move {
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
            // One project failing to start must not stop the others.
            let _ = state.runner.start(&project);
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    });
}
