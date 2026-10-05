//! Foreground-window watcher: a WinEvent hook on its own thread reports the
//! foreground window and the PID that owns it (UWP games resolved through
//! their CoreWindow), and follows the moves of one tracked window with a
//! second hook filtered on its PID. `window_geometry` reads a window's client
//! area, monitor and DPI in physical pixels. No process handles are opened
//! (spec section 10).

use std::cell::{Cell, RefCell};
use std::io;
use std::sync::mpsc;
use std::thread::JoinHandle;

use oma_core::overlay::{Foreground, PxRect, WindowGeometry};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetClassNameW, GetClientRect, GetForegroundWindow, GetMessageW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, PeekMessageW,
    PostThreadMessageW, CHILDID_SELF, EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND, MSG,
    OBJID_WINDOW, PM_NOREMOVE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP, WM_QUIT,
};

/// What the watcher reports to its sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForegroundEvent {
    /// A new foreground window (and the PID of the game behind it).
    Foreground(Foreground),
    /// The tracked window moved or was resized (`EVENT_OBJECT_LOCATIONCHANGE`).
    Moved { hwnd: isize },
}

type Sink = Box<dyn Fn(ForegroundEvent) + Send>;

const FRAME_HOST_CLASS: &str = "ApplicationFrameWindow";

/// Thread message sent by `track`: `wParam` is the window (0 = stop
/// tracking), `lParam` the PID to fall back on for the location hook.
const WM_TRACK: u32 = WM_APP + 1;

thread_local! {
    /// The sink of the watcher running on this thread. The WinEvent callback
    /// has no user data, and out-of-context callbacks run on the thread that
    /// installed the hook, so a thread-local is the right route.
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    /// The tracked window (0 = none), read by the location callback.
    static TRACKED: Cell<isize> = const { Cell::new(0) };
}

/// Reports the foreground window to a sink, from its own thread, and follows
/// the moves of the window chosen with [`ForegroundWatcher::track`].
pub struct ForegroundWatcher {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl ForegroundWatcher {
    /// Starts the `oma-foreground` thread. The current foreground window is
    /// reported at once, then every change. Fails if the hook cannot be set.
    ///
    /// The thread is per-monitor DPI aware (v2), so coordinates seen there
    /// are physical pixels.
    ///
    /// The sink runs on the `oma-foreground` thread (the initial report
    /// included), so it must never block and must not own or drop the
    /// watcher. A panic in the sink is caught and swallowed.
    ///
    /// The foreground hook uses `WINEVENT_SKIPOWNPROCESS`: switching to this
    /// app's own window produces no event, so the last reported window stays;
    /// the initial report may still deliver the app's own window.
    pub fn spawn(sink: Sink) -> io::Result<ForegroundWatcher> {
        let (ready_tx, ready_rx) = mpsc::sync_channel::<io::Result<u32>>(1);
        let thread = std::thread::Builder::new()
            .name("oma-foreground".into())
            .spawn(move || run_thread(sink, ready_tx))?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(ForegroundWatcher {
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(io::Error::other("the foreground thread ended at start"))
            }
        }
    }

    /// Follows the moves of `window` (reported as [`ForegroundEvent::Moved`]),
    /// or stops following with `None`. Asynchronous: the hooks are swapped on
    /// the watcher's thread, which removes the previous location hook first.
    ///
    /// The location hook is filtered on the PID that owns the window itself
    /// (for a UWP game, the frame host's), which may differ from `pid`.
    /// A `Foreground` with `hwnd == 0` names no window and untracks, like
    /// `None`.
    pub fn track(&self, window: Option<Foreground>) {
        let (hwnd, pid) = match window {
            Some(fg) if fg.hwnd != 0 => (fg.hwnd, fg.pid),
            _ => (0, 0),
        };
        // SAFETY: plain Win32 call with value arguments; it fails harmlessly
        // when the thread (and its queue) is already gone.
        let _ = unsafe {
            PostThreadMessageW(
                self.thread_id,
                WM_TRACK,
                WPARAM(hwnd as usize),
                LPARAM(pid as isize),
            )
        };
    }
}

impl Drop for ForegroundWatcher {
    fn drop(&mut self) {
        let Some(t) = self.thread.take() else { return };
        if t.is_finished() {
            let _ = t.join();
            return;
        }
        // SAFETY: plain Win32 call with value arguments; it fails harmlessly
        // when the thread (and its queue) is already gone.
        let posted =
            unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }.is_ok();
        if posted {
            let _ = t.join();
        }
        // Otherwise the thread cannot be told to stop: detach rather than
        // risk blocking forever on the join.
    }
}

