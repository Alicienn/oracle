//! Starting, watching, and stopping local project processes.
//!
//! The manager owns one handle per running project. It never blocks: spawning returns as
//! soon as the child exists, and everything after that — reading output, waiting for the
//! port, noticing the exit — happens in background tasks that report back through an event
//! callback.
//!
//! The callback is a plain closure rather than a Tauri `AppHandle` so this module stays
//! testable and knows nothing about the shell it runs inside.

pub mod logs;
pub mod probe;

use crate::config::model::Project;
use crate::error::{OracleError, Result};
use logs::{LogLine, LogRing, Stream};
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

/// How long a project gets to bind its port before Oracle calls it unhealthy.
const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a process gets to exit politely before it is killed outright.
const GRACEFUL_STOP: Duration = Duration::from_secs(5);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectStatus {
    Stopped,
    /// Spawned, but not yet serving on its declared port.
    Starting,
    Running,
    /// Alive, but never became reachable on its port.
    Unhealthy,
    /// Exited without being asked to.
    Crashed,
}

/// What the manager reports back to whoever is listening.
#[derive(Debug, Clone)]
pub enum RunnerEvent {
    Log { project_id: String, line: LogLine },
    Status { project_id: String, status: ProjectStatus },
}

pub type EventSink = Arc<dyn Fn(RunnerEvent) + Send + Sync>;

// ---------------------------------------------------------------------------
// Handle
// ---------------------------------------------------------------------------

struct ProcHandle {
    pid: u32,
    started_at: i64,
    logs: Arc<Mutex<LogRing>>,
    status: Arc<RwLock<ProjectStatus>>,
    /// Set before a kill so the exit watcher reports a stop rather than a crash.
    stop_requested: Arc<AtomicBool>,
    /// True when Oracle attached to a process it did not spawn.
    adopted: bool,
}

/// The public view of a running project.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningInfo {
    pub project_id: String,
    pub pid: u32,
    pub status: ProjectStatus,
    pub started_at: i64,
    pub adopted: bool,
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

pub struct ProcessManager {
    handles: Arc<RwLock<HashMap<String, ProcHandle>>>,
    sink: EventSink,
}

impl ProcessManager {
    pub fn new(sink: EventSink) -> Self {
        Self {
            handles: Arc::new(RwLock::new(HashMap::new())),
            sink,
        }
    }

