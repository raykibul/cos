//! The cos desktop app: one window showing the web UI, a menu bar icon, and
//! the commands that let the UI reach the daemon through `cos-bridge`. The
//! contract is `docs/desktop.md`.

mod commands;
mod tray;

use cos_bridge::Bridge;
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

use crate::commands::AppState;

/// The label of the one window, from `tauri.conf.json`.
const MAIN_WINDOW: &str = "main";

fn main() {
    let bridge = Bridge::from_env().unwrap_or_else(|| Bridge::new("/.cos/cos.sock"));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::new(bridge))
        .invoke_handler(tauri::generate_handler![
            commands::daemon_info,
            commands::daemon_request,
            commands::follow_events,
            commands::attach_terminal,
            commands::close_stream,
            commands::set_waiting_count,
            commands::notify,
            commands::reveal_path,
            commands::open_in_terminal,
        ])
        .setup(|app| {
            let tray = tray::Tray::create(app.handle())?;
            app.manage(tray);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing hides the window; the app stays in the menu bar.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .on_page_load(|webview, payload| {
            // A reload leaves the old page's streams with nobody listening.
            if payload.event() == PageLoadEvent::Started {
                webview.state::<AppState>().close_all_streams();
            }
        })
        .build(tauri::generate_context!())
        .expect("could not start cos");

    app.run(|app, event| {
        // Clicking the Dock icon brings the window back.
        if let RunEvent::Reopen { .. } = event {
            show_main_window(app);
        }
    });
}

/// Shows the main window, unminimized and focused.
pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
