//! Window lifecycle: the main window and the overlay editor (M7d) are
//! created on demand and destroyed on close, so WebView2 releases its memory
//! while the app keeps sampling in the tray. Also the app's exit from the
//! tray, which asks the editor first when it holds unsaved changes (DD13).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use oma_core::overlay::PxRect;
use oma_core::settings::overlay::WindowBounds;
use oma_core::settings::ViewKind;
use serde::{Serialize, Serializer};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl,
    WebviewWindowBuilder, WindowEvent,
};

use crate::i18n::t;
use crate::settings::SettingsStore;
use crate::tray::language_for;

pub const MAIN: &str = "main";
/// The overlay editor's window.
pub const EDITOR: &str = "overlay-editor";
/// Sent to the editor when the tray's «Quit» waits for its answer; the
/// editor calls `app_quit_confirmed` after «Save» or «Discard».
pub const EVENT_EDITOR_QUIT: &str = "overlay-editor-quit";
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
    /// The settings section to open (`about`), for the update toast.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_section: Option<String>,
    /// The Performance view's page (M8a1), for the tray and the stress toasts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance: Option<PerformanceNav>,
}

/// A page of the Performance view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PerformancePage {
    /// The test in progress.
    #[allow(dead_code)] // removed in A21
    Run,
    /// A saved session's result.
    Result,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceNav {
    pub page: PerformancePage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

impl PerformanceNav {
    #[allow(dead_code)] // removed in A21
    pub fn run() -> Self {
        Self {
            page: PerformancePage::Run,
            session_id: None,
        }
    }

    pub fn result(session_id: &str) -> Self {
        Self {
            page: PerformancePage::Result,
            session_id: Some(session_id.to_owned()),
        }
    }
}

fn view_name<S: Serializer>(view: &ViewKind, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(view.as_str())
}

impl NavigationTarget {
    pub fn view(view: ViewKind) -> Self {
        Self {
            view,
            device_id: None,
            settings_section: None,
            performance: None,
        }
    }

    /// The Advanced view on the page of `device_id`.
    pub fn device(device_id: &str) -> Self {
        Self {
            view: ViewKind::Advanced,
            device_id: Some(device_id.to_owned()),
            settings_section: None,
            performance: None,
        }
    }

    /// Settings › About, with `view` as the view Back returns to.
    pub fn about(view: ViewKind) -> Self {
        Self {
            view,
            device_id: None,
            settings_section: Some("about".to_owned()),
            performance: None,
        }
    }

    /// A page of the Performance view, with `view` as the view it returns to.
    pub fn performance(view: ViewKind, nav: PerformanceNav) -> Self {
        Self {
            view,
            device_id: None,
            settings_section: None,
            performance: Some(nav),
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

/// The open editor's state: whether it holds unsaved changes, and where its
/// window is (saved once, when it closes, DD12).
#[derive(Default)]
pub struct EditorState {
    dirty: AtomicBool,
    bounds: Mutex<Option<WindowBounds>>,
}

impl EditorState {
    fn update_bounds(&self, change: impl FnOnce(&mut WindowBounds)) {
        if let Some(bounds) = self
            .bounds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            change(bounds);
        }
    }
}

/// Whether `bounds` overlaps one of the `monitors` (an unplugged monitor
/// would leave the editor out of sight).
pub(crate) fn bounds_visible(bounds: WindowBounds, monitors: &[PxRect]) -> bool {
    let (left, top) = (i64::from(bounds.x), i64::from(bounds.y));
    let (right, bottom) = (
        left + i64::from(bounds.width),
        top + i64::from(bounds.height),
    );
    monitors.iter().any(|m| {
        let (m_left, m_top) = (i64::from(m.x), i64::from(m.y));
        left < m_left + i64::from(m.w)
            && m_left < right
            && top < m_top + i64::from(m.h)
            && m_top < bottom
    })
}

/// Whether the main window or the editor is open: someone listens to the
/// app's events.
pub fn any_open(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN).is_some() || app.get_webview_window(EDITOR).is_some()
}

/// Tells the overlay's controller that the editor opened or closed.
fn tell_overlay(app: &AppHandle, open: bool) {
    #[cfg(windows)]
    if let Some(overlay) = app.try_state::<crate::overlay::runner::OverlayHandle>() {
        overlay.editor_open(open);
    }
    #[cfg(not(windows))]
    let _ = (app, open);
}

/// Shows the overlay editor, creating it if it is closed: where it was last
/// closed if that is still on a monitor, centered otherwise.
pub fn show_editor(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(EDITOR) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let store = app.state::<Arc<SettingsStore>>();
    let settings = store.snapshot();
    let lang = language_for(settings.general.language);
    let monitors: Vec<PxRect> = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| PxRect {
            x: m.position().x,
            y: m.position().y,
            w: i32::try_from(m.size().width).unwrap_or(i32::MAX),
            h: i32::try_from(m.size().height).unwrap_or(i32::MAX),
        })
        .collect();
    let saved = settings
        .overlay
        .editor_bounds
        .filter(|b| bounds_visible(*b, &monitors));
    let mut builder = WebviewWindowBuilder::new(
        app,
        EDITOR,
        WebviewUrl::App("index.html?window=overlay-editor".into()),
    )
    .title(t(lang, "editor.title", &[]))
    .inner_size(1280.0, 800.0)
    .min_inner_size(1100.0, 700.0)
    .visible(false);
    if saved.is_none() {
        builder = builder.center();
    }
    let window = match builder.build() {
        Ok(window) => window,
        Err(err) => {
            tracing::error!(%err, "cannot create the overlay editor window");
            return;
        }
    };
    if let Some(b) = saved {
        let _ = window.set_position(PhysicalPosition::new(b.x, b.y));
        let _ = window.set_size(PhysicalSize::new(b.width, b.height));
    }
    let state = app.state::<EditorState>();
    state.dirty.store(false, Ordering::Release);
    *state.bounds.lock().unwrap_or_else(PoisonError::into_inner) =
        match (window.outer_position(), window.inner_size()) {
            (Ok(p), Ok(s)) => Some(WindowBounds {
                x: p.x,
                y: p.y,
                width: s.width,
                height: s.height,
            }),
            _ => None,
        };
    let handle = app.clone();
    window.on_window_event(move |event| {
        let state = handle.state::<EditorState>();
        // A maximized window's bounds are the monitor's: the restored ones
        // are kept.
        let maximized = || {
            handle
                .get_webview_window(EDITOR)
                .and_then(|w| w.is_maximized().ok())
                .unwrap_or(false)
        };
        match event {
            // A minimized window reports -32000 and a zero size: not kept.
            WindowEvent::Moved(p) if p.x > -32_000 && p.y > -32_000 && !maximized() => {
                state.update_bounds(|b| (b.x, b.y) = (p.x, p.y));
            }
            WindowEvent::Resized(s) if s.width > 0 && s.height > 0 && !maximized() => {
                state.update_bounds(|b| (b.width, b.height) = (s.width, s.height));
            }
            WindowEvent::Destroyed => {
                state.dirty.store(false, Ordering::Release);
                let bounds = state
                    .bounds
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
                    .filter(WindowBounds::is_valid);
                if bounds.is_some() {
                    handle
                        .state::<Arc<SettingsStore>>()
                        .update_with(|s| s.overlay.editor_bounds = bounds);
                }
                // Closing the editor also ends the preview.
                tell_overlay(&handle, false);
            }
            _ => {}
        }
    });
    let _ = window.show();
    let _ = window.set_focus();
    tell_overlay(app, true);
}

/// Who asks the app to exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitSource {
    /// «Quit» in the tray menu.
    Tray,
    /// `oma-app.exe --quit` (the installer).
    Flag,
}

