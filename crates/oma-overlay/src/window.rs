//! The overlay window (spec §5.1): a `WS_POPUP` that is topmost, click-through
//! (`WS_EX_TRANSPARENT | WS_EX_LAYERED` and `HTTRANSPARENT`), never active
//! (`WS_EX_NOACTIVATE`, `SWP_NOACTIVATE`) and never on the
//! taskbar (`WS_EX_TOOLWINDOW`). Its content is a DirectComposition swapchain
//! (`compose`), so it has no redirection bitmap. It starts hidden and shows
//! only at the rectangle the app's `SetPlacement` gives.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use oma_core::overlay::geometry::PxRect;
use windows::core::{w, Error, Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, PostMessageW, PostQuitMessage, RegisterClassW,
    SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowPos, ShowWindow, HTTRANSPARENT,
    HWND_TOPMOST, LWA_ALPHA, MA_NOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE,
    WDA_EXCLUDEFROMCAPTURE, WDA_MONITOR, WDA_NONE, WINDOW_DISPLAY_AFFINITY, WINDOW_EX_STYLE,
    WM_APP, WM_DESTROY, WM_DPICHANGED, WM_MOUSEACTIVATE, WM_NCHITTEST, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::link::Wake;

/// Posted by the link thread: there are events in the channel.
pub const WM_APP_DATA: u32 = WM_APP + 1;

const CLASS_NAME: PCWSTR = w!("OmaOverlayWindow");

/// Makes the process Per-Monitor v2 aware, so the rectangles of
/// `SetPlacement` (physical pixels) are taken as they are.
pub fn set_dpi_awareness() {
    // SAFETY: a process-wide setting with a constant context; called once at
    // start, before any window exists.
    if let Err(e) =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
    {
        // Already set (by a manifest or an earlier call): not fatal.
        tracing::warn!(error = %e, "cannot set the per-monitor v2 DPI awareness");
    }
}

/// The extended styles of the overlay window (spec §5.1, spike S4/SD10).
pub(crate) fn ex_style() -> WINDOW_EX_STYLE {
    WS_EX_TOPMOST
        | WS_EX_TRANSPARENT
        | WS_EX_LAYERED
        | WS_EX_NOACTIVATE
        | WS_EX_TOOLWINDOW
        | WS_EX_NOREDIRECTIONBITMAP
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Clicks go to the window below (the game).
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        // Never activated, even if a click were to reach us.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // The position comes from the app's `SetPlacement`, already in
        // physical pixels for the target's monitor.
        WM_DPICHANGED => LRESULT(0),
        WM_DESTROY => {
            // SAFETY: called on the window's own thread, from its procedure.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: forwarding the unmodified message to the default procedure,
        // on the window's thread.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Registers the window class once per process (tests create several windows).
fn register_class() -> Result<()> {
    static CLASS: OnceLock<std::result::Result<(), Error>> = OnceLock::new();
    CLASS
        .get_or_init(|| {
            // SAFETY: the class name is a static string and the procedure a
            // free function, so both outlive the class; the module handle
            // is our own executable's.
            unsafe {
                let instance = GetModuleHandleW(None)?;
                let class = WNDCLASSW {
                    lpfnWndProc: Some(wndproc),
                    hInstance: instance.into(),
                    lpszClassName: CLASS_NAME,
                    ..Default::default()
                };
                if RegisterClassW(&class) == 0 {
                    return Err(Error::from_thread());
                }
            }
            Ok(())
        })
        .clone()
}

/// Creates the overlay window, hidden. The calling thread owns it and must
/// pump its messages.
pub fn create() -> Result<HWND> {
    register_class()?;
    // SAFETY: the class is registered above; the title is a static string;
    // no parent, menu or creation data. The window is created without
    // `WS_VISIBLE`, so it stays hidden.
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None)?;
        CreateWindowExW(
            ex_style(),
            CLASS_NAME,
            w!("OpenMonitor Advanced overlay"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(instance.into()),
            None,
        )?
    };
    // SAFETY: `hwnd` is the layered window just created on this thread.
    let layered = unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA) };
    if let Err(e) = layered {
        destroy(hwnd);
        return Err(e);
    }
    Ok(hwnd)
}

/// Destroys the window (from its own thread).
pub(crate) fn destroy(hwnd: HWND) {
    // SAFETY: `hwnd` is a window created by `create` on this thread; a stale
    // handle only makes the call fail.
    let _ = unsafe { DestroyWindow(hwnd) };
}

/// Moves the window to `rect`, on top of the topmost band again, and shows
/// it without activating it, in one `SetWindowPos`; `None` hides it (spec
/// §3.2, §5.1).
pub fn apply_placement(hwnd: HWND, rect: Option<PxRect>) {
    match rect {
        Some(r) => {
            // SAFETY: our own window, from its thread; plain values only.
            let moved = unsafe {
                SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    r.x,
                    r.y,
                    r.w.max(1),
                    r.h.max(1),
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
            };
            if let Err(e) = moved {
                tracing::warn!(error = %e, "cannot place the overlay window");
            }
        }
        None => {
            // SAFETY: as above.
            let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
        }
    }
}

/// The display affinity to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Affinity {
    /// `WDA_NONE`: captures see the overlay.
    None,
    /// `WDA_EXCLUDEFROMCAPTURE`: captures do not see it (Windows 10 2004+).
    ExcludeFromCapture,
    /// `WDA_MONITOR`: captures see a black rectangle (older Windows).
    Monitor,
}

