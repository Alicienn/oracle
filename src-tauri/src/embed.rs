//! Project web apps, shown inside Oracle's own window.
//!
//! A real child webview, not an iframe. An iframe would be blocked twice over: by the app's
//! own content-security policy, and by any site sending `X-Frame-Options` or a
//! `frame-ancestors` directive — which would work for a local dev server and fail on a
//! deployment, the worst possible split.
//!
//! A child webview is a browser in its own process with no framing rules to obey. Two
//! consequences shape everything here:
//!
//! * It does not participate in CSS layout. It is positioned in logical pixels from Rust,
//!   so the frontend has to report where the central panel is and keep reporting it as the
//!   window resizes and the rail expands.
//! * It paints over the glass, being a native surface. Nothing blurs behind it and it does
//!   not clip to the window's rounded corners.
//!
//! Webviews are kept alive once opened, and hidden rather than closed. Cookies would survive
//! either way — WebView2 keeps a persistent user-data folder per application — but the
//! in-page state would not: a half-filled form, a scroll position, an SPA's route. That is
//! what leaving and coming back should preserve. It costs a renderer process each, which is
//! why the memory is reported rather than hidden.

use crate::state::AppState;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl};

/// How long to wait before looking for the renderer process a new webview spawned.
///
/// WebView2 creates it asynchronously, so a snapshot taken the moment `add_child` returns
/// finds nothing.
const RENDERER_SETTLE: Duration = Duration::from_millis(900);

/// Where the webview should sit, in the frontend's own coordinates.
///
/// Logical pixels: the same units `getBoundingClientRect` reports, which Tauri scales for
/// the monitor. Passing physical pixels would misplace the webview on any display that is
/// not at 100%.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A project's live webview.
pub struct Embedded {
    pub label: String,
    /// The renderer processes this webview brought into being, for the memory readout.
    pub pids: Vec<u32>,
}

/// Everything Oracle has opened, and which one is on screen.
#[derive(Default)]
pub struct Embeds {
    pub views: HashMap<String, Embedded>,
    pub visible: Option<String>,
}

fn label_for(project_id: &str) -> String {
    format!("embed-{project_id}")
}

/// Shows a project's web app, creating its webview the first time.
///
/// The webview is given no capabilities — its label is not in any capability file — so a
/// page loaded here cannot reach Oracle's commands. That is the default and it is the point:
/// this is someone else's web app running inside our window.
pub fn open(app: &AppHandle, project_id: &str, url: &str, rect: Rect) -> Result<(), String> {
    let window = app
        .get_window("main")
        .ok_or_else(|| "the main window is gone".to_string())?;

    let state = app
        .try_state::<Arc<AppState>>()
        .ok_or_else(|| "Oracle is shutting down".to_string())?;

    // Whatever was on screen steps aside first, or two webviews overlap in the same space.
    hide_visible(app);

    let label = label_for(project_id);
    let existing = state.embeds.read().views.contains_key(project_id);

    if existing {
        if let Some(webview) = app.get_webview(&label) {
            let _ = webview.set_position(LogicalPosition::new(rect.x, rect.y));
            let _ = webview.set_size(LogicalSize::new(rect.width, rect.height));
            webview.show().map_err(|err| err.to_string())?;
        }
    } else {
        let parsed = url
            .parse()
            .map_err(|_| format!("{url} is not an address Oracle can open"))?;

        let before = renderer_pids();

        let builder = tauri::webview::WebviewBuilder::new(&label, WebviewUrl::External(parsed));
        window
            .add_child(
                builder,
                LogicalPosition::new(rect.x, rect.y),
                LogicalSize::new(rect.width, rect.height),
            )
            .map_err(|err| err.to_string())?;

        state.embeds.write().views.insert(
            project_id.to_string(),
            Embedded {
                label: label.clone(),
                pids: Vec::new(),
            },
        );

        // The renderer appears a moment later; its pid is what makes the memory readout
        // possible, so it is collected once the dust settles rather than guessed at.
        let handle = app.clone();
        let id = project_id.to_string();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(RENDERER_SETTLE).await;

            let fresh: Vec<u32> = renderer_pids()
                .into_iter()
                .filter(|pid| !before.contains(pid))
                .collect();

            if let Some(state) = handle.try_state::<Arc<AppState>>() {
                if let Some(view) = state.embeds.write().views.get_mut(&id) {
                    view.pids = fresh;
                }
            }
        });
    }

    state.embeds.write().visible = Some(project_id.to_string());
    Ok(())
}

