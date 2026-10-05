//! `oma-overlay.exe`: the overlay process the app starts while the overlay
//! is on (M7c). It reads the profile and the data from the app's private
//! pipe and draws over the game, without touching the game.
//!
//! The main thread owns the click-through window (`window`) and its
//! composition surface (`compose`); it sleeps in the message wait until the
//! link posts data or a deferred redraw is due (`Cadence::next_wake`), and
//! presents nothing without changes (§5.2). The renderer (`render`) draws
//! the profile from its cache, refreshing texts and charts at their rates.
//!
//! With `--preview` (M7d) the same loop draws in a normal window for the
//! overlay editor: the area is the client area, `SetPlacement` is ignored,
//! and closing the window ends the process with `EXIT_CLOSED`.

#![windows_subsystem = "windows"]

mod args;
mod cadence;
#[cfg(windows)]
mod compose;
mod link;
mod log;
mod render;
mod state;
#[cfg(windows)]
mod window;

use oma_core::overlay::geometry::{place, PxRect};
use oma_ipc::overlay::PxArea;

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
    run_window(args.pipe, args.preview)
}

/// The preview window's title before the first profile brings
/// `strings["previewTitle"]`.
const PREVIEW_TITLE: &str = "OpenMonitor Advanced";

/// The translation that draws the profile placed at `placed` on a surface
/// covering `client`.
fn preview_origin(client: PxRect, placed: PxRect) -> (f32, f32) {
    ((placed.x - client.x) as f32, (placed.y - client.y) as f32)
}

