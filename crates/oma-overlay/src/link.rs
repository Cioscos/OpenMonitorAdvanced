//! The pipe link to the app: the `oma-overlay-link` thread connects to the
//! pipe the app named, sends our `Hello`, validates every message it
//! receives and hands the good ones to the window through a channel, waking
//! it with a [`Wake`] (C11: `PostMessageW(hwnd, WM_APP_DATA, 0, 0)`).
//!
//! The link ends the process: [`LinkEvent::Closed`] carries the exit code.

use oma_ipc::overlay::{overlay_compatible, OverlayMessage};

/// The pipe closed after a working session.
pub const EXIT_OK: i32 = 0;
/// The command line was not `--pipe <overlay pipe name>`.
pub const EXIT_USAGE: i32 = 1;
/// The pipe could not be opened, or our `Hello` could not be sent.
pub const EXIT_CONNECT: i32 = 2;
/// The app speaks another overlay protocol version.
pub const EXIT_INCOMPATIBLE: i32 = 3;
/// The window or its drawing surface failed three frames in a row (device
/// lost, C11); the app restarts the overlay.
pub const EXIT_DEVICE_LOST: i32 = 4;

/// What the link hands to the window.
#[derive(Debug)]
pub enum LinkEvent {
    /// A validated message (boxed: the variants differ a lot in size).
    Message(Box<OverlayMessage>),
    /// The link is over; the process exits with this code.
    Closed { exit_code: i32 },
}

/// Wakes the thread that drains the link's channel.
pub trait Wake: Send + 'static {
    fn wake(&self);
}

impl<F: Fn() + Send + 'static> Wake for F {
    fn wake(&self) {
        self()
    }
}

/// What to do with one received message.
#[derive(Debug)]
pub(crate) enum Route {
    Deliver(Box<OverlayMessage>),
    /// Dropped: the reason goes to the log.
    Drop(String),
    Exit(i32),
}

/// Routes a decoded message: invalid ones are dropped, an incompatible
/// `Hello` ends the link, a compatible one stays here.
pub(crate) fn route(msg: OverlayMessage) -> Route {
    if let Err(e) = msg.validate() {
        return Route::Drop(e.to_string());
    }
    match msg {
        OverlayMessage::Hello(hello) if overlay_compatible(&hello) => {
            tracing::info!(app_version = %hello.version, "app hello received");
            Route::Drop("hello handled by the link".into())
        }
        OverlayMessage::Hello(hello) => {
            tracing::error!(
                protocol_version = hello.protocol_version,
                app_version = %hello.version,
                "the app speaks another overlay protocol"
            );
            Route::Exit(EXIT_INCOMPATIBLE)
        }
        msg => Route::Deliver(Box::new(msg)),
    }
}

#[cfg(windows)]
pub use imp::spawn;

#[cfg(windows)]
mod imp {
    use std::sync::mpsc::{self, Sender};
    use std::thread::JoinHandle;

    use oma_ipc::overlay::{OverlayHello, OverlayMessage, OVERLAY_PROTOCOL_VERSION};
    use oma_win::overlay_pipe::{connect_overlay_client, CloseReason, PipeEvent};

    use super::{route, LinkEvent, Route, Wake, EXIT_CONNECT, EXIT_OK};

    /// Starts the `oma-overlay-link` thread on `pipe`. Every event goes to
    /// `tx`, followed by a `wake`; the last one is always `Closed`.
    pub fn spawn(
        pipe: String,
        tx: Sender<LinkEvent>,
        waker: impl Wake,
    ) -> std::io::Result<JoinHandle<()>> {
        std::thread::Builder::new()
            .name("oma-overlay-link".into())
            .spawn(move || {
                let exit_code = run(&pipe, &tx, &waker);
                let _ = tx.send(LinkEvent::Closed { exit_code });
                waker.wake();
            })
    }

    fn run(pipe: &str, tx: &Sender<LinkEvent>, waker: &impl Wake) -> i32 {
        let conn = match connect_overlay_client(pipe) {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!(error = %e, "cannot connect to the overlay pipe");
                return EXIT_CONNECT;
            }
        };
        let hello = OverlayMessage::Hello(OverlayHello {
            protocol_version: OVERLAY_PROTOCOL_VERSION,
            version: env!("CARGO_PKG_VERSION").to_owned(),
        });
        if let Err(e) = conn.send(&hello) {
            tracing::error!(error = %e, "cannot send the overlay hello");
            return EXIT_CONNECT;
        }
        tracing::info!("connected to the app");

        let (events_tx, events) = mpsc::channel();
        let reader = conn.start_reader(move |event| events_tx.send(event).is_ok());
        let code = loop {
            match events.recv() {
                Ok(PipeEvent::Message(msg)) => match route(msg) {
                    Route::Deliver(msg) => {
                        if tx.send(LinkEvent::Message(msg)).is_err() {
                            break EXIT_OK;
                        }
                        waker.wake();
                    }
                    Route::Drop(reason) => tracing::warn!(%reason, "overlay message dropped"),
                    Route::Exit(code) => break code,
                },
                Ok(PipeEvent::Closed(reason)) => {
                    match reason {
                        CloseReason::Disconnected | CloseReason::Stopped => {
                            tracing::info!("the app closed the overlay pipe")
                        }
                        other => tracing::warn!(reason = ?other, "the overlay pipe broke"),
                    }
                    break EXIT_OK;
                }
                Err(_) => break EXIT_OK,
            }
        };
        reader.stop();
        code
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_ipc::overlay::{OverlayHello, Values, WireValue, OVERLAY_PROTOCOL_VERSION};

    fn hello(version: u32) -> OverlayMessage {
        OverlayMessage::Hello(OverlayHello {
            protocol_version: version,
            version: "0.5.0".into(),
        })
    }

    fn values(v: f64) -> OverlayMessage {
        OverlayMessage::Values(Values {
            at_ms: 0,
            values: vec![WireValue {
                id: "a".into(),
                value: Some(v),
                quality: "fresh".into(),
            }],
        })
    }

    #[test]
    fn incompatible_hello_exits_with_code_3() {
        assert!(matches!(
            route(hello(OVERLAY_PROTOCOL_VERSION + 1)),
            Route::Exit(EXIT_INCOMPATIBLE)
        ));
        assert!(matches!(
            route(hello(OVERLAY_PROTOCOL_VERSION)),
            Route::Drop(_)
        ));
    }

    #[test]
    fn invalid_messages_are_dropped_valid_ones_delivered() {
        assert!(matches!(route(values(f64::NAN)), Route::Drop(_)));
        assert!(matches!(route(values(1.0)), Route::Deliver(_)));
    }
}
