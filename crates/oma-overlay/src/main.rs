//! `oma-overlay.exe`: the overlay process the app starts while the overlay
//! is on (M7c). It reads the profile and the data from the app's private
//! pipe and draws over the game, without touching the game.
//!
//! In this task the loop has no window yet (C11): it applies the messages
//! to the state and logs what changed.

#![windows_subsystem = "windows"]

mod args;
mod cadence;
mod link;
mod log;
mod state;

use crate::link::EXIT_USAGE;

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
    run_link(args.pipe)
}

#[cfg(windows)]
fn run_link(pipe: String) -> i32 {
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::{Duration, Instant};

    use crate::link::{LinkEvent, EXIT_OK};
    use crate::state::{Changes, OverlayState};

    let (tx, rx) = mpsc::channel();
    // No window yet: the loop below blocks on the channel itself.
    if let Err(e) = link::spawn(pipe, tx, || {}) {
        tracing::error!(error = %e, "cannot start the link thread");
        return link::EXIT_CONNECT;
    }
    let start = Instant::now();
    let mut state = OverlayState::default();
    let mut cadence = cadence::Cadence::new(state.draw.chart_fps, state.draw.text_hz);
    loop {
        // Waits for the link, or for the next slot of a deferred change
        // (C11 replaces this with the window's message wait).
        let now_s = start.elapsed().as_secs_f64();
        let event = match cadence.next_wake(now_s) {
            Some(wait_s) => match rx.recv_timeout(Duration::from_secs_f64(wait_s)) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return EXIT_OK,
            },
            None => match rx.recv() {
                Ok(event) => Some(event),
                Err(_) => return EXIT_OK,
            },
        };
        let now_s = start.elapsed().as_secs_f64();
        let changes = match event {
            Some(LinkEvent::Message(msg)) => {
                let changes = state.apply(*msg, now_s);
                if changes.settings {
                    cadence.set_rates(state.draw.chart_fps, state.draw.text_hz);
                }
                changes
            }
            Some(LinkEvent::Closed { exit_code }) => {
                tracing::info!(exit_code, "overlay exiting");
                return exit_code;
            }
            None => Changes::default(),
        };
        if changes.any() {
            tracing::debug!(?changes, "overlay message applied");
        }
        let due = cadence.due(now_s, &changes);
        if due.any() {
            // No window yet: C11 draws and presents here.
            tracing::trace!(?due, "redraw due");
        }
    }
}

#[cfg(not(windows))]
fn run_link(_pipe: String) -> i32 {
    tracing::error!("the overlay runs on Windows only");
    link::EXIT_CONNECT
}
