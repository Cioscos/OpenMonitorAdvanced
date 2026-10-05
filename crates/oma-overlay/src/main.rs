//! `oma-overlay.exe`: the overlay process the app starts while the overlay
//! is on (M7c). It reads the profile and the data from the app's private
//! pipe and draws over the game, without touching the game.
//!
//! The main thread owns the click-through window (`window`) and its
//! composition surface (`compose`); it sleeps in the message wait until the
//! link posts data or a deferred redraw is due (`Cadence::next_wake`), and
//! presents nothing without changes (§5.2). In this task only the panel is
//! drawn; the blocks come with the renderer (C12).

#![windows_subsystem = "windows"]

mod args;
mod cadence;
#[cfg(windows)]
mod compose;
mod link;
mod log;
mod state;
#[cfg(windows)]
mod window;

use oma_core::overlay::geometry::{place, PxRect};

use crate::link::EXIT_USAGE;
use crate::state::OverlayState;

fn main() {
    let code = {
        // Dropped before `exit`, so the buffered log lines are flushed.
        let _log_guard = log::init();
        run()
    };
    std::process::exit(code);
}

fn run() -> i32 {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match args::parse_args(&argv) {
        Ok(args) => args,
        Err(e) => {
            tracing::error!(error = %e, "bad command line");
            return EXIT_USAGE;
        }
    };
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "overlay starting");
    run_window(args.pipe)
}

/// Where the window goes: the profile placed in the target area (C3), or
/// `None` (hidden) without a profile, without an area or without blocks.
fn placement_rect(state: &OverlayState) -> Option<PxRect> {
    let profile = state.profile.as_ref()?;
    let (area, dpi) = state.placement.as_ref()?;
    let area = PxRect {
        x: area.x,
        y: area.y,
        w: area.width,
        h: area.height,
    };
    place(profile, area, *dpi)
}

/// The message-wait timeout for a wake in `wait_s` seconds: rounded up, so
/// the loop does not wake just before the slot and spin; `None` waits for a
/// message only.
fn wait_ms(wait_s: Option<f64>) -> u32 {
    const INFINITE: u32 = u32::MAX;
    match wait_s {
        None => INFINITE,
        Some(s) => (s.max(0.0) * 1000.0).ceil().min(f64::from(INFINITE - 1)) as u32,
    }
}

/// The panel's corner radius in pixels: `radius` is in 96-DPI pixels at
/// scale 1, and never more than half the shorter side.
fn panel_radius_px(radius: f64, scale: f64, dpi: u32, w: u32, h: u32) -> f32 {
    let r = radius.max(0.0) * scale * f64::from(dpi) / 96.0;
    r.min(f64::from(w.min(h)) / 2.0) as f32
}