/// What a quit request does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitAction {
    Exit,
    /// Show the editor and let it ask: save, discard or cancel.
    AskEditor,
}

/// The tray asks first while the editor holds unsaved changes; `--quit`
/// never asks, so the installer can always close the app (DD13).
pub fn quit_action(source: QuitSource, editor_open: bool, editor_dirty: bool) -> QuitAction {
    if source == QuitSource::Tray && editor_open && editor_dirty {
        QuitAction::AskEditor
    } else {
        QuitAction::Exit
    }
}

/// Exits the app, or asks the editor first (see [`quit_action`]).
pub fn quit(app: &AppHandle, source: QuitSource) {
    let open = app.get_webview_window(EDITOR).is_some();
    let dirty = app
        .try_state::<EditorState>()
        .is_some_and(|s| s.dirty.load(Ordering::Acquire));
    match quit_action(source, open, dirty) {
        QuitAction::Exit => app.exit(0),
        QuitAction::AskEditor => {
            show_editor(app);
            let _ = app.emit_to(EDITOR, EVENT_EDITOR_QUIT, ());
        }
    }
}

/// Opens the overlay editor. Async: a window created in a synchronous
/// command deadlocks on Windows.
#[tauri::command]
pub async fn open_overlay_editor(app: AppHandle) {
    show_editor(&app);
}