fn run_thread(sink: Sink, ready: mpsc::SyncSender<io::Result<u32>>) {
    SINK.with(|s| *s.borrow_mut() = Some(sink));
    // Physical pixels for everything this thread reads. A failure (older
    // Windows) leaves the process default, which is still usable.
    // SAFETY: plain call with a predefined context constant.
    let _ = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

    // Create the message queue before announcing the thread id, so a
    // `PostThreadMessageW` from `Drop` or `track` can never miss it.
    let mut msg = MSG::default();
    // SAFETY: `msg` is a valid MSG owned by this frame; PM_NOREMOVE only
    // peeks (and creates the queue).
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };

    // SAFETY: the callback is an `extern "system"` fn with the WINEVENTPROC
    // signature that never unwinds; no module is needed for out-of-context
    // hooks; the hook is removed below by this same thread.
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    if hook.0.is_null() {
        let err = io::Error::last_os_error();
        SINK.with(|s| *s.borrow_mut() = None);
        let _ = ready.send(Err(err));
        return;
    }
    // SAFETY: trivial query of the calling thread's id.
    let thread_id = unsafe { GetCurrentThreadId() };
    let _ = ready.send(Ok(thread_id));

    // SAFETY: trivial query of the foreground window handle.
    report_guarded(unsafe { GetForegroundWindow() });

    // The location hook of the tracked window, if any; only this thread
    // installs and removes it.
    let mut location: Option<HWINEVENTHOOK> = None;
    // SAFETY: `msg` is a valid MSG; the loop ends on WM_QUIT (0) or error (-1).
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        // WinEvent callbacks are dispatched inside GetMessageW; the only
        // thread message handled here is `WM_TRACK`.
        if msg.hwnd.0.is_null() && msg.message == WM_TRACK {
            unhook_location(&mut location);
            let hwnd = msg.wParam.0 as isize;
            if hwnd != 0 {
                location = hook_location(hwnd, msg.lParam.0 as u32);
            }
        }
    }

    unhook_location(&mut location);
    // SAFETY: `hook` came from SetWinEventHook on this thread and is removed once.
    let _ = unsafe { UnhookWinEvent(hook) };
    SINK.with(|s| *s.borrow_mut() = None);
}

/// Installs the location hook for `hwnd`, filtered on the PID that owns the
/// window (falling back to `pid` if the window is already gone).
fn hook_location(hwnd: isize, pid: u32) -> Option<HWINEVENTHOOK> {
    let owner = pid_of(HWND(hwnd as *mut _));
    let pid = if owner != 0 { owner } else { pid };
    if pid == 0 {
        return None;
    }
    // SAFETY: as for the foreground hook: a non-unwinding WINEVENTPROC, no
    // module for an out-of-context hook, removed by this same thread in
    // `unhook_location`. Own-process windows are not skipped (the PID filter
    // already narrows the events), so a test window can be tracked.
    let hook = unsafe {
        SetWinEventHook(
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_OBJECT_LOCATIONCHANGE,
            None,
            Some(on_location_event),
            pid,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.0.is_null() {
        tracing::warn!(err = %io::Error::last_os_error(), pid, "no location hook for the tracked window");
        return None;
    }
    TRACKED.with(|t| t.set(hwnd));
    Some(hook)
}

fn unhook_location(location: &mut Option<HWINEVENTHOOK>) {
    TRACKED.with(|t| t.set(0));
    if let Some(hook) = location.take() {
        // SAFETY: `hook` came from SetWinEventHook on this thread and is
        // removed once (`take` clears the slot).
        let _ = unsafe { UnhookWinEvent(hook) };
    }
}

unsafe extern "system" fn on_foreground_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    report_guarded(hwnd);
}

unsafe extern "system" fn on_location_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    let hwnd = hwnd.0 as isize;
    let tracked = TRACKED.with(Cell::get);
    if location_event_matches(hwnd, object, child, tracked) {
        let _ = std::panic::catch_unwind(|| emit(ForegroundEvent::Moved { hwnd }));
    }
}