#[cfg(windows)]
fn run_window(pipe: String) -> i32 {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::{self, TryRecvError};
    use std::time::Instant;

    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MsgWaitForMultipleObjectsEx, PeekMessageW, TranslateMessage, MSG,
        MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, WM_QUIT,
    };

    use crate::cadence::Cadence;
    use crate::compose::{needs_recreate, Compositor};
    use crate::link::{LinkEvent, EXIT_CONNECT, EXIT_DEVICE_LOST, EXIT_OK};
    use crate::state::Changes;
    use crate::window::WM_APP_DATA;

    /// After a failed frame the next attempt comes this soon.
    const RETRY_S: f64 = 0.1;
    /// Failed frames in a row before giving up (the app restarts us).
    const MAX_FAILURES: u32 = 3;

    window::set_dpi_awareness();
    let hwnd = match window::create() {
        Ok(hwnd) => hwnd,
        Err(e) => {
            tracing::error!(error = %e, "cannot create the overlay window");
            return EXIT_DEVICE_LOST;
        }
    };
    let (tx, rx) = mpsc::channel();
    let (waker, posted) = window::data_waker(hwnd);
    if let Err(e) = link::spawn(pipe, tx, waker) {
        tracing::error!(error = %e, "cannot start the link thread");
        return EXIT_CONNECT;
    }

    let start = Instant::now();
    let mut state = OverlayState::default();
    let mut cadence = Cadence::new(state.draw.chart_fps, state.draw.text_hz);
    // Built at the first visible frame, dropped on a lost device.
    let mut gfx: Option<Compositor> = None;
    let mut failures = 0u32;
    // A frame failed: draw everything again at `RETRY_S`.
    let mut retry = false;
    // Where the window should be, and where it is shown.
    let mut rect: Option<PxRect> = None;
    let mut shown: Option<PxRect> = None;
    // The window starts with `WDA_NONE`.
    let mut hidden_from_capture = false;
    loop {
        let now_s = start.elapsed().as_secs_f64();
        let mut wake = cadence.next_wake(now_s);
        if retry {
            wake = Some(wake.map_or(RETRY_S, |w| w.min(RETRY_S)));
        }
        // SAFETY: no handles, only the queue of this thread; returns on a
        // message (also one already seen) or at the timeout.
        let _ = unsafe {
            MsgWaitForMultipleObjectsEx(None, wait_ms(wake), QS_ALLINPUT, MWMO_INPUTAVAILABLE)
        };

        let mut msg = MSG::default();
        // SAFETY: the standard pump of the thread that owns the window; `msg`
        // is a valid local.
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            match msg.message {
                WM_QUIT => {
                    tracing::info!("overlay window closed");
                    return EXIT_OK;
                }
                // Only a wake: the channel is drained below in any case.
                WM_APP_DATA => {}
                _ => {
                    // SAFETY: dispatching a message just taken from our queue.
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
        }

        // Cleared before draining, so an event sent from now on posts a new
        // wake. Everything queued is applied in order, so a burst costs one
        // pass and at most one frame.
        posted.store(false, Ordering::SeqCst);
        let now_s = start.elapsed().as_secs_f64();
        let mut changes = Changes::default();
        loop {
            match rx.try_recv() {
                Ok(LinkEvent::Message(msg)) => changes.merge(state.apply(*msg, now_s)),
                Ok(LinkEvent::Closed { exit_code }) => {
                    tracing::info!(exit_code, "overlay exiting");
                    return exit_code;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return EXIT_OK,
            }
        }
        if changes.any() {
            tracing::debug!(?changes, "overlay messages applied");
        }
        if changes.settings {
            cadence.set_rates(state.draw.chart_fps, state.draw.text_hz);
            if state.draw.hide_from_capture != hidden_from_capture {
                hidden_from_capture = state.draw.hide_from_capture;
                window::apply_capture_affinity(hwnd, hidden_from_capture);
            }
        }
        if changes.layout || changes.placement {
            rect = placement_rect(&state);
        }

        let due = cadence.due(now_s, &changes);
        let Some(r) = rect else {
            // Hidden: nothing to draw, and nothing to retry.
            if shown.take().is_some() {
                window::apply_placement(hwnd, None);
            }
            retry = false;
            continue;
        };
        if !(due.any() || retry) {
            continue;
        }
        match draw_frame(&mut gfx, hwnd, r, &state) {
            Ok(()) => {
                failures = 0;
                retry = false;
                // Shown (or moved) only once its content is presented.
                if shown != Some(r) {
                    window::apply_placement(hwnd, Some(r));
                    shown = Some(r);
                }
            }
            Err(e) => {
                failures += 1;
                if needs_recreate(e.code()) {
                    tracing::warn!(error = %e, failures, "graphics device lost; recreating");
                    gfx = None;
                } else {
                    tracing::warn!(error = %e, failures, "overlay frame failed");
                }
                if failures >= MAX_FAILURES {
                    tracing::error!(failures, "overlay drawing keeps failing; exiting");
                    return EXIT_DEVICE_LOST;
                }
                retry = true;
            }
        }
    }
}

/// Draws one frame at the size of `r`: for now only the profile's panel.
#[cfg(windows)]
fn draw_frame(
    gfx: &mut Option<compose::Compositor>,
    hwnd: windows::Win32::Foundation::HWND,
    r: PxRect,
    state: &OverlayState,
) -> windows::core::Result<()> {
    use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_RECT_F};
    use windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT;

    let (w, h) = (r.w.max(1) as u32, r.h.max(1) as u32);
    let g = match gfx {
        Some(g) => g,
        None => gfx.insert(compose::Compositor::new(hwnd, w, h)?),
    };
    g.resize(w, h)?;
    let (w, h) = g.size();
    {
        let dc = g.begin();
        if let (Some(profile), Some((_, dpi))) = (&state.profile, &state.placement) {
            let p = &profile.panel;
            let color = D2D1_COLOR_F {
                r: f32::from(p.color.r) / 255.0,
                g: f32::from(p.color.g) / 255.0,
                b: f32::from(p.color.b) / 255.0,
                a: (f64::from(p.color.a) / 255.0 * p.opacity.clamp(0.0, 1.0)) as f32,
            };
            let radius = panel_radius_px(p.radius, profile.scale, *dpi, w, h);
            let panel = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: w as f32,
                    bottom: h as f32,
                },
                radiusX: radius,
                radiusY: radius,
            };
            // SAFETY: drawing on the context between `begin` and
            // `end_and_present`, from the thread that owns it; the inputs are
            // locals that outlive the calls.
            unsafe {
                let brush = dc.CreateSolidColorBrush(&color, None)?;
                dc.FillRoundedRectangle(&panel, &brush);
            }
        }
    }
    g.end_and_present()
}

