//! Project web apps, shown over Oracle's central panel.
//!
//! A real browser, not an iframe. An iframe is refused twice over: by Oracle's own
//! content-security policy, and by any site sending `X-Frame-Options` — which would have
//! worked for a local dev server and failed on a deployment, the worst possible split.
//!
//! ## Why an owned window and not a child webview
//!
//! Tauri can put several webviews in one window, and that was the first implementation. It
//! does not work here, for a reason worth writing down: multiple webviews are meant to
//! *partition* a window, each owning a disjoint rectangle. Oracle's own webview already
//! covers the whole client area, so a child added on top of it lands behind it — no error,
//! a renderer process running, a page loaded, and nothing on screen. The panel stayed black
//! while `http://localhost:3000` was answering with 282 KB of HTML.
//!
//! So the web app gets its own undecorated window, owned by the main one. An owned window is
//! always above its owner, which settles the z-order for good. The cost is that it does not
//! participate in the main window's layout at all, and has to be kept in place by hand:
//!
//! * The frontend reports where the panel is, in CSS pixels.
//! * Those are client coordinates, so they are offset by the main window's position and
//!   scaled for the monitor to become screen coordinates.
//! * The main window moving, resizing, or being minimised all have to be followed, because
//!   an owned window would otherwise sit in mid-air over the desktop.
//!
//! ## Why webviews are kept alive
//!
//! Leaving a web app hides its window instead of closing it. Cookies would survive either
//! way — WebView2 keeps a persistent user-data folder per application — so signing in again
//! was never the risk. What dies with a closed webview is the page state: a half-filled
//! form, a scroll position, an SPA's route. That is what coming back should preserve. It
//! costs a renderer process each, which is why the memory is reported rather than hidden.

use crate::state::AppState;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewUrl};

/// How long to wait before looking for the renderer process a new window spawned.
///
/// WebView2 creates it asynchronously, so a snapshot taken the moment the window is built
/// finds nothing.
const RENDERER_SETTLE: Duration = Duration::from_millis(900);

/// Where the web app should sit, in the frontend's own coordinates.
///
/// CSS pixels, as `getBoundingClientRect` reports them, relative to the main window's client
/// area. Turning those into a screen position is this module's job.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A project's live web app window.
pub struct Embedded {
    pub label: String,
    /// The renderer processes it brought into being, for the memory readout.
    pub pids: Vec<u32>,
}

#[derive(Default)]
pub struct Embeds {
    pub views: HashMap<String, Embedded>,
    pub visible: Option<String>,
    /// The last rectangle the frontend reported, replayed whenever the window moves.
    pub rect: Option<Rect>,
}

fn label_for(project_id: &str) -> String {
    format!("embed-{project_id}")
}

/// Shows a project's web app, building its window the first time.
///
/// The window is given no capabilities — its label is in no capability file — so the page
/// cannot reach Oracle's commands. That is the default, and it is the point: this is someone
/// else's web app running inside our frame.
pub fn open<R: Runtime>(
    app: &AppHandle<R>,
    project_id: &str,
    url: &str,
    rect: Rect,
) -> Result<(), String> {
    let state = app
        .try_state::<Arc<AppState>>()
        .ok_or_else(|| "Oracle is shutting down".to_string())?;

    // Whatever was on screen steps aside first, or two web apps overlap in the same space.
    hide_visible(app);

    state.embeds.write().rect = Some(rect);

    let label = label_for(project_id);
    let known = state.embeds.read().views.contains_key(project_id);

    if !known {
        let before = build(app, &label, url)?;

        state.embeds.write().views.insert(
            project_id.to_string(),
            Embedded {
                label: label.clone(),
                pids: Vec::new(),
            },
        );

        collect_pids_later(app, project_id, before);
    }

    state.embeds.write().visible = Some(project_id.to_string());

    // Positioned before being shown, so it never appears in the wrong place first.
    follow_owner(app);

    // Verified rather than assumed. The window is a native surface that Oracle's own
    // interface cannot see: if it failed to appear, the panel would show an empty rectangle
    // and say nothing, which is how the first two attempts at this feature looked.
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("the window for {url} was not created"))?;

    if !window.is_visible().unwrap_or(false) {
        return Err(format!("the window for {url} was created but stayed hidden"));
    }

    Ok(())
}

