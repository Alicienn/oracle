//! The IPC surface.
//!
//! Every handler is thin: validate, delegate to the module that owns the behaviour, persist
//! if something changed. No business logic lives here, which keeps the boundary between the
//! frontend and the backend easy to audit.

use crate::config::{self, model::*};
use crate::discovery::{self, Candidate};
use crate::error::{OracleError, Result};
use crate::monitor::SystemUsage;
use crate::remote::RemoteStatus;
use crate::runner::{logs::LogLine, ProjectStatus};
use crate::state::AppState;
use crate::{autostart, vcs};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

type St<'a> = State<'a, Arc<AppState>>;

// ---------------------------------------------------------------------------
// View types
// ---------------------------------------------------------------------------

/// A project plus everything the UI needs to render it, resolved server-side so the
/// frontend never has to reimplement fallback rules.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    #[serde(flatten)]
    pub project: Project,
    pub status: ProjectStatus,
    pub remote_status: RemoteStatus,
    pub pid: Option<u32>,
    /// The project's own accent, or the one derived from its kind.
    pub resolved_accent: String,
    pub resolved_url: Option<String>,
    pub kind_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub projects: Vec<ProjectView>,
    pub settings: Settings,
    pub system: SystemUsage,
    /// Notices to show once, such as a config file that had to be recovered.
    pub warnings: Vec<String>,
    pub git_available: bool,
    pub version: String,
}