/// What `hideFromCapture` asks for, given whether `WDA_EXCLUDEFROMCAPTURE`
/// is supported.
pub(crate) fn affinity_plan(hide: bool, exclude_supported: bool) -> Affinity {
    match (hide, exclude_supported) {
        (false, _) => Affinity::None,
        (true, true) => Affinity::ExcludeFromCapture,
        (true, false) => Affinity::Monitor,
    }
}

fn set_affinity(hwnd: HWND, affinity: Affinity) -> Result<()> {
    let value: WINDOW_DISPLAY_AFFINITY = match affinity {
        Affinity::None => WDA_NONE,
        Affinity::ExcludeFromCapture => WDA_EXCLUDEFROMCAPTURE,
        Affinity::Monitor => WDA_MONITOR,
    };
    // SAFETY: our own top-level window, from its thread; a plain value.
    unsafe { SetWindowDisplayAffinity(hwnd, value) }
}

/// Hides the window from screen captures, or shows it again (§5.1). Where
/// `WDA_EXCLUDEFROMCAPTURE` is refused (before Windows 10 2004) it falls
/// back to `WDA_MONITOR` and says so in the log.
pub fn apply_capture_affinity(hwnd: HWND, hide: bool) {
    let plan = affinity_plan(hide, true);
    let Err(e) = set_affinity(hwnd, plan) else {
        return;
    };
    if plan == Affinity::ExcludeFromCapture {
        tracing::info!(
            error = %e,
            "WDA_EXCLUDEFROMCAPTURE not supported; captures show a black rectangle (WDA_MONITOR)"
        );
        if let Err(e) = set_affinity(hwnd, affinity_plan(hide, false)) {
            tracing::warn!(error = %e, "cannot set the capture affinity");
        }
    } else {
        tracing::warn!(error = %e, "cannot set the capture affinity");
    }
}

/// Wakes the window thread with one `WM_APP_DATA` per burst: the flag stays
/// set until the window clears it, right before draining the channel.
pub struct DataWaker {
    /// The window handle as an integer (`HWND` is not `Send`).
    hwnd: isize,
    posted: Arc<AtomicBool>,
}

/// The waker for the link and the flag the window clears before draining.
pub fn data_waker(hwnd: HWND) -> (DataWaker, Arc<AtomicBool>) {
    let posted = Arc::new(AtomicBool::new(false));
    let waker = DataWaker {
        hwnd: hwnd.0 as isize,
        posted: Arc::clone(&posted),
    };
    (waker, posted)
}

impl Wake for DataWaker {
    fn wake(&self) {
        if self.posted.swap(true, Ordering::SeqCst) {
            return;
        }
        let hwnd = HWND(self.hwnd as *mut core::ffi::c_void);
        // SAFETY: posting to a window handle is allowed from any thread; a
        // destroyed window only makes the call fail.
        if unsafe { PostMessageW(Some(hwnd), WM_APP_DATA, WPARAM(0), LPARAM(0)) }.is_err() {
            // Queue full or window gone: the next event tries again.
            self.posted.store(false, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowDisplayAffinity, GetWindowLongPtrW, IsWindowVisible, GWL_EXSTYLE,
    };

    #[test]
    fn ex_style_is_click_through_topmost_tool_window() {
        let style = ex_style();
        for flag in [
            WS_EX_TOPMOST,
            WS_EX_TRANSPARENT,
            WS_EX_LAYERED,
            WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW,
            WS_EX_NOREDIRECTIONBITMAP,
        ] {
            assert!(style.contains(flag), "missing {flag:?} in {style:?}");
        }
        // Nothing else: no app window, no caption-related extended style.
        assert_eq!(
            style,
            WS_EX_TOPMOST
                | WS_EX_TRANSPARENT
                | WS_EX_LAYERED
                | WS_EX_NOACTIVATE
                | WS_EX_TOOLWINDOW
                | WS_EX_NOREDIRECTIONBITMAP
        );
    }

    #[test]
    fn affinity_plan_falls_back_to_monitor() {
        assert_eq!(affinity_plan(true, true), Affinity::ExcludeFromCapture);
        assert_eq!(affinity_plan(true, false), Affinity::Monitor);
        assert_eq!(affinity_plan(false, true), Affinity::None);
        assert_eq!(affinity_plan(false, false), Affinity::None);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn window_is_hidden_until_placement() {
        let hwnd = create().expect("overlay window");
        // SAFETY: our window, on this thread.
        let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
        assert!(!visible, "the overlay window starts hidden");
        // SAFETY: as above; reads the extended style of our own window.
        let actual = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        assert_eq!(actual & ex_style().0, ex_style().0, "{actual:#x}");
        // Without a rectangle it stays hidden; with one it would show on the
        // user's screen, which tests never do.
        apply_placement(hwnd, None);
        // SAFETY: as above.
        assert!(!unsafe { IsWindowVisible(hwnd) }.as_bool());
        let affinity = || {
            let mut value = u32::MAX;
            // SAFETY: our own window; `value` is a valid local out-pointer.
            unsafe { GetWindowDisplayAffinity(hwnd, &mut value) }.expect("affinity");
            value
        };
        apply_capture_affinity(hwnd, true);
        let hidden = affinity();
        assert!(
            hidden == WDA_EXCLUDEFROMCAPTURE.0 || hidden == WDA_MONITOR.0,
            "{hidden:#x}"
        );
        apply_capture_affinity(hwnd, false);
        assert_eq!(affinity(), WDA_NONE.0);
        destroy(hwnd);
    }
}