/// Builds the window, returning the renderer processes that existed beforehand.
fn build<R: Runtime>(app: &AppHandle<R>, label: &str, url: &str) -> Result<Vec<u32>, String> {
    let main = app
        .get_webview_window("main")
        .ok_or_else(|| "the main window is gone".to_string())?;

    let parsed: tauri::Url = url
        .parse()
        .map_err(|_| format!("{url} is not an address Oracle can open"))?;

    let before = renderer_pids();

    tauri::WebviewWindowBuilder::new(app, label, WebviewUrl::External(parsed))
        // Owned by the main window: that is what keeps it above, and what makes it follow
        // the main window into the background instead of floating over other applications.
        .owner(&main)
        .map_err(|err| err.to_string())?
        .decorations(false)
        .resizable(false)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .build()
        .map_err(|err| err.to_string())?;

    Ok(before)
}

/// Attributes the new renderer to this project, once WebView2 has created it.
fn collect_pids_later<R: Runtime>(app: &AppHandle<R>, project_id: &str, before: Vec<u32>) {
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

/// Takes the visible web app off screen, keeping it and its page state alive.
pub fn hide_visible<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };

    if let Some(window) = visible_window(app, &state) {
        let _ = window.hide();
    }

    state.embeds.write().visible = None;
}

/// Records a new rectangle and moves the web app onto it.
pub fn set_bounds<R: Runtime>(app: &AppHandle<R>, rect: Rect) {
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        state.embeds.write().rect = Some(rect);
    }

    follow_owner(app);
}

/// Keeps the visible web app over the panel, wherever the main window now is.
///
/// An owned window holds its own screen position and its own visibility, so everything the
/// main window does has to be mirrored: dragging Oracle would leave the page behind,
/// minimising it would leave the page drawing over the desktop, and hiding to the tray would
/// leave it on screen with nothing behind it.
///
/// Self-correcting on purpose — it decides from the current state rather than from what
/// happened, so it can be called from any window event without tracking which.
pub fn follow_owner<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };

    let Some(main) = app.get_webview_window("main") else {
        return;
    };

    let Some(window) = visible_window(app, &state) else {
        return;
    };

    // A minimised owner leaves an owned window drawing over the desktop.
    if main.is_minimized().unwrap_or(false) || !main.is_visible().unwrap_or(true) {
        let _ = window.hide();
        return;
    }

    let Some(rect) = state.embeds.read().rect else {
        return;
    };

    let Ok(origin) = main.inner_position() else {
        return;
    };
    let scale = main.scale_factor().unwrap_or(1.0);

    // Client coordinates are logical; the window's own position is physical. Mixing the two
    // is how a panel lands an inch off on a scaled display.
    let position = PhysicalPosition::new(
        origin.x + (rect.x * scale).round() as i32,
        origin.y + (rect.y * scale).round() as i32,
    );
    let size = PhysicalSize::new(
        (rect.width * scale).round().max(1.0) as u32,
        (rect.height * scale).round().max(1.0) as u32,
    );

    let _ = window.set_position(position);
    let _ = window.set_size(size);
    let _ = window.show();
}

/// Closes a project's web app for good, releasing its renderer.
pub fn close<R: Runtime>(app: &AppHandle<R>, project_id: &str) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };

    let removed = state.embeds.write().views.remove(project_id);

    if let Some(view) = removed {
        if let Some(window) = app.get_webview_window(&view.label) {
            let _ = window.destroy();
        }
    }

    let mut embeds = state.embeds.write();
    if embeds.visible.as_deref() == Some(project_id) {
        embeds.visible = None;
    }
}

/// Closes every web app. Called when Oracle itself is shutting down.
pub fn close_all<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };

    let labels: Vec<String> = state
        .embeds
        .read()
        .views
        .values()
        .map(|view| view.label.clone())
        .collect();

    for label in labels {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.destroy();
        }
    }
}

pub fn reload<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };

    if let Some(window) = visible_window(app, &state) {
        let _ = window.eval("window.location.reload()");
    }
}

/// Resident memory of the visible web app's renderers, in bytes.
///
/// `None` when nothing is open, or when no renderer could be attributed to it: WebView2 can
/// reuse a process, and reporting the wrong number is worse than reporting none.
pub fn visible_memory<R: Runtime>(app: &AppHandle<R>) -> Option<(String, u64)> {
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

fn visible_window<R: Runtime>(
    app: &AppHandle<R>,
    state: &Arc<AppState>,
) -> Option<tauri::WebviewWindow<R>> {
    let label = {
        let embeds = state.embeds.read();
        embeds
            .visible
            .as_ref()
            .and_then(|id| embeds.views.get(id))
            .map(|view| view.label.clone())
    };

    label.and_then(|label| app.get_webview_window(&label))
}

/// The pids of every WebView2 renderer running right now.
///
/// A whole-system scan, heavy enough that it only happens when a web app is opened — twice,
/// either side of it, so the difference identifies the new process.
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