fn view(state: &AppState, project: &Project) -> ProjectView {
    ProjectView {
        status: state.runner.status(&project.id),
        remote_status: state
            .remote
            .read()
            .get(&project.id)
            .cloned()
            .unwrap_or(RemoteStatus::Unchecked),
        pid: state.runner.pid(&project.id),
        resolved_accent: project
            .accent
            .clone()
            .unwrap_or_else(|| project.kind.accent().to_string()),
        resolved_url: project.open_url(),
        kind_label: project.kind.label().to_string(),
        project: project.clone(),
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_snapshot(state: St) -> Snapshot {
    let config = state.config.read();

    let projects = config.projects.iter().map(|p| view(&state, p)).collect();
    let system = state.monitor.lock().system_usage();

    Snapshot {
        projects,
        settings: config.settings.clone(),
        system,
        warnings: state.take_warnings(),
        git_available: vcs::is_available(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[tauri::command]
pub fn get_logs(state: St, project_id: String, since: Option<u64>) -> Vec<LogLine> {
    state.runner.logs(&project_id, since)
}

#[tauri::command]
pub fn clear_logs(state: St, project_id: String) {
    state.runner.clear_logs(&project_id);
}

#[tauri::command]
pub fn git_status(state: St, project_id: String) -> Result<Option<vcs::GitStatus>> {
    let root = {
        let config = state.config.read();
        let project = config
            .project(&project_id)
            .ok_or_else(|| OracleError::ProjectNotFound(project_id.clone()))?;

        match project.local.as_ref() {
            Some(local) => local.root.clone(),
            // A project that only exists on the VPS has no working copy to inspect.
            None => return Ok(None),
        }
    };

    vcs::status(&root)
}

// ---------------------------------------------------------------------------
// Project lifecycle
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn start_project(state: St, project_id: String) -> Result<u32> {
    let project = {
        let config = state.config.read();
        config
            .project(&project_id)
            .ok_or_else(|| OracleError::ProjectNotFound(project_id.clone()))?
            .clone()
    };

    state.runner.start(&project)
}

#[tauri::command]
pub async fn stop_project(state: St<'_>, project_id: String) -> Result<()> {
    state.runner.stop(&project_id).await
}

#[tauri::command]
pub async fn restart_project(state: St<'_>, project_id: String) -> Result<u32> {
    // Ignore a stop failure: the project may simply not be running yet.
    let _ = state.runner.stop(&project_id).await;

    let project = {
        let config = state.config.read();
        config
            .project(&project_id)
            .ok_or_else(|| OracleError::ProjectNotFound(project_id.clone()))?
            .clone()
    };

    state.runner.start(&project)
}

// ---------------------------------------------------------------------------
// Project CRUD
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn add_project(state: St, mut project: Project) -> Result<ProjectView> {
    if project.id.is_empty() {
        project.id = uuid::Uuid::new_v4().to_string();
    }

    {
        let mut config = state.config.write();
        project.order = config.projects.len() as i32;
        config.projects.push(project.clone());
    }

    state.persist()?;
    Ok(view(&state, &project))
}

#[tauri::command]
pub fn update_project(state: St, project: Project) -> Result<ProjectView> {
    {
        let mut config = state.config.write();
        let existing = config
            .project_mut(&project.id)
            .ok_or_else(|| OracleError::ProjectNotFound(project.id.clone()))?;

        // Position is owned by the reorder command, not by the edit form.
        let order = existing.order;
        *existing = project.clone();
        existing.order = order;
    }

    state.persist()?;
    Ok(view(&state, &project))
}

#[tauri::command]
pub async fn delete_project(state: St<'_>, project_id: String) -> Result<()> {
    // Never leave an orphaned process behind a deleted project.
    let _ = state.runner.stop(&project_id).await;

    {
        let mut config = state.config.write();
        config.projects.retain(|p| p.id != project_id);
        config.normalise_order();
    }

    state.monitor.lock().forget(&project_id);
    state.remote.write().remove(&project_id);
    state.persist()
}

#[tauri::command]
pub fn reorder_projects(state: St, ordered_ids: Vec<String>) -> Result<()> {
    {
        let mut config = state.config.write();
        for (index, id) in ordered_ids.iter().enumerate() {
            if let Some(project) = config.project_mut(id) {
                project.order = index as i32;
            }
        }
        config.normalise_order();
    }

    state.persist()
}

#[tauri::command]
pub fn toggle_favorite(state: St, project_id: String) -> Result<bool> {
    let favorite = {
        let mut config = state.config.write();
        let project = config
            .project_mut(&project_id)
            .ok_or_else(|| OracleError::ProjectNotFound(project_id.clone()))?;
        project.favorite = !project.favorite;
        project.favorite
    };

    state.persist()?;
    Ok(favorite)
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn scan_projects(state: St<'_>) -> Result<Vec<Candidate>> {
    let (roots, known) = {
        let config = state.config.read();
        let known: Vec<PathBuf> = config
            .projects
            .iter()
            .filter_map(|p| p.local.as_ref().map(|l| l.root.clone()))
            .collect();
        (config.settings.scan_roots.clone(), known)
    };

    // Walking the disk blocks; keep it off the async runtime's worker threads.
    tokio::task::spawn_blocking(move || discovery::scan(&roots, &known))
        .await
        .map_err(|err| OracleError::Other(format!("the scan did not finish: {err}")))
}

/// Turns accepted scan candidates into real projects.
#[tauri::command]
pub fn import_candidates(state: St, candidates: Vec<Candidate>) -> Result<Vec<ProjectView>> {
    let mut created = Vec::new();

    {
        let mut config = state.config.write();
        let mut order = config.projects.len() as i32;

        for candidate in candidates {
            let mut project = Project::new(candidate.name);
            project.kind = candidate.kind;
            project.repo = candidate.repo;
            project.order = order;

            let mut local = LocalTarget::new(candidate.root, candidate.suggested_command);
            local.port = candidate.suggested_port;
            project.local = Some(local);

            order += 1;
            config.projects.push(project.clone());
            created.push(project);
        }
    }

    state.persist()?;
    Ok(created.iter().map(|p| view(&state, p)).collect())
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn update_settings(state: St, settings: Settings) -> Result<Settings> {
    // The registry is the source of truth for autostart, so keep it in step with the
    // setting rather than trusting the stored flag.
    if settings.start_with_windows != autostart::is_enabled() {
        autostart::set(settings.start_with_windows, settings.start_hidden)?;
    } else if settings.start_with_windows {
        // The hidden flag may have changed even when the toggle did not.
        autostart::set(true, settings.start_hidden)?;
    }

    {
        let mut config = state.config.write();
        config.settings = settings.clone();
    }

    state.persist()?;
    Ok(settings)
}

#[tauri::command]
pub fn get_autostart_state() -> bool {
    autostart::is_enabled()
}

/// Copies a user-chosen image into Oracle's own data directory.
///
/// Storing a reference to wherever the user happened to pick the file from would break the
/// moment they tidied their downloads folder.
#[tauri::command]
pub fn import_icon(source: PathBuf) -> Result<PathBuf> {
    let dir = config::icons_dir();
    std::fs::create_dir_all(&dir).map_err(|err| OracleError::Io {
        path: dir.clone(),
        source: err,
    })?;

    let extension = source
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "png".into());

    let target = dir.join(format!("{}.{extension}", uuid::Uuid::new_v4()));

    std::fs::copy(&source, &target).map_err(|err| OracleError::Io {
        path: source,
        source: err,
    })?;

    Ok(target)
}

// ---------------------------------------------------------------------------
// Shell integration
// ---------------------------------------------------------------------------

/// Reveals a folder in the system file manager.
#[tauri::command]
pub fn reveal_folder(path: PathBuf) -> Result<()> {
    if !path.exists() {
        return Err(OracleError::MissingWorkingDir(path));
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new("explorer");
        command.arg(&path);
        command.creation_flags(0x0800_0000);
        command
            .spawn()
            .map_err(|err| OracleError::Other(format!("cannot open the folder: {err}")))?;
    }

    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|err| OracleError::Other(format!("cannot open the folder: {err}")))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelAnchor {
    pub width: f64,
    pub height: f64,
}

#[tauri::command]
pub fn show_main_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }

    // Opening the full app should dismiss the panel it was opened from.
    if let Some(panel) = app.get_webview_window("panel") {
        let _ = panel.hide();
    }
}

#[tauri::command]
pub fn hide_panel(app: AppHandle) {
    if let Some(panel) = app.get_webview_window("panel") {
        let _ = panel.hide();
    }
}

#[tauri::command]
pub async fn quit_app(app: AppHandle, state: St<'_>) -> Result<()> {
    // Children outlive their parent on Windows unless they are killed explicitly.
    state.runner.stop_all().await;
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::ProcessManager;

    fn state_with(projects: Vec<Project>) -> Arc<AppState> {
        let runner = Arc::new(ProcessManager::new(Arc::new(|_| {})));
        let loaded = config::Loaded {
            config: Config {
                projects,
                ..Config::default()
            },
            recovered_from: None,
            is_first_run: true,
        };
        Arc::new(AppState::new(loaded, runner))
    }

    #[test]
    fn a_view_falls_back_to_the_accent_of_its_kind() {
        let mut project = Project::new("Site");
        project.kind = ProjectKind::Rust;
        let state = state_with(vec![project.clone()]);

        let rendered = view(&state, &project);
        assert_eq!(rendered.resolved_accent, ProjectKind::Rust.accent());
        assert_eq!(rendered.kind_label, "Rust");
    }

    #[test]
    fn an_explicit_accent_wins_over_the_kind() {
        let mut project = Project::new("Site");
        project.kind = ProjectKind::Rust;
        project.accent = Some("#123456".into());
        let state = state_with(vec![project.clone()]);

        assert_eq!(view(&state, &project).resolved_accent, "#123456");
    }

    #[test]
    fn a_project_that_never_ran_reads_as_stopped_and_unchecked() {
        let project = Project::new("Idle");
        let state = state_with(vec![project.clone()]);

        let rendered = view(&state, &project);
        assert_eq!(rendered.status, ProjectStatus::Stopped);
        assert!(matches!(rendered.remote_status, RemoteStatus::Unchecked));
        assert!(rendered.pid.is_none());
    }

    #[test]
    fn warnings_are_delivered_once() {
        let runner = Arc::new(ProcessManager::new(Arc::new(|_| {})));
        let loaded = config::Loaded {
            config: Config::default(),
            recovered_from: Some(PathBuf::from("C:/tmp/config.corrupt-1.json")),
            is_first_run: false,
        };
        let state = Arc::new(AppState::new(loaded, runner));

        assert_eq!(state.take_warnings().len(), 1);
        // A second read must be empty, or the toast would reappear on every refresh.
        assert!(state.take_warnings().is_empty());
    }
}
