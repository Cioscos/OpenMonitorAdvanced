//! Main window lifecycle: created on demand, destroyed on close so WebView2
//! releases its memory while the app keeps sampling in the tray.

use std::sync::{Mutex, PoisonError};

use oma_core::settings::ViewKind;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const MAIN: &str = "main";
/// Sent to an already open window; the payload is `"simple"` or `"advanced"`.
pub const EVENT_NAVIGATE: &str = "oma:navigate";

/// The view a tray item asked for while the window did not exist yet. The UI
/// reads it once at mount (`take_pending_view`); an open window is told with
/// [`EVENT_NAVIGATE`] instead.
#[derive(Default)]
pub struct NavState(Mutex<Option<ViewKind>>);

impl NavState {
    /// Records a request for `view`. Returns `true` when the caller must emit
    /// [`EVENT_NAVIGATE`] (the window is open); otherwise the view stays
    /// pending for the window about to be created.
    pub fn request(&self, view: ViewKind, window_open: bool) -> bool {
        let mut pending = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        *pending = if window_open { None } else { Some(view) };
        window_open
    }

    /// The pending view, once: the next call returns `None`.
    pub fn take(&self) -> Option<ViewKind> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

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

/// Shows the main window on `view`: the view is left pending for a window that
/// has to be created, or sent as an event to one that is already open.
pub fn show_main_on(app: &AppHandle, view: ViewKind) {
    let window_open = app.get_webview_window(MAIN).is_some();
    if app.state::<NavState>().request(view, window_open) {
        let _ = app.emit(EVENT_NAVIGATE, view.as_str());
    }
    show_main(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_view_is_taken_once() {
        let nav = NavState::default();
        assert_eq!(nav.take(), None);
        assert!(!nav.request(ViewKind::Advanced, false));
        assert_eq!(nav.take(), Some(ViewKind::Advanced));
        assert_eq!(nav.take(), None);
    }

    #[test]
    fn an_open_window_gets_an_event_instead_of_a_pending_view() {
        let nav = NavState::default();
        assert!(nav.request(ViewKind::Simple, true));
        // Nothing stays behind for the next window to pick up by mistake.
        assert_eq!(nav.take(), None);
    }

    #[test]
    fn the_latest_request_wins() {
        let nav = NavState::default();
        nav.request(ViewKind::Simple, false);
        nav.request(ViewKind::Advanced, false);
        assert_eq!(nav.take(), Some(ViewKind::Advanced));
    }
}
