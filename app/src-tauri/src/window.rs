//! Main window lifecycle: created on demand, destroyed on close so WebView2
//! releases its memory while the app keeps sampling in the tray.

use std::sync::{Mutex, PoisonError};

use oma_core::settings::ViewKind;
use serde::{Serialize, Serializer};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const MAIN: &str = "main";
/// Sent to an already open window; the payload is a [`NavigationTarget`].
pub const EVENT_NAVIGATE: &str = "oma:navigate";

/// Where a tray item or a toast sends the window: a view and, for a toast,
/// the device whose page opens in the Advanced view. A Tauri payload, not a
/// protocol type: `deviceId` is left out when absent, as the UI expects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationTarget {
    #[serde(serialize_with = "view_name")]
    pub view: ViewKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

fn view_name<S: Serializer>(view: &ViewKind, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(view.as_str())
}

impl NavigationTarget {
    pub fn view(view: ViewKind) -> Self {
        Self {
            view,
            device_id: None,
        }
    }

    /// The Advanced view on the page of `device_id`.
    pub fn device(device_id: &str) -> Self {
        Self {
            view: ViewKind::Advanced,
            device_id: Some(device_id.to_owned()),
        }
    }
}

/// The latest navigation request, kept until the UI takes it
/// (`take_pending_view`). An open window is also told with
/// [`EVENT_NAVIGATE`], and takes the request as acknowledgment; a window
/// that is still loading may miss the event, so the request stays for it.
#[derive(Default)]
pub struct NavState(Mutex<Option<NavigationTarget>>);

impl NavState {
    /// Records `target`, replacing any request not taken yet.
    pub fn request(&self, target: NavigationTarget) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(target);
    }

    /// The pending request, once: the next call returns `None`.
    pub fn take(&self) -> Option<NavigationTarget> {
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
    match result {
        Ok(window) => {
            // A capture box that had focus cannot keep the hotkeys suspended.
            let app = app.clone();
            window.on_window_event(move |event| {
                crate::hotkeys::resume_on_window_event(&app, event);
            });
        }
        Err(err) => tracing::error!(%err, "cannot create the main window"),
    }
}

/// Shows the main window on `view`.
pub fn show_main_on(app: &AppHandle, view: ViewKind) {
    navigate(app, NavigationTarget::view(view));
}

/// Shows the Advanced view on the page of `device_id` (a clicked toast); the
/// UI falls back to the Advanced view with `health.deviceGone` when the
/// device no longer exists.
pub fn show_device(app: &AppHandle, device_id: &str) {
    navigate(app, NavigationTarget::device(device_id));
}

/// Leaves `target` pending, tells an open window at once, and shows it.
fn navigate(app: &AppHandle, target: NavigationTarget) {
    app.state::<NavState>().request(target.clone());
    if app.get_webview_window(MAIN).is_some() {
        let _ = app.emit(EVENT_NAVIGATE, &target);
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
        nav.request(NavigationTarget::view(ViewKind::Advanced));
        assert_eq!(nav.take(), Some(NavigationTarget::view(ViewKind::Advanced)));
        assert_eq!(nav.take(), None);
    }

    #[test]
    fn pending_device_survives_webview_startup() {
        // The window exists but its page may not listen yet: the request stays
        // until the UI takes it.
        let nav = NavState::default();
        nav.request(NavigationTarget::device("gpu/0"));
        assert_eq!(nav.take(), Some(NavigationTarget::device("gpu/0")));
        assert_eq!(nav.take(), None);
    }

    #[test]
    fn the_latest_request_wins() {
        let nav = NavState::default();
        nav.request(NavigationTarget::device("gpu/0"));
        nav.request(NavigationTarget::view(ViewKind::Simple));
        assert_eq!(nav.take(), Some(NavigationTarget::view(ViewKind::Simple)));
    }

    #[test]
    fn navigation_target_serializes_for_the_ui() {
        let json = |target| serde_json::to_string(&target).unwrap();
        assert_eq!(
            json(NavigationTarget::view(ViewKind::Simple)),
            r#"{"view":"simple"}"#
        );
        assert_eq!(
            json(NavigationTarget::device("storage/a\"b")),
            r#"{"view":"advanced","deviceId":"storage/a\"b"}"#
        );
    }
}