/// In preview the placement is the client area at the window's DPI; true if
/// it changed.
fn set_preview_area(state: &mut OverlayState, client: PxRect, dpi: u32) -> bool {
    let placement = Some((
        PxArea {
            x: client.x,
            y: client.y,
            width: client.w,
            height: client.h,
        },
        dpi,
    ));
    let changed = state.placement != placement;
    state.placement = placement;
    changed
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

/// Failed frames in a row before giving up (the app restarts us).
const MAX_FAILURES: u32 = 3;

/// Seconds before the next attempt after `failures` failed frames in a row:
/// a quick retry for a transient error, then a long one, so that the last
/// attempt comes after a driver reset (TDR, about 2 s) is over.
fn retry_delay_s(failures: u32) -> f64 {
    if failures <= 1 {
        0.25
    } else {
        3.0
    }
}

/// After this long hidden, the device and the swapchain are released (§11);
/// the next visible frame builds them again.
const HIDDEN_RELEASE_S: f64 = 30.0;

/// Seconds left before the drawing surface of a window hidden since
/// `hidden_since_s` is released; zero or less means now.
fn release_in_s(hidden_since_s: f64, now_s: f64) -> f64 {
    hidden_since_s + HIDDEN_RELEASE_S - now_s
}

#[cfg(windows)]
fn run_window(pipe: String, preview: bool) -> i32 {
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
    use oma_ipc::overlay::OverlayMessage;

    window::set_dpi_awareness();
    let created = if preview {
        window::create_preview(PREVIEW_TITLE)
    } else {
        window::create()
    };
    let hwnd = match created {
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
    // Built at the first visible frame, dropped on a lost device and after
    // `HIDDEN_RELEASE_S` hidden.
    let mut gfx: Option<Compositor> = None;
    // Brushes, text layouts and geometries, bound to `gfx`'s device.
    let mut cache = render::RenderCache::default();
    let mut failures = 0u32;
    // A frame failed: draw everything again at this time.
    let mut retry_at: Option<f64> = None;
    // Since when the window has been hidden.
    let mut hidden_since: Option<f64> = None;
    // Where the window should be, and where it is shown.
    let mut rect: Option<PxRect> = None;
    let mut shown: Option<PxRect> = None;
    // The window starts with `WDA_NONE`.
    let mut hidden_from_capture = false;
    // The preview window's title as last set.
    let mut title = PREVIEW_TITLE.to_owned();
    loop {
        let now_s = start.elapsed().as_secs_f64();
        let mut wake = cadence.next_wake(now_s);
        let mut wake_at = |s: f64| {
            let w = (s - now_s).max(0.0);
            wake = Some(wake.map_or(w, |v| v.min(w)));
        };
        if let Some(at) = retry_at {
            wake_at(at);
        }
        if let (Some(since), Some(_)) = (hidden_since, &gfx) {
            wake_at(now_s + release_in_s(since, now_s));
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
                    // `EXIT_OK` from the overlay window, `EXIT_CLOSED` from
                    // the preview's close button.
                    let code = msg.wParam.0 as i32;
                    tracing::info!(code, "overlay window closed");
                    return code;
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
        // A placement message, even an unchanged one, puts the window back on
        // top of the topmost band (a topmost window shown later covers us).
        let mut reassert = false;
        loop {
            match rx.try_recv() {
                // The preview's area is its client area.
                Ok(LinkEvent::Message(msg))
                    if preview && matches!(*msg, OverlayMessage::SetPlacement(_)) => {}
                Ok(LinkEvent::Message(msg)) => {
                    reassert |= matches!(*msg, OverlayMessage::SetPlacement(_));
                    changes.merge(state.apply(*msg, now_s));
                }
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
        let mut client = None;
        if preview {
            let (area, dpi) = window::client_area(hwnd);
            changes.placement |= set_preview_area(&mut state, area, dpi);
            client = (area.w > 0 && area.h > 0).then_some(area);
            if changes.layout {
                let wanted = state
                    .strings
                    .get("previewTitle")
                    .map_or(PREVIEW_TITLE, String::as_str);
                if wanted != title {
                    title = wanted.to_owned();
                    window::set_title(hwnd, &title);
                }
            }
        }
        if changes.settings {
            cadence.set_rates(state.draw.chart_fps, state.draw.text_hz);
            if !preview && state.draw.hide_from_capture != hidden_from_capture {
                hidden_from_capture = state.draw.hide_from_capture;
                window::apply_capture_affinity(hwnd, hidden_from_capture);
            }
        }
        if changes.layout || changes.placement {
            rect = placement_rect(&state);
        }
        // The preview draws on its whole client area, the profile inside it.
        let (surface, view) = match client {
            Some(c) => (
                Some(c),
                render::View {
                    origin: rect.map_or((0.0, 0.0), |p| preview_origin(c, p)),
                    background: Some(render::PREVIEW_BACKGROUND),
                },
            ),
            None if preview => (None, render::View::default()),
            None => (rect, render::View::default()),
        };
        if changes.layout || changes.placement || changes.settings {
            cache.invalidate();
        }

        let due = cadence.due(now_s, &changes);
        let Some(r) = surface else {
            // Hidden: nothing to draw, and nothing to retry.
            if preview {
                // Minimized: the preview keeps its surface while it exists.
                retry_at = None;
                failures = 0;
                continue;
            }
            if shown.take().is_some() {
                window::apply_placement(hwnd, None);
            }
            retry_at = None;
            failures = 0;
            let since = *hidden_since.get_or_insert(now_s);
            if gfx.is_some() && release_in_s(since, now_s) <= 0.0 {
                tracing::debug!("overlay hidden for a while; releasing the graphics device");
                cache.release_device();
                gfx = None;
            }
            continue;
        };
        hidden_since = None;
        let wants_draw = match retry_at {
            Some(at) => now_s >= at,
            None => due.any(),
        };
        let mut place_now = reassert && shown == Some(r);
        if wants_draw {
            match draw_frame(&mut gfx, &mut cache, hwnd, r, &state, due, view) {
                Ok(()) => {
                    failures = 0;
                    retry_at = None;
                    // Shown (or moved) only once its content is presented.
                    place_now |= shown != Some(r);
                }
                Err(e) => {
                    failures += 1;
                    // The retry rebuilds everything: the due texts and charts
                    // of this frame may be half updated.
                    cache.invalidate();
                    if needs_recreate(e.code()) {
                        tracing::warn!(error = %e, failures, "graphics device lost; recreating");
                        cache.release_device();
                        gfx = None;
                    } else {
                        tracing::warn!(error = %e, failures, "overlay frame failed");
                    }
                    if failures >= MAX_FAILURES {
                        tracing::error!(failures, "overlay drawing keeps failing; exiting");
                        return EXIT_DEVICE_LOST;
                    }
                    retry_at = Some(now_s + retry_delay_s(failures));
                }
            }
        }
        // The user places the preview window.
        if place_now && !preview {
            window::apply_placement(hwnd, Some(r));
            shown = Some(r);
        }
    }
}

/// Draws one frame at the size of `r`: the profile, from the cache, as
/// `view` says.
#[cfg(windows)]
fn draw_frame(
    gfx: &mut Option<compose::Compositor>,
    cache: &mut render::RenderCache,
    hwnd: windows::Win32::Foundation::HWND,
    r: PxRect,
    state: &OverlayState,
    due: cadence::Due,
    view: render::View,
) -> windows::core::Result<()> {
    let (w, h) = (r.w.max(1) as u32, r.h.max(1) as u32);
    let g = match gfx {
        Some(g) => g,
        None => gfx.insert(compose::Compositor::new(hwnd, w, h)?),
    };
    g.resize(w, h)?;
    let drawn = render::draw(g.begin(), cache, state, due, view);
    // The frame is closed (and presented) even after a drawing error, so the
    // context is not left inside `BeginDraw`.
    let presented = g.end_and_present();
    drawn.and(presented)
}

#[cfg(not(windows))]
fn run_window(_pipe: String, _preview: bool) -> i32 {
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
    fn preview_origin_is_the_placed_offset() {
        let client = PxRect {
            x: 0,
            y: 0,
            w: 1280,
            h: 720,
        };
        let placed = PxRect {
            x: 1100,
            y: 16,
            w: 164,
            h: 80,
        };
        assert_eq!(preview_origin(client, placed), (1100.0, 16.0));
        assert_eq!(preview_origin(client, client), (0.0, 0.0));
        // Relative to the client rectangle, whatever its origin.
        let shifted = PxRect {
            x: 10,
            y: 20,
            ..client
        };
        assert_eq!(preview_origin(shifted, placed), (1090.0, -4.0));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires real Windows hardware"]
    fn preview_window_draws_the_gaming_profile() {
        // The test's own window, shown briefly without activation; no input
        // is sent to it or to anything else.
        let hwnd = window::create_preview("OMA preview test").expect("preview window");
        let mut state = render::tests::gaming_state();
        let (client, dpi) = window::client_area(hwnd);
        assert!(client.w > 0 && client.h > 0, "{client:?}");
        set_preview_area(&mut state, client, dpi);
        let placed = placement_rect(&state).expect("placed in the client area");
        let view = render::View {
            origin: preview_origin(client, placed),
            background: Some(render::PREVIEW_BACKGROUND),
        };
        let mut gfx = None;
        let mut cache = render::RenderCache::default();
        let due = cadence::Due {
            text: true,
            charts: true,
        };
        let drawn = draw_frame(&mut gfx, &mut cache, hwnd, client, &state, due, view);
        window::destroy(hwnd);
        drawn.expect("one preview frame presented");
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
    fn retry_backs_off_past_a_driver_reset() {
        assert_eq!(retry_delay_s(1), 0.25);
        assert_eq!(retry_delay_s(2), 3.0);
        assert_eq!(retry_delay_s(5), 3.0);
        // The last attempt before the exit comes after a TDR (2 s) is over.
        let before_last: f64 = (1..MAX_FAILURES).map(retry_delay_s).sum();
        assert!(before_last > 3.0, "{before_last}");
    }

    #[test]
    fn compositor_released_after_30_s_hidden() {
        assert_eq!(release_in_s(10.0, 10.0), 30.0);
        assert_eq!(release_in_s(10.0, 25.0), 15.0);
        assert!(release_in_s(10.0, 40.0) <= 0.0);
        assert!(release_in_s(10.0, 100.0) <= 0.0);
    }
}