/// The editor holds unsaved changes (or no longer does).
#[tauri::command]
pub fn overlay_editor_dirty(state: State<'_, EditorState>, dirty: bool) {
    state.dirty.store(dirty, Ordering::Release);
}

/// The editor answered the tray's «Quit» with «Save» or «Discard».
#[tauri::command]
pub fn app_quit_confirmed(app: AppHandle) {
    app.exit(0);
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

/// Shows Settings › About (a clicked update toast); Back returns to the last
/// view, Simple when none is remembered.
pub fn show_about(app: &AppHandle) {
    navigate(app, NavigationTarget::about(last_view(app)));
}

/// Shows a page of the Performance view (a clicked stress toast, the tray).
pub fn show_performance(app: &AppHandle, nav: PerformanceNav) {
    navigate(app, NavigationTarget::performance(last_view(app), nav));
}

/// The last view, Simple when none is remembered.
fn last_view(app: &AppHandle) -> ViewKind {
    app.state::<Arc<SettingsStore>>()
        .snapshot()
        .view
        .last
        .unwrap_or(ViewKind::Simple)
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

    #[test]
    fn bounds_visible_needs_an_intersecting_monitor() {
        let bounds = |x, y| WindowBounds {
            x,
            y,
            width: 1280,
            height: 800,
        };
        let primary = PxRect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        let left = PxRect {
            x: -2560,
            y: 0,
            w: 2560,
            h: 1440,
        };
        assert!(bounds_visible(bounds(100, 100), &[primary]));
        // Partly off the edge still intersects.
        assert!(bounds_visible(bounds(1800, 1000), &[primary]));
        // On a monitor that was unplugged.
        assert!(bounds_visible(bounds(-2000, 200), &[primary, left]));
        assert!(!bounds_visible(bounds(-2000, 200), &[primary]));
        // Touching an edge is not intersecting.
        assert!(!bounds_visible(bounds(1920, 0), &[primary]));
        assert!(!bounds_visible(bounds(0, 0), &[]));
    }

    #[test]
    fn tray_quit_with_a_dirty_editor_asks_first() {
        assert_eq!(
            quit_action(QuitSource::Tray, true, true),
            QuitAction::AskEditor
        );
        assert_eq!(quit_action(QuitSource::Tray, true, false), QuitAction::Exit);
        assert_eq!(quit_action(QuitSource::Tray, false, true), QuitAction::Exit);
        assert_eq!(
            quit_action(QuitSource::Tray, false, false),
            QuitAction::Exit
        );
    }

    #[test]
    fn quit_flag_never_asks() {
        for (open, dirty) in [(true, true), (true, false), (false, true), (false, false)] {
            assert_eq!(
                quit_action(QuitSource::Flag, open, dirty),
                QuitAction::Exit,
                "{open} {dirty}"
            );
        }
    }

    #[test]
    fn navigation_target_serializes_performance() {
        let json = |nav| {
            serde_json::to_string(&NavigationTarget::performance(ViewKind::Simple, nav)).unwrap()
        };
        assert_eq!(
            json(PerformanceNav::run()),
            r#"{"view":"simple","performance":{"page":"run"}}"#
        );
        assert_eq!(
            json(PerformanceNav::result(
                "0b9f6c1e-7d2a-4c53-9a1e-3f5d8e2b7a10"
            )),
            r#"{"view":"simple","performance":{"page":"result","sessionId":"0b9f6c1e-7d2a-4c53-9a1e-3f5d8e2b7a10"}}"#
        );
    }

    #[test]
    fn about_target_serializes_section() {
        assert_eq!(
            serde_json::to_string(&NavigationTarget::about(ViewKind::Simple)).unwrap(),
            r#"{"view":"simple","settingsSection":"about"}"#
        );
    }
}