    /// Spawns the project's command and begins watching it.
    ///
    /// Returns as soon as the child process exists. The project reaches `Running` only once
    /// its port answers, which happens on a background task.
    pub fn start(&self, project: &Project) -> Result<u32> {
        let local = project
            .local
            .as_ref()
            .ok_or_else(|| OracleError::NoLocalTarget(project.name.clone()))?;

        if self.is_running(&project.id) {
            return Err(OracleError::AlreadyRunning(project.name.clone()));
        }

        let command = local.command.trim();
        if command.is_empty() {
            return Err(OracleError::EmptyCommand);
        }

        if !local.root.is_dir() {
            return Err(OracleError::MissingWorkingDir(local.root.clone()));
        }

        let mut child = build_command(command, project)?
            .spawn()
            .map_err(|err| OracleError::SpawnFailed(err.to_string()))?;

        let pid = child
            .id()
            .ok_or_else(|| OracleError::SpawnFailed("the process exited immediately".into()))?;

        let ring = Arc::new(Mutex::new(LogRing::new()));
        let status = Arc::new(RwLock::new(ProjectStatus::Starting));
        let stop_requested = Arc::new(AtomicBool::new(false));

        self.append_log(&ring, &project.id, Stream::System, format!("$ {command}"));

        // Pipe both streams into the ring buffer.
        if let Some(stdout) = child.stdout.take() {
            self.pump(stdout, project.id.clone(), Stream::Stdout, ring.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            self.pump(stderr, project.id.clone(), Stream::Stderr, ring.clone());
        }

        self.handles.write().insert(
            project.id.clone(),
            ProcHandle {
                pid,
                started_at: chrono::Utc::now().timestamp_millis(),
                logs: ring.clone(),
                status: status.clone(),
                stop_requested: stop_requested.clone(),
                adopted: false,
            },
        );

        self.emit_status(&project.id, ProjectStatus::Starting);
        self.watch_exit(child, project.id.clone(), ring.clone(), status.clone(), stop_requested);
        self.watch_readiness(project, ring, status);

        Ok(pid)
    }

    /// Attaches to a process Oracle did not spawn, if the project's port is already taken.
    ///
    /// Output cannot be captured for an adopted process — the pipes belong to whoever
    /// started it — so its log buffer only ever holds Oracle's own notes.
    pub async fn adopt(&self, project: &Project) -> Option<u32> {
        let port = project.local.as_ref()?.port?;

        if self.is_running(&project.id) {
            return None;
        }

        let pid = probe::pid_on_port(port).await?;

        let ring = Arc::new(Mutex::new(LogRing::new()));
        self.append_log(
            &ring,
            &project.id,
            Stream::System,
            format!("Attached to an existing process on port {port} (pid {pid}). Output is not captured."),
        );

        self.handles.write().insert(
            project.id.clone(),
            ProcHandle {
                pid,
                started_at: chrono::Utc::now().timestamp_millis(),
                logs: ring,
                status: Arc::new(RwLock::new(ProjectStatus::Running)),
                stop_requested: Arc::new(AtomicBool::new(false)),
                adopted: true,
            },
        );

        self.emit_status(&project.id, ProjectStatus::Running);
        Some(pid)
    }

    /// Asks the process tree to exit, then forces it if it lingers.
    pub async fn stop(&self, project_id: &str) -> Result<()> {
        let (pid, ring, stop_flag) = {
            let handles = self.handles.read();
            let handle = handles
                .get(project_id)
                .ok_or_else(|| OracleError::NotRunning(project_id.to_string()))?;
            (handle.pid, handle.logs.clone(), handle.stop_requested.clone())
        };

        // Set before killing, so the exit watcher reports a clean stop rather than a crash.
        stop_flag.store(true, Ordering::SeqCst);
        self.append_log(&ring, project_id, Stream::System, "Stopping…");

        kill_tree(pid, false).await;

        // Give the tree a moment to unwind before forcing it.
        let deadline = tokio::time::Instant::now() + GRACEFUL_STOP;
        while tokio::time::Instant::now() < deadline {
            if !self.handles.read().contains_key(project_id) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        self.append_log(&ring, project_id, Stream::System, "Still alive, forcing.");
        kill_tree(pid, true).await;

        // Whatever happens to the OS process, Oracle must not keep a stale handle.
        self.forget(project_id, ProjectStatus::Stopped);
        Ok(())
    }

    /// Stops everything. Called when Oracle itself is shutting down.
    pub async fn stop_all(&self) {
        let ids: Vec<String> = self.handles.read().keys().cloned().collect();
        for id in ids {
            // One project refusing to die must not block the rest of shutdown.
            let _ = self.stop(&id).await;
        }
    }

    pub fn is_running(&self, project_id: &str) -> bool {
        self.handles.read().contains_key(project_id)
    }

    pub fn status(&self, project_id: &str) -> ProjectStatus {
        self.handles
            .read()
            .get(project_id)
            .map(|handle| *handle.status.read())
            .unwrap_or(ProjectStatus::Stopped)
    }

    /// Every running project, for the monitor and the UI.
    pub fn running(&self) -> Vec<RunningInfo> {
        self.handles
            .read()
            .iter()
            .map(|(id, handle)| RunningInfo {
                project_id: id.clone(),
                pid: handle.pid,
                status: *handle.status.read(),
                started_at: handle.started_at,
                adopted: handle.adopted,
            })
            .collect()
    }

    pub fn pid(&self, project_id: &str) -> Option<u32> {
        self.handles.read().get(project_id).map(|h| h.pid)
    }

    /// Buffered log lines, optionally only those newer than `since`.
    pub fn logs(&self, project_id: &str, since: Option<u64>) -> Vec<LogLine> {
        self.handles
            .read()
            .get(project_id)
            .map(|handle| {
                let ring = handle.logs.lock();
                match since {
                    Some(seq) => ring.since(seq),
                    None => ring.all(),
                }
            })
            .unwrap_or_default()
    }

    pub fn clear_logs(&self, project_id: &str) {
        if let Some(handle) = self.handles.read().get(project_id) {
            handle.logs.lock().clear();
        }
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    /// Reads a child stream line by line into the ring buffer.
    fn pump<R>(&self, reader: R, project_id: String, stream: Stream, ring: Arc<Mutex<LogRing>>)
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        let sink = self.sink.clone();

        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();

            // Dev servers emit plenty of invalid UTF-8 (progress spinners, ANSI art); the
            // reader replaces it rather than aborting, so a bad byte never stops the pump.
            while let Ok(Some(text)) = lines.next_line().await {
                let line = ring.lock().push(stream, text);
                sink(RunnerEvent::Log {
                    project_id: project_id.clone(),
                    line,
                });
            }
        });
    }

