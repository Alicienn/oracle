//! Oracle — a command center for local and remote projects.

pub mod autostart;
pub mod changelog;
pub mod commands;
pub mod config;
pub mod discovery;
pub mod embed;
pub mod error;
pub mod favicon;
pub mod monitor;
pub mod panel;
pub mod remote;
pub mod runner;
pub mod state;
pub mod tasks;
pub mod tray;
pub mod vcs;

use runner::{ProcessManager, RunnerEvent};
use state::AppState;
use std::sync::Arc;
use tauri::{Emitter, Manager, WindowEvent};

pub const LOG_EVENT: &str = "log:line";
pub const STATUS_EVENT: &str = "status:change";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Oracle owns its runtime rather than letting Tauri build one implicitly, so that the
    // same handle can be handed to the process manager. Entering it here also means code
    // running on the main thread — `setup`, and synchronous commands — is inside a runtime
    // context.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("oracle")
        .build()
        .expect("Oracle could not start its async runtime");

    tauri::async_runtime::set(runtime.handle().clone());
    let runtime_handle = runtime.handle().clone();
    let _runtime_guard = runtime.enter();

    tauri::Builder::default()
        // A second launch should surface the window that already exists rather than start
        // a rival instance fighting over the same config file and the same tray icon.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Needed to relaunch after an update installs.
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![autostart::HIDDEN_FLAG]),
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::get_changelog,
            commands::get_logs,
            commands::clear_logs,
            commands::git_status,
            commands::start_project,
            commands::stop_project,
            commands::restart_project,
            commands::add_project,
            commands::update_project,
            commands::delete_project,
            commands::reorder_projects,
            commands::toggle_favorite,
            commands::scan_projects,
            commands::import_candidates,
            commands::update_settings,
            commands::get_autostart_state,
            commands::import_icon,
            commands::refresh_favicon,
            commands::reveal_folder,
            commands::open_embed,
            commands::hide_embed,
            commands::set_embed_bounds,
            commands::close_embed,
            commands::reload_embed,
            commands::open_devtools,
            commands::show_main_window,
            commands::hide_panel,
            commands::quit_app,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            // The runner reports through a closure so it never has to know about Tauri.
            let sink = {
                let handle = handle.clone();
                Arc::new(move |event: RunnerEvent| match event {
                    RunnerEvent::Log { project_id, line } => {
                        let _ = handle.emit(
                            LOG_EVENT,
                            serde_json::json!({ "projectId": project_id, "line": line }),
                        );
                    }
                    RunnerEvent::Status { project_id, status } => {
                        let _ = handle.emit(
                            STATUS_EVENT,
                            serde_json::json!({ "projectId": project_id, "status": status }),
                        );
                    }
                })
            };

            let runner = Arc::new(ProcessManager::new(sink, runtime_handle.clone()));
            let loaded = config::load();
            let start_hidden = loaded.config.settings.start_hidden || autostart::launched_hidden();

            // Must happen before any window exists. A webview starts loading — and can
            // invoke a command — the moment it is created, so a window declared in
            // tauri.conf.json races this line and intermittently fails with
            // "state not managed". Creating the windows here, by hand, removes the race.
            app.manage(Arc::new(AppState::new(loaded, runner)));

            // The panel is built on first open instead of here: see `panel`.
            let main = build_main_window(app)?;

            apply_window_effects(&handle);
            tray::build(&handle)?;
            register_shortcut(&handle);

            if !start_hidden {
                let _ = main.show();
            }

            tasks::spawn_metrics_loop(handle.clone());
            tasks::spawn_health_loop(handle.clone());
            tasks::spawn_adoption_sweep(handle.clone());
            tasks::spawn_favicon_sweep(handle.clone());
            tasks::spawn_project_autostart(handle);

            Ok(())
        })
        .on_window_event(|window, event| match event {
            // A web app is an owned window with its own screen position and visibility, so
            // it has to be told every time the main window moves, resizes, or goes away.
            // `follow_owner` decides from the current state, so it does not matter which.
            WindowEvent::Moved(_) | WindowEvent::Resized(_) if window.label() == "main" => {
                embed::follow_owner(window.app_handle());
            }
            // Closing the main window sends Oracle to the tray; quitting is explicit.
            WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                let minimise = window
                    .app_handle()
                    .try_state::<Arc<AppState>>()
                    .map(|state| state.config.read().settings.minimise_to_tray)
                    .unwrap_or(true);

                if minimise {
                    api.prevent_close();
                    let _ = window.hide();
                    // Or the page stays on screen with nothing behind it.
                    embed::follow_owner(window.app_handle());
                }
            }
            // The panel behaves like a popover: losing focus dismisses it, and its webview
            // is released if it stays dismissed.
            WindowEvent::Focused(false) if window.label() == panel::LABEL => {
                panel::hide(window.app_handle());
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("Oracle failed to start")
        .run(|handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // Best effort: children are killed on the way out so no dev server is
                // left running with no way to reach it.
                embed::close_all(handle);

                if let Some(state) = handle.try_state::<Arc<AppState>>() {
                    let runner = state.runner.clone();
                    tauri::async_runtime::block_on(async move {
                        runner.stop_all().await;
                    });
                }
            }
        });
}

/// The application window.
///
/// Undecorated and transparent so the custom title bar and the glass can take over, and
/// hidden until `setup` decides whether this launch should show it.
fn build_main_window(app: &tauri::App) -> tauri::Result<tauri::WebviewWindow> {
    tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
        .title("Oracle")
        .inner_size(1240.0, 820.0)
        .min_inner_size(960.0, 640.0)
        .center()
        .resizable(true)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .visible(false)
        .build()
}

/// Applies the native backdrop so the CSS glass has something real to refract.
///
/// Mica is the Windows 11 material and the cheapest of the three; acrylic is the fallback for
/// Windows 10. Both are best-effort — a machine with transparency effects disabled in
/// accessibility settings simply gets the opaque background the CSS already defines.
///
/// Only the main window is handled here. The panel applies the same treatment when it is
/// built, which no longer happens at startup.
fn apply_window_effects(app: &tauri::AppHandle) {
    #[cfg(windows)]
    {
        use window_vibrancy::{apply_acrylic, apply_mica};

        if let Some(window) = app.get_webview_window("main") {
            if apply_mica(&window, None).is_err() {
                let _ = apply_acrylic(&window, Some((22, 20, 18, 160)));
            }
        }
    }

    #[cfg(not(windows))]
    let _ = app;
}

/// Binds the configured shortcut to the tray panel.
///
/// A shortcut another application already owns cannot be registered; that is a normal
/// conflict rather than a startup failure, so it is ignored.
fn register_shortcut(app: &tauri::AppHandle) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let accelerator = app
        .try_state::<Arc<AppState>>()
        .map(|state| state.config.read().settings.panel_shortcut.clone())
        .unwrap_or_else(|| "CmdOrCtrl+Shift+Space".to_string());

    let handle = app.clone();
    let _ = app.global_shortcut().on_shortcut(
        accelerator.as_str(),
        move |_app, _shortcut, event| {
            if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                tray::toggle_panel_centred(&handle);
            }
        },
    );
}
