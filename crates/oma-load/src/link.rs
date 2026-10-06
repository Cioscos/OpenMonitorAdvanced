//! The pipe link to the app: connect, send our `Hello` and the full topology,
//! then wait for `Run`. The phase engine arrives with A8: until then `Run` is
//! answered with `Finished { reason: Failed }`.
//!
//! The link ends the process: [`run`] returns the exit code.

use oma_ipc::load::{
    load_compatible, FinishReason, Finished, Isa, LoadHello, LoadMessage, LOAD_PROTOCOL_VERSION,
};

/// The pipe closed after a working session.
pub const EXIT_OK: i32 = 0;
/// The command line was not valid, or the app sent an invalid message.
pub const EXIT_USAGE: i32 = 1;
/// The pipe could not be opened, or our `Hello` could not be sent.
pub const EXIT_CONNECT: i32 = 2;
/// The app speaks another load protocol version.
pub const EXIT_INCOMPATIBLE: i32 = 3;

/// The instruction sets this CPU has, best first. `avx512` asks for AVX-512F only (the
/// kernels use F alone); `avx2` also needs FMA; SSE2 is part of x86_64.
pub fn detected_isa() -> Vec<Isa> {
    let mut isa = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx512f") {
            isa.push(Isa::Avx512);
        }
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            isa.push(Isa::Avx2);
        }
        isa.push(Isa::Sse2);
    }
    isa
}

/// What to do with one received message.
#[derive(Debug, PartialEq)]
pub enum Route {
    /// A valid `Run`: start the plan.
    Run(Box<oma_ipc::load::RunRequest>),
    /// A `Stop`.
    Stop,
    /// A compatible `Hello`, or a message only the app receives: nothing to do.
    Ignore,
    Exit(i32),
}

/// Routes a decoded message: an invalid one ends the process (the app sent a plan that
/// cannot be run), an incompatible `Hello` too.
pub fn route(msg: LoadMessage) -> Route {
    if let Err(e) = msg.validate() {
        tracing::error!(error = %e, "invalid message from the app");
        return Route::Exit(EXIT_USAGE);
    }
    match msg {
        LoadMessage::Hello(h) if load_compatible(&h) => {
            tracing::info!(app_version = %h.version, "app hello received");
            Route::Ignore
        }
        LoadMessage::Hello(h) => {
            tracing::error!(
                protocol_version = h.protocol_version,
                app_version = %h.version,
                "the app speaks another load protocol"
            );
            Route::Exit(EXIT_INCOMPATIBLE)
        }
        LoadMessage::Run(r) => Route::Run(Box::new(r)),
        LoadMessage::Stop(_) => Route::Stop,
        other => {
            tracing::warn!(message = ?other, "unexpected message ignored");
            Route::Ignore
        }
    }
}

/// The answer to `Run` until the engine exists (A8).
pub fn failed_finish() -> LoadMessage {
    LoadMessage::Finished(Finished {
        reason: FinishReason::Failed,
        checks: 0,
        errors: 0,
    })
}

/// Our `Hello`.
pub fn hello() -> LoadMessage {
    LoadMessage::Hello(LoadHello {
        protocol_version: LOAD_PROTOCOL_VERSION,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        isa: detected_isa(),
    })
}

/// Connects to `pipe`, serves the app until the pipe closes and returns the exit code.
#[cfg(windows)]
pub fn run(pipe: &str) -> i32 {
    use std::sync::mpsc;

    use oma_win::load_pipe::connect_load_client;
    use oma_win::private_pipe::{CloseReason, PipeEvent};

    let conn = match connect_load_client(pipe) {
        Ok(conn) => conn,
        Err(e) => {
            tracing::error!(error = %e, "cannot connect to the load pipe");
            return EXIT_CONNECT;
        }
    };
    if let Err(e) = conn.send(&hello()) {
        tracing::error!(error = %e, "cannot send the load hello");
        return EXIT_CONNECT;
    }
    match crate::sys::full_topology() {
        Ok(t) => {
            if let Err(e) = conn.send(&LoadMessage::Topology(t)) {
                tracing::error!(error = %e, "cannot send the topology");
                return EXIT_CONNECT;
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "cannot read the topology");
            return EXIT_CONNECT;
        }
    }
    tracing::info!("connected to the app");

    let (tx, events) = mpsc::channel();
    let reader = conn.start_reader(move |event| tx.send(event).is_ok());
    let code = loop {
        match events.recv() {
            Ok(PipeEvent::Message(msg)) => match route(msg) {
                Route::Run(_) => {
                    // The engine is A8; until then every plan fails at once.
                    tracing::warn!("no phase engine yet: the plan is refused");
                    if let Err(e) = conn.send(&failed_finish()) {
                        tracing::warn!(error = %e, "cannot send Finished");
                        break EXIT_OK;
                    }
                }
                Route::Stop | Route::Ignore => {}
                Route::Exit(code) => break code,
            },
            Ok(PipeEvent::Closed(reason)) => {
                match reason {
                    CloseReason::Disconnected | CloseReason::Stopped => {
                        tracing::info!("the app closed the load pipe")
                    }
                    other => tracing::warn!(reason = ?other, "the load pipe broke"),
                }
                break EXIT_OK;
            }
            Err(_) => break EXIT_OK,
        }
    };
    reader.stop();
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_ipc::load::StopRequest;

    fn hello_v(v: u32) -> LoadMessage {
        LoadMessage::Hello(LoadHello {
            protocol_version: v,
            version: "0.5.0".into(),
            isa: vec![],
        })
    }

    #[test]
    fn incompatible_hello_exits_with_code_3() {
        assert_eq!(
            route(hello_v(LOAD_PROTOCOL_VERSION + 1)),
            Route::Exit(EXIT_INCOMPATIBLE)
        );
        assert_eq!(route(hello_v(LOAD_PROTOCOL_VERSION)), Route::Ignore);
    }

    #[test]
    fn stop_is_routed_and_empty_plan_is_usage() {
        assert_eq!(route(LoadMessage::Stop(StopRequest {})), Route::Stop);
        let empty = LoadMessage::Run(oma_ipc::load::RunRequest {
            plan: oma_ipc::load::Plan {
                seed: 1,
                ram_bytes: 0,
                phases: vec![],
            },
        });
        assert_eq!(route(empty), Route::Exit(EXIT_USAGE));
    }

    #[test]
    fn sse2_is_always_detected() {
        #[cfg(target_arch = "x86_64")]
        assert_eq!(detected_isa().last(), Some(&Isa::Sse2));
    }
}
