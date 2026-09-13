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
    /// The project's display name, so a port conflict can name the culprit.
    name: String,
    /// The port this run is expected to hold, after any override.
    port: Option<u16>,
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
    /// Where background tasks are spawned.
    ///
    /// Held explicitly rather than relying on `tokio::spawn`, which needs the *calling*
    /// thread to be inside a runtime. Commands arrive on whatever thread the IPC layer
    /// happens to use, so depending on that would be a latent panic.
    runtime: tokio::runtime::Handle,
}

impl ProcessManager {
    pub fn new(sink: EventSink, runtime: tokio::runtime::Handle) -> Self {
        Self {
            handles: Arc::new(RwLock::new(HashMap::new())),
            sink,
            runtime,
        }
    }

    fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.runtime.spawn(future);
    }

    /// Spawns the project's command and begins watching it.
    ///
    /// Returns as soon as the child process exists. The project reaches `Running` only once
    /// its port answers, which happens on a background task.
    ///
    /// `port_override` replaces the project's declared port for this run only: it is handed
    /// to the child through the environment and never written back to the configuration.
    pub async fn start(&self, project: &Project, port_override: Option<u16>) -> Result<u32> {
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

        // The port is settled before anything is spawned. Discovering the collision from
        // the child's own output would mean a process that appears to start and then dies,
        // which is precisely the experience this check exists to replace.
        let port = port_override.or(local.port);
        if let Some(port) = port {
            self.ensure_port_free(port, &project.id).await?;
        }

        let command = match port_override {
            Some(port) => apply_port_override(command, port),
            None => command.to_string(),
        };

        let mut child = build_command(&command, project, port_override)?
            .spawn()
            .map_err(|err| OracleError::SpawnFailed(err.to_string()))?;

        let pid = child
            .id()
            .ok_or_else(|| OracleError::SpawnFailed("the process exited immediately".into()))?;

        let ring = Arc::new(Mutex::new(LogRing::new()));
        let status = Arc::new(RwLock::new(ProjectStatus::Starting));
        let stop_requested = Arc::new(AtomicBool::new(false));

        self.append_log(&ring, &project.id, Stream::System, format!("$ {command}"));

        if let Some(port) = port_override {
            self.append_log(
                &ring,
                &project.id,
                Stream::System,
                format!("Started on port {port} for this run only (PORT={port})."),
            );
        }

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
                name: project.name.clone(),
                port,
                started_at: chrono::Utc::now().timestamp_millis(),
                logs: ring.clone(),
                status: status.clone(),
                stop_requested: stop_requested.clone(),
                adopted: false,
            },
        );

        self.emit_status(&project.id, ProjectStatus::Starting);
        self.watch_exit(child, project.id.clone(), ring.clone(), status.clone(), stop_requested);
        self.watch_readiness(&project.id, port, ring, status);

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
                name: project.name.clone(),
                port: Some(port),
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

        self.spawn(async move {
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

        self.spawn(async move {
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

    /// Refuses a port that something else already holds.
    ///
    /// Another Oracle project is checked first: it gives a message the user can act on
    /// ("Promethee has it") rather than a bare PID, and it catches the collision even during
    /// the window where the other project has spawned but not yet bound its port.
    async fn ensure_port_free(&self, port: u16, project_id: &str) -> Result<()> {
        let conflict = self.handles.read().iter().find_map(|(id, handle)| {
            (id != project_id && handle.port == Some(port)).then(|| handle.name.clone())
        });

        if let Some(holder) = conflict {
            return Err(OracleError::PortInUse {
                port,
                holder,
                suggestion: probe::find_free_port(port),
            });
        }

        if !probe::port_is_taken(port) {
            return Ok(());
        }

        // Something outside Oracle holds it. A PID is the most we can say, and even that is
        // best effort — the listener may be in a container or owned by another user.
        let holder = match probe::pid_on_port(port).await {
            Some(pid) => format!("another process (pid {pid})"),
            None => "another process".to_string(),
        };

        Err(OracleError::PortInUse {
            port,
            holder,
            suggestion: probe::find_free_port(port),
        })
    }

    /// Moves the project from `Starting` to `Running` once its port answers.
    fn watch_readiness(
        &self,
        project_id: &str,
        port: Option<u16>,
        ring: Arc<Mutex<LogRing>>,
        status: Arc<RwLock<ProjectStatus>>,
    ) {
        let project_id = project_id.to_string();
        let sink = self.sink.clone();

        self.spawn(async move {
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
/// Rewrites a command line so a one-off port override actually reaches the server.
///
/// Most dev servers read `PORT` from the environment, which `build_command` sets. Vite and
/// `serve` do not: they take `--port` and ignore the variable entirely, so for those the
/// flag has to be appended.
///
/// A command that already names a port is left exactly as written. The user was explicit,
/// and a second `--port` would either be rejected or silently win over the first.
///
/// The honest limitation: a Vite project launched as `npm run dev` is indistinguishable from
/// any other npm script, so the flag cannot be added and the override may not take. The
/// conflict is still reported rather than hidden, which is the part that matters.
fn apply_port_override(command: &str, port: u16) -> String {
    const TAKES_A_FLAG: [&str; 2] = ["vite", "serve"];

    if names_a_port(command) {
        return command.to_string();
    }

    let mentions = |name: &str| {
        command
            .split(|c: char| c.is_whitespace() || c == '/' || c == '\\')
            .any(|token| token.eq_ignore_ascii_case(name))
    };

    if TAKES_A_FLAG.iter().any(|name| mentions(name)) {
        format!("{command} --port {port}")
    } else {
        command.to_string()
    }
}

/// True when the command already sets a port itself, in any of the spellings in use.
fn names_a_port(command: &str) -> bool {
    command.split_whitespace().any(|token| {
        token == "--port"
            || token == "-p"
            || token == "-l"
            || token.starts_with("--port=")
            || token.starts_with("PORT=")
    })
}

fn build_command(
    command: &str,
    project: &Project,
    port_override: Option<u16>,
) -> Result<tokio::process::Command> {
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

    // Applied after the project's own environment so the override wins for this run. The
    // stored configuration is untouched: the next ordinary start goes back to its own port.
    if let Some(port) = port_override {
        std_command.env("PORT", port.to_string());
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

    /// A command that outlives the call that spawned it without outliving the test.
    #[cfg(windows)]
    const SHORT_LIVED: &str = "ping -n 3 127.0.0.1";
    #[cfg(not(windows))]
    const SHORT_LIVED: &str = "sleep 2";

    fn project_running(command: &str) -> Project {
        let mut project = Project::new("Test");
        project.local = Some(LocalTarget::new(std::env::temp_dir(), command));
        project
    }

    #[tokio::test]
    async fn starting_without_a_local_target_is_rejected() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink, tokio::runtime::Handle::current());

        let err = manager.start(&Project::new("Bare"), None).await.unwrap_err();
        assert_eq!(err.code(), "no_local_target");
    }

    #[tokio::test]
    async fn an_empty_command_is_rejected() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink, tokio::runtime::Handle::current());

        let err = manager.start(&project_running("   "), None).await.unwrap_err();
        assert_eq!(err.code(), "empty_command");
    }

    #[tokio::test]
    async fn a_missing_working_directory_is_rejected() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink, tokio::runtime::Handle::current());

        let mut project = Project::new("Gone");
        project.local = Some(LocalTarget::new(
            PathBuf::from("Z:/definitely/not/here"),
            "echo hi",
        ));

        let err = manager.start(&project, None).await.unwrap_err();
        assert_eq!(err.code(), "missing_working_dir");
    }

    #[tokio::test]
    async fn stopping_something_that_never_ran_is_rejected() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink, tokio::runtime::Handle::current());

        let err = manager.stop("nope").await.unwrap_err();
        assert_eq!(err.code(), "not_running");
    }

    #[tokio::test]
    async fn a_short_command_runs_and_its_output_is_captured() {
        let (sink, recorded) = recording_sink();
        let manager = ProcessManager::new(sink, tokio::runtime::Handle::current());

        let project = project_running("echo oracle-marker");
        let pid = manager.start(&project, None).await.expect("should spawn");
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
        let manager = ProcessManager::new(sink, tokio::runtime::Handle::current());

        // A command that stays alive long enough for the second call to collide.
        #[cfg(windows)]
        let project = project_running("ping -n 6 127.0.0.1");
        #[cfg(not(windows))]
        let project = project_running("sleep 6");

        manager.start(&project, None).await.expect("first start should work");
        let err = manager.start(&project, None).await.unwrap_err();

        assert_eq!(err.code(), "already_running");
        manager.stop(&project.id).await.ok();
    }

    #[test]
    fn an_override_adds_the_flag_only_where_the_environment_is_ignored() {
        // Vite and serve read the flag, not PORT.
        assert_eq!(apply_port_override("npx vite", 3001), "npx vite --port 3001");
        assert_eq!(apply_port_override("npx serve .", 8081), "npx serve . --port 8081");

        // Everything else is left alone: PORT in the environment already covers it, and
        // appending a flag a program does not know is how a start turns into a usage error.
        assert_eq!(apply_port_override("npm run dev", 3001), "npm run dev");
        assert_eq!(apply_port_override("cargo run", 3001), "cargo run");
    }

    #[test]
    fn an_explicit_port_in_the_command_is_never_rewritten() {
        for command in [
            "npx vite --port 4000",
            "npx vite --port=4000",
            "npx serve -l 4000",
            "npx serve -p 4000",
            "PORT=4000 npm start",
        ] {
            assert_eq!(apply_port_override(command, 3001), command);
        }
    }

    #[test]
    fn the_flag_survives_a_path_qualified_binary() {
        assert_eq!(
            apply_port_override("./node_modules/.bin/vite", 3001),
            "./node_modules/.bin/vite --port 3001"
        );
    }

    #[tokio::test]
    async fn a_port_held_by_another_process_blocks_the_start() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink, tokio::runtime::Handle::current());

        // Hold a port the way a foreign dev server would.
        let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = squatter.local_addr().unwrap().port();

        let mut project = project_running("echo never-runs");
        project.local.as_mut().unwrap().port = Some(port);

        let err = manager.start(&project, None).await.unwrap_err();
        assert_eq!(err.code(), "port_in_use");

        // The error has to carry a way out, not just a complaint.
        let wire = serde_json::to_value(&err).unwrap();
        assert_eq!(wire["data"]["port"], port);
        assert!(wire["data"]["suggestion"].as_u64().is_some());
        assert!(!manager.is_running(&project.id));
    }

    #[tokio::test]
    async fn two_projects_cannot_hold_the_same_port() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink, tokio::runtime::Handle::current());

        // A free port, declared by both projects. The first start claims it.
        let port = probe::find_free_port(41000).expect("a free port should exist");

        // Long enough to still hold a handle for the second start, short enough to reap
        // itself when the test ends. A long-lived child plus the taskkill to stop it costs
        // enough wall-clock to starve the timing-sensitive tests running alongside.
        let mut first = project_running(SHORT_LIVED);
        first.name = "First".into();
        first.local.as_mut().unwrap().port = Some(port);

        let mut second = project_running(SHORT_LIVED);
        second.name = "Second".into();
        second.local.as_mut().unwrap().port = Some(port);

        manager.start(&first, None).await.expect("the first should start");

        let err = manager.start(&second, None).await.unwrap_err();
        assert_eq!(err.code(), "port_in_use");
        // Naming the project is the whole point of checking Oracle's own handles first: the
        // command has not bound the port yet, so a probe alone would see it as free.
        assert!(err.to_string().contains("First"), "got: {err}");
    }

    #[tokio::test]
    async fn an_override_starts_a_project_whose_port_is_taken() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink, tokio::runtime::Handle::current());

        // Hold the declared port with a bare socket rather than a second project: this test
        // is about the override being honoured, not about who the squatter is.
        let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = squatter.local_addr().unwrap().port();
        let spare = probe::find_free_port(taken).expect("a free port should exist");

        let mut project = project_running(SHORT_LIVED);
        project.local.as_mut().unwrap().port = Some(taken);

        // Without the override this is the rejection the previous test asserts.
        assert_eq!(
            manager.start(&project, None).await.unwrap_err().code(),
            "port_in_use"
        );

        manager
            .start(&project, Some(spare))
            .await
            .expect("the override should clear the conflict");
        assert!(manager.is_running(&project.id));
    }

    #[tokio::test]
    async fn a_running_project_can_be_stopped() {
        let (sink, _) = recording_sink();
        let manager = ProcessManager::new(sink, tokio::runtime::Handle::current());

        #[cfg(windows)]
        let project = project_running("ping -n 30 127.0.0.1");
        #[cfg(not(windows))]
        let project = project_running("sleep 30");

        manager.start(&project, None).await.expect("should spawn");
        assert!(manager.is_running(&project.id));

        manager.stop(&project.id).await.expect("should stop");
        assert!(!manager.is_running(&project.id));
    }
}
