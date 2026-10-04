//! Foreground-window watcher: a WinEvent hook on its own thread reports the
//! PID that owns the foreground window (UWP games resolved through their
//! CoreWindow). No process handles are opened (spec section 10).

use std::cell::RefCell;
use std::io;
use std::sync::mpsc;
use std::thread::JoinHandle;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetClassNameW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
    PeekMessageW, PostThreadMessageW, EVENT_SYSTEM_FOREGROUND, MSG, PM_NOREMOVE,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT,
};

type Sink = Box<dyn Fn(u32) + Send>;

const FRAME_HOST_CLASS: &str = "ApplicationFrameWindow";

thread_local! {
    /// The sink of the watcher running on this thread. The WinEvent callback
    /// has no user data, and out-of-context callbacks run on the thread that
    /// installed the hook, so a thread-local is the right route.
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
}

/// Reports the PID of the foreground window to a sink, from its own thread.
pub struct ForegroundWatcher {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl ForegroundWatcher {
    /// Starts the `oma-foreground` thread. The current foreground window is
    /// reported at once, then every change. Fails if the hook cannot be set.
    ///
    /// The sink runs on the `oma-foreground` thread (the initial report
    /// included), so it must never block and must not own or drop the
    /// watcher. A panic in the sink is caught and swallowed.
    ///
    /// The hook uses `WINEVENT_SKIPOWNPROCESS`: switching to this app's own
    /// window produces no event, so the last reported PID stays; the initial
    /// report may still deliver the app's own PID.
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

    // Create the message queue before announcing the thread id, so a
    // `PostThreadMessageW` from `Drop` can never miss it.
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

    // SAFETY: `msg` is a valid MSG; the loop ends on WM_QUIT (0) or error (-1).
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        // WinEvent callbacks are dispatched inside GetMessageW; nothing else
        // to translate or dispatch on this thread.
    }

    // SAFETY: `hook` came from SetWinEventHook on this thread and is removed once.
    let _ = unsafe { UnhookWinEvent(hook) };
    SINK.with(|s| *s.borrow_mut() = None);
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

/// `report` behind `catch_unwind`: a panicking sink must neither unwind across
/// the FFI boundary nor kill the thread before it removes the hook.
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
    SINK.with(|s| {
        if let Ok(guard) = s.try_borrow() {
            if let Some(sink) = guard.as_ref() {
                sink(pid);
            }
        }
    });
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
    use std::time::Duration;

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn watcher_reports_the_current_foreground_at_start() {
        let (tx, rx) = mpsc::channel();
        let watcher = ForegroundWatcher::spawn(Box::new(move |pid| {
            let _ = tx.send(pid);
        }))
        .expect("hook installed");
        let pid = rx.recv_timeout(Duration::from_secs(2)).expect("a pid");
        assert_ne!(pid, 0);
        let started = std::time::Instant::now();
        drop(watcher);
        assert!(started.elapsed() < Duration::from_secs(2));
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