/// True for a location change of the tracked window itself: not of one of
/// its child objects (caret, cursor, scroll bars) nor of another window.
pub(crate) fn location_event_matches(
    event_hwnd: isize,
    id_object: i32,
    id_child: i32,
    tracked: isize,
) -> bool {
    tracked != 0
        && event_hwnd == tracked
        && id_object == OBJID_WINDOW.0
        && id_child == CHILDID_SELF as i32
}

/// `report` behind `catch_unwind`: a panicking sink must neither unwind across
/// the FFI boundary nor kill the thread before it removes the hooks.
fn report_guarded(hwnd: HWND) {
    let _ = std::panic::catch_unwind(|| report(hwnd));
}

/// Resolves the window to a PID and hands it to the sink; null windows and
/// PID 0 are skipped.
fn report(hwnd: HWND) {
    if hwnd.0.is_null() {
        return;
    }
    let Some(pid) = window_pid(hwnd) else { return };
    emit(ForegroundEvent::Foreground(Foreground {
        pid,
        hwnd: hwnd.0 as isize,
    }));
}

fn emit(event: ForegroundEvent) {
    SINK.with(|s| {
        if let Ok(guard) = s.try_borrow() {
            if let Some(sink) = guard.as_ref() {
                sink(event);
            }
        }
    });
}

/// Client area (screen coordinates), monitor, DPI and state of a window, in
/// physical pixels whatever the calling thread's DPI awareness. `None` if
/// the window no longer exists.
pub fn window_geometry(hwnd: isize) -> Option<WindowGeometry> {
    if hwnd == 0 {
        return None;
    }
    let hwnd = HWND(hwnd as *mut _);
    // Per-monitor v2 for the duration of the queries, then restored.
    // SAFETY: plain call with a predefined context constant; the previous
    // context it returns is restored below.
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let geometry = read_geometry(hwnd);
    if !previous.0.is_null() {
        // SAFETY: `previous` is the context this thread had a moment ago.
        let _ = unsafe { SetThreadDpiAwarenessContext(previous) };
    }
    geometry
}

fn read_geometry(hwnd: HWND) -> Option<WindowGeometry> {
    // SAFETY: plain query; any handle value is accepted.
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return None;
    }
    // The client rect of a DPI-virtualized window of another process is in
    // its own logical units: map both corners to the screen under PMv2 and
    // take the size from the mapped points, never from GetClientRect alone.
    let mut client = RECT::default();
    // SAFETY: `client` is a valid out pointer; a stale handle fails cleanly.
    unsafe { GetClientRect(hwnd, &mut client) }.ok()?;
    let mut top_left = POINT { x: 0, y: 0 };
    let mut bottom_right = POINT {
        x: client.right,
        y: client.bottom,
    };
    // SAFETY: both points are valid in/out pointers for the calls.
    let mapped = unsafe {
        ClientToScreen(hwnd, &mut top_left).as_bool()
            && ClientToScreen(hwnd, &mut bottom_right).as_bool()
    };
    if !mapped {
        return None;
    }
    // SAFETY: plain call; MONITOR_DEFAULTTONEAREST always yields a monitor.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid MONITORINFO with `cbSize` set.
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    // The monitor's effective DPI, not GetDpiForWindow: that one answers 96
    // for a DPI-unaware game (the system DPI for a system-aware one), which
    // would shrink the overlay on a scaled monitor.
    let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
    // SAFETY: `monitor` came from MonitorFromWindow; both out pointers are
    // valid locals.
    let monitor_dpi =
        unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }
            .ok()
            .map(|()| dpi_x);
    // SAFETY: plain queries on a window handle.
    let (minimized, visible) =
        unsafe { (IsIconic(hwnd).as_bool(), IsWindowVisible(hwnd).as_bool()) };
    Some(WindowGeometry {
        client: client_px(top_left, bottom_right),
        monitor: rect_to_px(info.rcMonitor),
        dpi: dpi_or_default(monitor_dpi),
        minimized,
        visible,
    })
}

/// The monitor DPI, or 96 when the query failed or answered 0.
pub(crate) fn dpi_or_default(dpi: Option<u32>) -> u32 {
    dpi.filter(|d| *d != 0).unwrap_or(96)
}

