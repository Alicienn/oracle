//! The tray panel's window lifetime.
//!
//! The panel used to be built during startup and kept hidden for the whole session. That is
//! a second WebView2 renderer — around 35 MB resident — held for a window most sessions open
//! for a few seconds at a time. It is now created on first open and released once it has
//! been hidden for a while.
//!
//! The delay is the point. Destroying it the moment it hides would make every toggle pay
//! the cost of creating a webview; holding it for a minute means a user who opens the panel
//! repeatedly never notices, while one who opens it once does not keep paying for it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const LABEL: &str = "panel";

/// How long the panel stays hidden before its webview is released.
const IDLE_RELEASE: Duration = Duration::from_secs(60);

/// Bumped on every show and hide.
///
/// A pending release captures the value it was scheduled at and does nothing if it has
/// moved, which is how showing the panel again cancels a release without needing a handle
/// on the task.
static EPOCH: AtomicU64 = AtomicU64::new(0);

pub fn existing<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(LABEL)
}

/// The panel window, built if it does not currently exist.
///
/// Returns `None` only if the window could not be created, which leaves the caller with
/// nothing to show — a failed panel must not take the app down with it.
pub fn ensure<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    if let Some(existing) = existing(app) {
        return Some(existing);
    }

    match build(app) {
        Ok(window) => {
            apply_effects(&window);
            Some(window)
        }
        Err(_) => None,
    }
}

/// Notes that the panel is on screen, cancelling any pending release.
pub fn mark_shown() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
}

/// Hides the panel and schedules its webview to be released if it stays hidden.
pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    let Some(panel) = existing(app) else {
        return;
    };

    let _ = panel.hide();

    let token = EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(IDLE_RELEASE).await;

        // Shown or hidden again since this was scheduled: whichever it was, a later task
        // owns the decision.
        if EPOCH.load(Ordering::SeqCst) != token {
            return;
        }

        if let Some(panel) = existing(&app) {
            if !panel.is_visible().unwrap_or(false) {
                // `destroy` rather than `close`: closing raises a close-requested event,
                // which is the hook the main window uses to stay alive in the tray.
                let _ = panel.destroy();
            }
        }
    });
}

/// The floating tray panel.
///
/// Kept out of the taskbar and above everything else: it behaves like a popover, not like a
/// second application window.
fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<WebviewWindow<R>> {
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("panel.html".into()))
        .title("Oracle Panel")
        .inner_size(400.0, 620.0)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .build()
}

/// Applies the native backdrop, so the CSS glass has something real to refract.
///
/// Best effort, exactly as for the main window: a machine with transparency effects turned
/// off simply gets the opaque background the CSS already defines.
fn apply_effects<R: Runtime>(window: &WebviewWindow<R>) {
    #[cfg(windows)]
    {
        use window_vibrancy::{apply_acrylic, apply_mica};

        if apply_mica(window, None).is_err() {
            let _ = apply_acrylic(window, Some((22, 20, 18, 160)));
        }
    }

    #[cfg(not(windows))]
    let _ = window;
}
