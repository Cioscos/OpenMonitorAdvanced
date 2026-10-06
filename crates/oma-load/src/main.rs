//! `oma-load.exe`: the load generator the app starts for a stress test
//! (M8a1). It exists only while a test runs: it connects to the app's private
//! pipe with `--pipe <name>`, sends its `Hello` and the topology, runs the
//! plan it receives and exits when the pipe closes.
//!
//! Exit codes: 0 pipe closed, 1 arguments (or an invalid message), 2
//! connection, 3 incompatible `Hello`.

#![windows_subsystem = "windows"]

use oma_load::{args, link, log};

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
            return link::EXIT_USAGE;
        }
    };
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "oma-load starting");
    #[cfg(windows)]
    {
        link::run(&args.pipe, args.inject)
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        link::EXIT_CONNECT
    }
}