    /// Waits for the process to exit and decides whether that was a stop or a crash.
    fn watch_exit(
        &self,
        mut child: tokio::process::Child,
        project_id: String,
        ring: Arc<Mutex<LogRing>>,
        status: Arc<RwLock<ProjectStatus>>,
        stop_requested: Arc<AtomicBool>,
    ) {
        let sink = self.sink.clone();
        let handles_ref = self.handles_ref();

        tokio::spawn(async move {
            let outcome = child.wait().await;
            let was_asked = stop_requested.load(Ordering::SeqCst);

            let (final_status, note) = match outcome {
                Ok(exit) if was_asked => (ProjectStatus::Stopped, format!("Stopped ({exit}).")),
                Ok(exit) if exit.success() => {
                    (ProjectStatus::Stopped, "Process finished.".to_string())
                }
                Ok(exit) => (ProjectStatus::Crashed, format!("Process exited: {exit}.")),
                Err(err) => (
                    ProjectStatus::Crashed,
                    format!("Lost track of the process: {err}."),
                ),
            };

            let line = ring.lock().push(Stream::System, note);
            sink(RunnerEvent::Log {
                project_id: project_id.clone(),
                line,
            });

            *status.write() = final_status;

            // A crashed project keeps its handle so its logs stay readable; a stopped one
            // is forgotten so it can be started again cleanly.
            if final_status == ProjectStatus::Crashed {
                if let Some(handles) = handles_ref.upgrade() {
                    if let Some(handle) = handles.read().get(&project_id) {
                        *handle.status.write() = ProjectStatus::Crashed;
                    }
                }
            } else if let Some(handles) = handles_ref.upgrade() {
                handles.write().remove(&project_id);
            }

            sink(RunnerEvent::Status {
                project_id,
                status: final_status,
            });
        });
    }

    /// Moves the project from `Starting` to `Running` once its port answers.
    fn watch_readiness(
        &self,
        project: &Project,
        ring: Arc<Mutex<LogRing>>,
        status: Arc<RwLock<ProjectStatus>>,
    ) {
        let project_id = project.id.clone();
        let port = project.local.as_ref().and_then(|l| l.port);
        let sink = self.sink.clone();

        tokio::spawn(async move {
            let Some(port) = port else {
                // Without a declared port there is nothing to wait for: a process that is
                // alive is as running as Oracle can tell.
                *status.write() = ProjectStatus::Running;
                sink(RunnerEvent::Status {
                    project_id,
                    status: ProjectStatus::Running,
                });
                return;
            };

            let ready = probe::wait_for_port(port, READY_TIMEOUT).await;

            // The process may well have died while we waited; do not resurrect it.
            if !matches!(*status.read(), ProjectStatus::Starting) {
                return;
            }

            let (next, note) = if ready {
                (
                    ProjectStatus::Running,
                    format!("Listening on port {port}."),
                )
            } else {
                (
                    ProjectStatus::Unhealthy,
                    format!("Port {port} never opened after {}s.", READY_TIMEOUT.as_secs()),
                )
            };

            let line = ring.lock().push(Stream::System, note);
            sink(RunnerEvent::Log {
                project_id: project_id.clone(),
                line,
            });

            *status.write() = next;
            sink(RunnerEvent::Status {
                project_id,
                status: next,
            });
        });
    }

    fn append_log(
        &self,
        ring: &Arc<Mutex<LogRing>>,
        project_id: &str,
        stream: Stream,
        text: impl Into<String>,
    ) {
        let line = ring.lock().push(stream, text);
        (self.sink)(RunnerEvent::Log {
            project_id: project_id.to_string(),
            line,
        });
    }

    fn emit_status(&self, project_id: &str, status: ProjectStatus) {
        (self.sink)(RunnerEvent::Status {
            project_id: project_id.to_string(),
            status,
        });
    }

    fn forget(&self, project_id: &str, status: ProjectStatus) {
        self.handles.write().remove(project_id);
        self.emit_status(project_id, status);
    }

    /// The exit watcher outlives individual calls but must not keep the manager alive, so
    /// it holds a weak reference to the handle map.
    fn handles_ref(&self) -> std::sync::Weak<RwLock<HashMap<String, ProcHandle>>> {
        Arc::downgrade(&self.handles)
    }
}

// ---------------------------------------------------------------------------
// Process plumbing
// ---------------------------------------------------------------------------