#[cfg(not(windows))]
fn run_window(_pipe: String) -> i32 {
    tracing::error!("the overlay runs on Windows only");
    link::EXIT_CONNECT
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::overlay::parse_profile;
    use oma_ipc::overlay::{OverlayMessage, PxArea, SetPlacement};

    #[test]
    fn placement_rect_needs_profile_and_area() {
        let mut state = OverlayState::default();
        assert_eq!(placement_rect(&state), None);
        let area = PxArea {
            x: 100,
            y: 50,
            width: 1920,
            height: 1080,
        };
        state.apply(
            OverlayMessage::SetPlacement(SetPlacement {
                area: Some(area),
                dpi: 96,
            }),
            0.0,
        );
        // An area without a profile stays hidden.
        assert_eq!(placement_rect(&state), None);
        let json = r#"{ "format": 1, "name": "t", "blocks": [
            { "id": "a", "rect": { "x": 0, "y": 0, "w": 10, "h": 2 },
              "source": { "frames": "fps-displayed" }, "kind": "text" } ] }"#;
        state.profile = Some(parse_profile(json).expect("profile"));
        let r = placement_rect(&state).expect("placed");
        assert!(r.w > 0 && r.h > 0, "{r:?}");
        assert!(r.x >= area.x && r.y >= area.y, "{r:?}");
        // The area goes away: hidden again.
        state.apply(
            OverlayMessage::SetPlacement(SetPlacement {
                area: None,
                dpi: 96,
            }),
            0.0,
        );
        assert_eq!(placement_rect(&state), None);
    }

    #[test]
    fn wait_ms_rounds_up_and_waits_forever_without_a_wake() {
        assert_eq!(wait_ms(None), u32::MAX);
        assert_eq!(wait_ms(Some(0.0)), 0);
        assert_eq!(wait_ms(Some(-1.0)), 0);
        assert_eq!(wait_ms(Some(1.0 / 30.0)), 34);
        assert_eq!(wait_ms(Some(0.5)), 500);
        assert_eq!(wait_ms(Some(1e12)), u32::MAX - 1);
    }

    #[test]
    fn panel_radius_scales_with_dpi_and_fits_the_panel() {
        assert_eq!(panel_radius_px(4.0, 1.0, 96, 100, 100), 4.0);
        assert_eq!(panel_radius_px(4.0, 1.5, 192, 100, 100), 12.0);
        assert_eq!(panel_radius_px(40.0, 1.0, 96, 100, 20), 10.0);
        assert_eq!(panel_radius_px(-1.0, 1.0, 96, 100, 20), 0.0);
    }
}