/// The client area from its two corners mapped to the screen; a degenerate
/// mapping gives an empty size, never a negative one.
pub(crate) fn client_px(top_left: POINT, bottom_right: POINT) -> PxRect {
    PxRect {
        x: top_left.x,
        y: top_left.y,
        w: (bottom_right.x - top_left.x).max(0),
        h: (bottom_right.y - top_left.y).max(0),
    }
}

fn rect_to_px(r: RECT) -> PxRect {
    PxRect {
        x: r.left,
        y: r.top,
        w: r.right - r.left,
        h: r.bottom - r.top,
    }
}

fn window_pid(hwnd: HWND) -> Option<u32> {
    let own = pid_of(hwnd);
    let mut class = [0u16; 64];
    // SAFETY: the slice is valid and writable for its length, and the returned
    // count is bounded by it.
    let len = unsafe { GetClassNameW(hwnd, &mut class) };
    let class = String::from_utf16_lossy(&class[..len.max(0) as usize]);
    let core = if class == FRAME_HOST_CLASS {
        // SAFETY: plain call with a window handle and static wide strings.
        unsafe {
            FindWindowExW(
                Some(hwnd),
                None,
                w!("Windows.UI.Core.CoreWindow"),
                PCWSTR::null(),
            )
        }
        .ok()
        .and_then(|child| Some(pid_of(child)).filter(|p| *p != 0))
    } else {
        None
    };
    Some(foreground_pid(own, &class, core)).filter(|p| *p != 0)
}

fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out pointer for the call.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