/// Builds the child process command.
///
/// The command line is handed to the system shell verbatim rather than parsed here. Users
/// write things like `npm run dev` but also `npx prisma migrate && npm run dev`, and the
/// shell already knows how to handle quoting, chaining, and redirection. Re-implementing
/// that would add a parser and a class of bugs for no benefit.
fn build_command(command: &str, project: &Project) -> Result<tokio::process::Command> {
    let local = project
        .local
        .as_ref()
        .ok_or_else(|| OracleError::NoLocalTarget(project.name.clone()))?;

    #[cfg(windows)]
    let mut std_command = {
        use std::os::windows::process::CommandExt;
        let mut c = std::process::Command::new("cmd");
        // `raw_arg` avoids Rust's quoting, which cmd.exe parses differently from every
        // other program and would otherwise mangle the command.
        c.raw_arg(format!("/C {command}"));
        c.creation_flags(CREATE_NO_WINDOW);
        c
    };

    #[cfg(not(windows))]
    let mut std_command = {
        let mut c = std::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };

    std_command.current_dir(&local.root);
    for (key, value) in &local.env {
        std_command.env(key, value);
    }

    let mut command = tokio::process::Command::from(std_command);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(false);

    Ok(command)
}

/// Terminates a process and everything it spawned.
///
/// Killing only the PID Oracle holds would orphan the real work: `cmd /C npm run dev` is
/// three processes deep before the dev server appears.
async fn kill_tree(pid: u32, force: bool) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        let mut command = std::process::Command::new("taskkill");
        command.args(["/PID", &pid.to_string(), "/T"]);
        if force {
            command.arg("/F");
        }
        command.creation_flags(CREATE_NO_WINDOW);

        let _ = tokio::process::Command::from(command).output().await;
    }

    #[cfg(not(windows))]
    {
        let signal = if force { "-KILL" } else { "-TERM" };
        let _ = tokio::process::Command::new("kill")
            .args([signal, &format!("-{pid}")])
            .output()
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::LocalTarget;
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;

    fn recording_sink() -> (EventSink, Arc<StdMutex<Vec<RunnerEvent>>>) {
        let recorded = Arc::new(StdMutex::new(Vec::new()));
        let captured = recorded.clone();
        let sink: EventSink = Arc::new(move |event| captured.lock().unwrap().push(event));
        (sink, recorded)
    }

    fn project_running(command: &str) -> Project {
        let mut project = Project::new("Test");
        project.local = Some(LocalTarget::new(std::env::temp_dir(), command));
        project
    }

    #[tokio::test]
    async fn starting_without_a_local_target_is_rejected() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink);

        let err = manager.start(&Project::new("Bare")).unwrap_err();
        assert_eq!(err.code(), "no_local_target");
    }

    #[tokio::test]
    async fn an_empty_command_is_rejected() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink);

        let err = manager.start(&project_running("   ")).unwrap_err();
        assert_eq!(err.code(), "empty_command");
    }

    #[tokio::test]
    async fn a_missing_working_directory_is_rejected() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink);

        let mut project = Project::new("Gone");
        project.local = Some(LocalTarget::new(
            PathBuf::from("Z:/definitely/not/here"),
            "echo hi",
        ));

        let err = manager.start(&project).unwrap_err();
        assert_eq!(err.code(), "missing_working_dir");
    }

    #[tokio::test]
    async fn stopping_something_that_never_ran_is_rejected() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink);

        let err = manager.stop("nope").await.unwrap_err();
        assert_eq!(err.code(), "not_running");
    }

    #[tokio::test]
    async fn a_short_command_runs_and_its_output_is_captured() {
        let (sink, recorded) = recording_sink();
        let manager = ProcessManager::new(sink);

        let project = project_running("echo oracle-marker");
        let pid = manager.start(&project).expect("should spawn");
        assert!(pid > 0);

        // The process is tiny; give it room to run and be reaped.
        tokio::time::sleep(Duration::from_millis(1500)).await;

        let events = recorded.lock().unwrap();
        let printed = events.iter().any(|event| match event {
            RunnerEvent::Log { line, .. } => line.text.contains("oracle-marker"),
            _ => false,
        });

        assert!(printed, "the child's stdout should have reached the sink");
    }

    #[tokio::test]
    async fn starting_the_same_project_twice_is_rejected() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink);

        // A command that stays alive long enough for the second call to collide.
        #[cfg(windows)]
        let project = project_running("ping -n 6 127.0.0.1");
        #[cfg(not(windows))]
        let project = project_running("sleep 6");

        manager.start(&project).expect("first start should work");
        let err = manager.start(&project).unwrap_err();

        assert_eq!(err.code(), "already_running");
        manager.stop(&project.id).await.ok();
    }

    #[tokio::test]
    async fn a_running_project_can_be_stopped() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink);

        #[cfg(windows)]
        let project = project_running("ping -n 30 127.0.0.1");
        #[cfg(not(windows))]
        let project = project_running("sleep 30");

        manager.start(&project).expect("should spawn");
        assert!(manager.is_running(&project.id));

        manager.stop(&project.id).await.expect("should stop");
        assert!(!manager.is_running(&project.id));
    }
}
