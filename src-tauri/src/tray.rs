//! The tray icon and the floating panel it opens.
//!
//! The panel is a real window, not a menu. It is positioned by hand next to the tray icon
//! because the OS offers no anchoring for this: the click event carries the icon's rectangle
//! in physical pixels, which is enough to place the window above it and keep it on screen.

use crate::state::AppState;
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, PhysicalPosition, Runtime, WebviewWindow,
};

/// Gap between the panel and the edge of the screen or the tray icon.
const MARGIN: f64 = 12.0;

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Oracle", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &separator, &quit])?;

    // The application icon sits on the artwork's off-white ground, which reads as a white
    // tile in the notification area. This one has that ground stripped to alpha.
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-32.png"))?;

    TrayIconBuilder::with_id("oracle")
        .icon(icon)
        .icon_as_template(false)
        .tooltip("Oracle")
        .menu(&menu)
        // Left click is reserved for the panel; without this the menu would steal it.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "quit" => quit_app(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                toggle_panel(tray.app_handle(), rect.position, rect.size);
            }
        })
        .build(app)?;

    Ok(())
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    if let Some(panel) = app.get_webview_window("panel") {
        let _ = panel.hide();
    }
}

fn quit_app<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();

    // Killing children is async, and the app must not exit before it finishes or the dev
    // servers Oracle started would survive it.
    tauri::async_runtime::spawn(async move {
        if let Some(state) = handle.try_state::<Arc<AppState>>() {
            state.runner.stop_all().await;
        }
        handle.exit(0);
    });
}

/// Shows the panel if it is hidden, hides it if it is already up.
pub fn toggle_panel<R: Runtime>(
    app: &AppHandle<R>,
    anchor: tauri::Position,
    size: tauri::Size,
) {
    let Some(panel) = app.get_webview_window("panel") else {
        return;
    };

    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        return;
    }

    position_near(&panel, anchor, size);
    let _ = panel.show();
    let _ = panel.set_focus();
}

/// Opens the panel centred on the primary monitor, for the keyboard shortcut where there is
/// no tray rectangle to anchor to.
pub fn toggle_panel_centred<R: Runtime>(app: &AppHandle<R>) {
    let Some(panel) = app.get_webview_window("panel") else {
        return;
    };

    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        return;
    }

    if let Ok(Some(monitor)) = panel.primary_monitor() {
        if let Ok(size) = panel.outer_size() {
            let area = monitor.size();
            let scale = monitor.scale_factor();
            let position = monitor.position();

            // Bottom-right, where the tray lives, rather than dead centre.
            let x = position.x as f64 + area.width as f64 - size.width as f64 - MARGIN * scale;
            let y = position.y as f64 + area.height as f64 - size.height as f64 - 48.0 * scale;

            let _ = panel.set_position(PhysicalPosition::new(x, y));
        }
    }

    let _ = panel.show();
    let _ = panel.set_focus();
}

/// Places the panel just above the tray icon, clamped to the monitor it sits on.
fn position_near<R: Runtime>(
    panel: &WebviewWindow<R>,
    anchor: tauri::Position,
    anchor_size: tauri::Size,
) {
    let Ok(panel_size) = panel.outer_size() else {
        return;
    };

    let scale = panel.scale_factor().unwrap_or(1.0);
    let anchor = anchor.to_physical::<f64>(scale);
    let anchor_size = anchor_size.to_physical::<f64>(scale);

    // Centre the panel on the icon horizontally, and sit it above the taskbar.
    let mut x = anchor.x + anchor_size.width / 2.0 - panel_size.width as f64 / 2.0;
    let mut y = anchor.y - panel_size.height as f64 - MARGIN;

    if let Ok(Some(monitor)) = panel.monitor_from_point(anchor.x, anchor.y) {
        let area = monitor.size();
        let origin = monitor.position();

        let min_x = origin.x as f64 + MARGIN;
        let max_x = origin.x as f64 + area.width as f64 - panel_size.width as f64 - MARGIN;
        x = x.clamp(min_x, max_x.max(min_x));

        // A taskbar docked at the top leaves no room above the icon; drop below it instead.
        let min_y = origin.y as f64 + MARGIN;
        if y < min_y {
            y = anchor.y + anchor_size.height + MARGIN;
        }
    }

    let _ = panel.set_position(PhysicalPosition::new(x, y));
}