/// Takes the visible webview off screen, keeping it and its page state alive.
pub fn hide_visible(app: &AppHandle) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };

    let label = {
        let embeds = state.embeds.read();
        embeds
            .visible
            .as_ref()
            .and_then(|id| embeds.views.get(id))
            .map(|view| view.label.clone())
    };

    if let Some(label) = label {
        if let Some(webview) = app.get_webview(&label) {
            let _ = webview.hide();
        }
    }

    state.embeds.write().visible = None;
}

/// Moves the visible webview, after the panel around it changed shape.
pub fn set_bounds(app: &AppHandle, rect: Rect) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };

    let label = {
        let embeds = state.embeds.read();
        embeds
            .visible
            .as_ref()
            .and_then(|id| embeds.views.get(id))
            .map(|view| view.label.clone())
    };

    if let Some(webview) = label.and_then(|label| app.get_webview(&label)) {
        let _ = webview.set_position(LogicalPosition::new(rect.x, rect.y));
        let _ = webview.set_size(LogicalSize::new(rect.width, rect.height));
    }
}

/// Closes a project's webview for good, releasing its renderer.
///
/// The way out of paying for a view that is being kept alive. Page state goes with it;
/// cookies do not, so this is not a sign-out.
pub fn close(app: &AppHandle, project_id: &str) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };

    let removed = state.embeds.write().views.remove(project_id);

    if let Some(view) = removed {
        if let Some(webview) = app.get_webview(&view.label) {
            let _ = webview.close();
        }
    }

    let mut embeds = state.embeds.write();
    if embeds.visible.as_deref() == Some(project_id) {
        embeds.visible = None;
    }
}

/// Sends the visible webview back to the address it should be showing.
pub fn reload(app: &AppHandle) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };

    let label = {
        let embeds = state.embeds.read();
        embeds
            .visible
            .as_ref()
            .and_then(|id| embeds.views.get(id))
            .map(|view| view.label.clone())
    };

    if let Some(webview) = label.and_then(|label| app.get_webview(&label)) {
        let _ = webview.eval("window.location.reload()");
    }
}

/// Resident memory of the visible webview's renderers, in bytes.
///
/// `None` when nothing is open, or when no renderer could be attributed to it: WebView2 can
/// reuse a process, and reporting the wrong number is worse than reporting none.
pub fn visible_memory(app: &AppHandle) -> Option<(String, u64)> {
    let state = app.try_state::<Arc<AppState>>()?;

    let (project_id, pids) = {
        let embeds = state.embeds.read();
        let id = embeds.visible.clone()?;
        let pids = embeds.views.get(&id)?.pids.clone();
        (id, pids)
    };

    if pids.is_empty() {
        return None;
    }

    let total = state.monitor.lock().memory_of(&pids);
    (total > 0).then_some((project_id, total))
}

/// The pids of every WebView2 renderer running right now.
///
/// A whole-system scan, which is heavy enough that it only happens when a webview is
/// created — twice, either side of it, so the difference identifies the new process.
fn renderer_pids() -> Vec<u32> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());

    system
        .processes()
        .values()
        .filter(|process| {
            process
                .name()
                .to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("msedgewebview2.exe"))
        })
        .map(|process| process.pid().as_u32())
        .collect()
}