/// Pure part of the resolution: a UWP frame host (`ApplicationFrameWindow`)
/// belongs to the shell, the game is the process of its `CoreWindow` child.
pub(crate) fn foreground_pid(
    window_pid: u32,
    window_class: &str,
    core_window_pid: Option<u32>,
) -> u32 {
    match core_window_pid {
        Some(pid) if window_class == FRAME_HOST_CLASS => pid,
        _ => window_pid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use windows::Win32::UI::HiDpi::{
        SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn watcher_reports_the_current_foreground_at_start() {
        let (tx, rx) = mpsc::channel();
        let watcher = ForegroundWatcher::spawn(Box::new(move |event| {
            let _ = tx.send(event);
        }))
        .expect("hook installed");
        let event = rx.recv_timeout(Duration::from_secs(2)).expect("an event");
        let ForegroundEvent::Foreground(fg) = event else {
            panic!("expected a foreground event, got {event:?}");
        };
        assert_ne!(fg.pid, 0);
        assert_ne!(fg.hwnd, 0);
        let started = Instant::now();
        drop(watcher);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn geometry_of_the_foreground_window() {
        // SAFETY: trivial query of the foreground window handle.
        let hwnd = unsafe { GetForegroundWindow() };
        assert!(!hwnd.0.is_null(), "a foreground window");
        let geo = window_geometry(hwnd.0 as isize).expect("geometry");
        assert!(geo.monitor.w > 0 && geo.monitor.h > 0, "{geo:?}");
        if !geo.minimized {
            assert!(geo.client.w > 0 && geo.client.h > 0, "{geo:?}");
        }
        assert!(geo.dpi >= 96, "{geo:?}");
        assert_eq!(window_geometry(0), None);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn track_reports_moves_of_a_test_window() {
        // The test window lives on this thread; physical pixels on both sides.
        // The guard restores the DPI context and destroys the window even if
        // an assert fails.
        let mut guard = TestWindow {
            // SAFETY: plain call with a predefined context constant; the
            // previous context is restored by the guard.
            previous: unsafe {
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
            },
            window: None,
        };
        // SAFETY: predefined "STATIC" class, static strings, no parent, menu
        // or creation data; the guard destroys the window.
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                w!("oma-foreground-test"),
                WS_POPUP,
                -20000,
                -20000,
                64,
                48,
                None,
                None,
                None,
                None,
            )
        }
        .expect("test window");
        guard.window = Some(window);
        let hwnd = window.0 as isize;
        let (tx, rx) = mpsc::channel();
        let watcher = ForegroundWatcher::spawn(Box::new(move |event| {
            let _ = tx.send(event);
        }))
        .expect("hook installed");
        watcher.track(Some(Foreground {
            pid: std::process::id(),
            hwnd,
        }));
        let move_to = |x: i32| {
            // SAFETY: `window` is a live window owned by this thread.
            unsafe {
                SetWindowPos(
                    window,
                    None,
                    x,
                    -20000,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            }
            .expect("moved");
        };
        let moved = |rx: &mpsc::Receiver<ForegroundEvent>, wait: Duration| {
            let until = Instant::now() + wait;
            while let Some(left) = until.checked_duration_since(Instant::now()) {
                match rx.recv_timeout(left) {
                    Ok(ForegroundEvent::Moved { hwnd: h }) if h == hwnd => return true,
                    Ok(_) => {}
                    Err(_) => return false,
                }
            }
            false
        };
        // `track` is asynchronous: retry until the hook is in place.
        let mut seen = false;
        for i in 0..20 {
            move_to(-20000 + 10 * (i + 1));
            if moved(&rx, Duration::from_millis(100)) {
                seen = true;
                break;
            }
        }
        assert!(seen, "a Moved event for the tracked window");
        let geo = window_geometry(hwnd).expect("geometry of the test window");
        assert_eq!((geo.client.w, geo.client.h), (64, 48));
        assert!(!geo.visible);

        watcher.track(None);
        std::thread::sleep(Duration::from_millis(200));
        while rx.try_recv().is_ok() {}
        move_to(-19000);
        assert!(!moved(&rx, Duration::from_millis(300)), "untracked");

        drop(watcher);
        guard.destroy_window();
        assert_eq!(window_geometry(hwnd), None);
    }

    /// Restores the test thread's DPI context and destroys its test window,
    /// on success and on a failed assert alike.
    struct TestWindow {
        previous: DPI_AWARENESS_CONTEXT,
        window: Option<HWND>,
    }

    impl TestWindow {
        fn destroy_window(&mut self) {
            if let Some(window) = self.window.take() {
                // SAFETY: `window` was created by this thread and `take`
                // ensures it is destroyed once.
                let _ = unsafe { DestroyWindow(window) };
            }
        }
    }

    impl Drop for TestWindow {
        fn drop(&mut self) {
            self.destroy_window();
            if !self.previous.0.is_null() {
                // SAFETY: `previous` is the context this thread had before
                // the test switched it.
                let _ = unsafe { SetThreadDpiAwarenessContext(self.previous) };
            }
        }
    }

    #[test]
    fn location_event_for_tracked_window_matches() {
        assert!(location_event_matches(0x1234, OBJID_WINDOW.0, 0, 0x1234));
    }

    #[test]
    fn child_object_events_are_ignored() {
        // A caret or cursor of the window, or a child element of it.
        assert!(!location_event_matches(0x1234, -8, 0, 0x1234));
        assert!(!location_event_matches(0x1234, OBJID_WINDOW.0, 3, 0x1234));
        // Another window of the same process, or nothing tracked.
        assert!(!location_event_matches(0x5678, OBJID_WINDOW.0, 0, 0x1234));
        assert!(!location_event_matches(0, OBJID_WINDOW.0, 0, 0));
    }

    #[test]
    fn monitor_dpi_falls_back_to_96() {
        assert_eq!(dpi_or_default(Some(144)), 144);
        assert_eq!(dpi_or_default(Some(0)), 96);
        assert_eq!(dpi_or_default(None), 96);
    }

    #[test]
    fn client_rect_spans_both_mapped_corners() {
        let tl = POINT { x: -100, y: 50 };
        let br = POINT { x: 860, y: 590 };
        assert_eq!(
            client_px(tl, br),
            PxRect {
                x: -100,
                y: 50,
                w: 960,
                h: 540
            }
        );
        // A degenerate mapping never yields a negative size.
        assert_eq!(client_px(br, tl).w, 0);
        assert_eq!(client_px(br, tl).h, 0);
    }

    #[test]
    fn uwp_frame_host_resolves_to_the_core_window_pid() {
        assert_eq!(
            foreground_pid(100, "ApplicationFrameWindow", Some(200)),
            200
        );
    }

    #[test]
    fn a_normal_window_keeps_its_pid() {
        assert_eq!(foreground_pid(100, "UnrealWindow", Some(200)), 100);
        assert_eq!(foreground_pid(100, "UnrealWindow", None), 100);
    }

    #[test]
    fn frame_host_without_core_window_keeps_its_pid() {
        assert_eq!(foreground_pid(100, "ApplicationFrameWindow", None), 100);
    }
}
