//! Main window lifecycle: created on demand, destroyed on close so WebView2
//! releases its memory while the app keeps sampling in the tray.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const MAIN: &str = "main";

/// Shows the main window, creating it if it was closed (destroyed).
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let result = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("OpenMonitor Advanced")
        .inner_size(1100.0, 720.0)
        .min_inner_size(900.0, 600.0)
        .build();
    if let Err(err) = result {
        tracing::error!(%err, "cannot create the main window");
    }
}
