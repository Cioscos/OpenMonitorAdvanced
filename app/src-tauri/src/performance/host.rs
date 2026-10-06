//! The `oma-load.exe` child process (M8a1): spawn, private pipe, handshake.
//!
//! [`LoadHost::start`] follows the overlay host (`overlay::host`):
//!
//! 1. a fresh pipe name and the only instance of that pipe (user-only DACL);
//! 2. `oma-load.exe --pipe <name>` from the app's folder, without a console,
//!    put in a kill-on-close Job Object at once (it dies with the app);
//! 3. `accept` for at most 5 s in 250 ms slices; the client PID must be the
//!    child's. On a mismatch nothing is sent and the pipe is dropped;
//! 4. `Hello` both ways, then the `Topology`, within 5 s.
//!
//! After that the pipe reader forwards every message as [`HostEvent::Message`]
//! and `Closed` as [`HostEvent::Closed`]; a thread waiting for the child sends
//! [`HostEvent::Exited`].

use std::fmt;
use std::path::{Path, PathBuf};

/// The helper's file name, next to `oma-app.exe`.
pub const LOAD_EXE: &str = "oma-load.exe";

/// `oma-load.exe` in the folder of the app's executable `current_exe`.
pub fn load_exe(current_exe: &Path) -> PathBuf {
    current_exe.with_file_name(LOAD_EXE)
}

/// The `--inject-fault` value for debug builds (`OMA_LOAD_INJECT`); release builds never
/// inject.
pub fn inject_from_env() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        std::env::var("OMA_LOAD_INJECT")
            .ok()
            .filter(|v| !v.is_empty())
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// Why the helper could not be started.
#[derive(Debug)]
pub enum StartFailure {
    /// `oma-load.exe` is not there.
    Missing,
    /// The process, its pipe or its job could not be created, or it exited or closed the pipe
    /// during the handshake, or our `Hello` could not be sent.
    Spawn(std::io::Error),
    /// No connection or no `Hello` within 5 s, the child still running.
    Timeout,
    /// The pipe client is not the process we spawned.
    ForeignClient,
    /// The helper speaks another protocol version.
    Incompatible,
    /// The `Hello` came, the `Topology` did not.
    NoTopology,
}

impl StartFailure {
    /// The i18n key of the reason shown in `failed_to_start`.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::Missing => "performance.start.missing",
            Self::Spawn(_) => "performance.start.spawn",
            Self::Timeout => "performance.start.timeout",
            Self::ForeignClient => "performance.start.foreign_client",
            Self::Incompatible => "performance.start.incompatible",
            Self::NoTopology => "performance.start.no_topology",
        }
    }
}

impl fmt::Display for StartFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "{LOAD_EXE} not found"),
            Self::Spawn(e) => write!(f, "cannot start {LOAD_EXE}: {e}"),
            Self::Timeout => write!(f, "{LOAD_EXE} did not answer in time"),
            Self::ForeignClient => write!(f, "the load pipe client is not {LOAD_EXE}"),
            Self::Incompatible => write!(f, "{LOAD_EXE} speaks another protocol version"),
            Self::NoTopology => write!(f, "{LOAD_EXE} sent no topology"),
        }
    }
}

impl std::error::Error for StartFailure {}

#[cfg(windows)]
#[allow(unused_imports)] // removed in A20
pub use imp::{HostEvent, LoadHost};

#[cfg(windows)]
mod imp {
    use std::cell::Cell;
    use std::io;
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{self, RecvTimeoutError, Sender};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use oma_ipc::load::{
        load_compatible, CacheSizes, LoadHello, LoadMessage, Topology, LOAD_PROTOCOL_VERSION,
    };
    use oma_win::job::KillOnCloseJob;
    use oma_win::load_pipe::{create_load_server, random_load_pipe_name, LoadConnection};
    use oma_win::private_pipe::{PipeEvent, PipeReader};

    use super::StartFailure;
    use crate::overlay::host::check_client;

    /// `CREATE_NO_WINDOW`: the child gets no console.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const ACCEPT_TIMEOUT: Duration = Duration::from_secs(5);
    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
    const SLICE: Duration = Duration::from_millis(250);

    /// What the helper does after the handshake. `Exited` may still arrive after `kill()` or
    /// `Drop`: consumers ignore the events of a host they let go.
    #[derive(Debug)]
    pub enum HostEvent {
        /// A message that passed `validate()`.
        Message(LoadMessage),
        /// The pipe closed (the helper exited or was killed).
        Closed,
        /// The helper process exited, with its exit code if known.
        Exited(Option<i32>),
    }

    /// A started `oma-load.exe` with its handshake done.
    pub struct LoadHost {
        conn: Option<LoadConnection>,
        reader: Option<PipeReader>,
        job: Option<KillOnCloseJob>,
        hello: LoadHello,
        topology: Topology,
    }

    /// A started child that is gone before the handshake ends: `Spawn`, exit code logged.
    fn exited(child: &mut Child) -> StartFailure {
        std::thread::sleep(Duration::from_millis(50));
        let code = child.try_wait().ok().flatten().and_then(|s| s.code());
        tracing::warn!(?code, "oma-load closed the pipe during the handshake");
        StartFailure::Spawn(io::Error::other("oma-load exited during the handshake"))
    }

    /// Turns a live pipe event into a host event. A message that fails `validate()` is dropped,
    /// with at most one warning per second (`last_warn`).
    pub(super) fn forward(
        event: PipeEvent<LoadMessage>,
        last_warn: &Cell<Option<Instant>>,
    ) -> Option<HostEvent> {
        match event {
            PipeEvent::Closed(_) => Some(HostEvent::Closed),
            PipeEvent::Message(m) => match m.validate() {
                Ok(()) => Some(HostEvent::Message(m)),
                Err(e) => {
                    let now = Instant::now();
                    if last_warn
                        .get()
                        .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1))
                    {
                        last_warn.set(Some(now));
                        tracing::warn!(error = %e, "invalid load message dropped");
                    }
                    None
                }
            },
        }
    }

    /// Kills and reaps a child we are giving up on.
    fn discard(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    impl LoadHost {
        /// Starts the helper at `exe` (see [`super::load_exe`]) and completes the handshake;
        /// `inject` becomes `--inject-fault <v>`. Later messages go to `events`.
        pub fn start(
            exe: &Path,
            inject: Option<String>,
            events: Sender<HostEvent>,
        ) -> Result<Self, StartFailure> {
            if !exe.is_file() {
                return Err(StartFailure::Missing);
            }
            let name = random_load_pipe_name().map_err(StartFailure::Spawn)?;
            let server = create_load_server(&name).map_err(StartFailure::Spawn)?;
            let job = KillOnCloseJob::new().map_err(StartFailure::Spawn)?;
            let mut cmd = Command::new(exe);
            cmd.args(["--pipe", &name]);
            if let Some(v) = &inject {
                cmd.args(["--inject-fault", v]);
            }
            let mut child = cmd
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(StartFailure::Spawn)?;
            if let Err(e) = job.assign(&child) {
                discard(&mut child);
                return Err(StartFailure::Spawn(e));
            }

            let deadline = Instant::now() + ACCEPT_TIMEOUT;
            let client_pid = loop {
                match server.accept(SLICE) {
                    Ok(pid) => break pid,
                    Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
                    Err(e) => {
                        discard(&mut child);
                        return Err(StartFailure::Spawn(e));
                    }
                }
                if !matches!(child.try_wait(), Ok(None)) || Instant::now() >= deadline {
                    discard(&mut child);
                    return Err(StartFailure::Timeout);
                }
            };
            // The PID is compared while we own the running child, so it cannot have been
            // reused. A foreign client gets nothing: the server is dropped here, and
            // `into_connection` is never called for it.
            if !matches!(child.try_wait(), Ok(None))
                || check_client(child.id(), client_pid).is_err()
            {
                discard(&mut child);
                return Err(StartFailure::ForeignClient);
            }

            let conn: LoadConnection = server.into_connection();
            // `Some` during the handshake: the reader hands events to it. Going live takes
            // the sender under the lock and drains what is left, so nothing is lost or reordered.
            let (hs_tx, hs_rx) = mpsc::channel();
            let gate = Arc::new(Mutex::new(Some(hs_tx)));
            let reader = {
                let (gate, events) = (Arc::clone(&gate), events.clone());
                let last_warn = Cell::new(None);
                conn.start_reader(move |event| {
                    let gate = gate.lock().unwrap_or_else(PoisonError::into_inner);
                    if let Some(hs) = gate.as_ref() {
                        return hs.send(event).is_ok();
                    }
                    match forward(event, &last_warn) {
                        Some(ev) => events.send(ev).is_ok(),
                        None => true,
                    }
                })
            };
            // On any failure below `host` drops, and the job kills the child.
            let mut host = Self {
                conn: Some(conn),
                reader: Some(reader),
                job: Some(job),
                hello: LoadHello {
                    protocol_version: 0,
                    version: String::new(),
                    isa: Vec::new(),
                },
                topology: Topology {
                    logical: Vec::new(),
                    caches: CacheSizes {
                        l1d_bytes: 0,
                        l2_bytes: 0,
                        l2_shared_by: 0,
                        l3_bytes: 0,
                        l3_total_bytes: 0,
                    },
                    hypervisor: false,
                    vendor: String::new(),
                    brand: String::new(),
                },
            };
            let ours = LoadMessage::Hello(LoadHello {
                protocol_version: LOAD_PROTOCOL_VERSION,
                version: env!("CARGO_PKG_VERSION").to_owned(),
                isa: Vec::new(),
            });
            if let Err(e) = host.send(&ours) {
                tracing::warn!(error = %e, "cannot send the hello to oma-load");
                return Err(StartFailure::Spawn(e));
            }

            let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
            let mut hello = None;
            let topology = loop {
                let left = deadline.saturating_duration_since(Instant::now());
                let event = match hs_rx.recv_timeout(left) {
                    Ok(e) => e,
                    Err(RecvTimeoutError::Timeout) => {
                        return Err(if hello.is_some() {
                            StartFailure::NoTopology
                        } else {
                            StartFailure::Timeout
                        });
                    }
                    Err(RecvTimeoutError::Disconnected) => return Err(exited(&mut child)),
                };
                let PipeEvent::Message(msg) = event else {
                    return Err(exited(&mut child));
                };
                if let Err(e) = msg.validate() {
                    tracing::warn!(error = %e, "invalid load message dropped");
                    continue;
                }
                match (msg, hello.is_some()) {
                    (LoadMessage::Hello(h), false) => {
                        if !load_compatible(&h) {
                            return Err(StartFailure::Incompatible);
                        }
                        hello = Some(h);
                    }
                    (LoadMessage::Topology(t), true) => break t,
                    _ => tracing::warn!("unexpected load message during the handshake ignored"),
                }
            };
            host.hello = hello.expect("hello seen before the topology");
            host.topology = topology;
            {
                let mut gate = gate.lock().unwrap_or_else(PoisonError::into_inner);
                gate.take();
                let last_warn = Cell::new(None);
                while let Ok(event) = hs_rx.try_recv() {
                    if let Some(ev) = forward(event, &last_warn) {
                        let _ = events.send(ev);
                    }
                }
            }

            let exit_tx = events.clone();
            let spawned = std::thread::Builder::new()
                .name("oma-load-wait".into())
                .spawn(move || {
                    let code = child.wait().ok().and_then(|s| s.code());
                    let _ = exit_tx.send(HostEvent::Exited(code));
                });
            if let Err(e) = spawned {
                tracing::error!(error = %e, "cannot start the oma-load exit watcher");
                let _ = events.send(HostEvent::Exited(None));
            }
            Ok(host)
        }

        pub fn send(&self, msg: &LoadMessage) -> io::Result<()> {
            match &self.conn {
                Some(conn) => conn.send(msg),
                None => Err(io::ErrorKind::BrokenPipe.into()),
            }
        }

        pub fn topology(&self) -> &Topology {
            &self.topology
        }

        pub fn hello(&self) -> &LoadHello {
            &self.hello
        }

        /// Closes the pipe (reader included) and the Job, which kills the helper.
        pub fn kill(&mut self) {
            if let Some(reader) = self.reader.take() {
                reader.stop();
            }
            // The reader is gone: the connection was the last holder of the pipe.
            drop(self.conn.take());
            drop(self.job.take());
        }
    }

    impl Drop for LoadHost {
        fn drop(&mut self) {
            self.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_exe_is_next_to_the_app() {
        assert_eq!(
            load_exe(Path::new(
                r"C:\Program Files\OpenMonitor Advanced\oma-app.exe"
            )),
            PathBuf::from(r"C:\Program Files\OpenMonitor Advanced\oma-load.exe")
        );
    }

    #[cfg(windows)]
    #[test]
    fn missing_exe_is_missing() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let r = imp::LoadHost::start(Path::new(r"C:\no\such\oma-load.exe"), None, tx);
        assert!(matches!(r, Err(StartFailure::Missing)));
    }

    #[test]
    fn foreign_client_pid_is_rejected() {
        use crate::overlay::host::check_client;
        assert!(check_client(10, 10).is_ok());
        assert!(check_client(10, 11).is_err());
    }

    #[test]
    fn every_failure_has_a_distinct_key() {
        let mut keys = vec![
            StartFailure::Missing.i18n_key(),
            StartFailure::Spawn(std::io::Error::other("x")).i18n_key(),
            StartFailure::Timeout.i18n_key(),
            StartFailure::ForeignClient.i18n_key(),
            StartFailure::Incompatible.i18n_key(),
            StartFailure::NoTopology.i18n_key(),
        ];
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), 6);
    }

    #[cfg(windows)]
    #[test]
    fn invalid_live_messages_are_dropped() {
        use oma_ipc::load::{LoadMessage, Progress};
        use oma_win::private_pipe::PipeEvent;
        use std::cell::Cell;
        let mut p = Progress {
            phase: 0,
            phase_elapsed_ms: 0,
            elapsed_ms: 0,
            checks: 0,
            errors: 0,
            current_core: None,
            cores: vec![],
            memory_bytes: 0,
            rate: Some(1.0),
        };
        let w = Cell::new(None);
        let ok = imp::forward(PipeEvent::Message(LoadMessage::Progress(p.clone())), &w);
        assert!(matches!(ok, Some(imp::HostEvent::Message(_))));
        p.rate = Some(f64::NAN);
        assert!(imp::forward(PipeEvent::Message(LoadMessage::Progress(p)), &w).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn real_host_runs_a_two_second_plan() {
        use oma_ipc::load::*;
        use std::sync::mpsc;
        use std::time::Duration;

        let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join(r"..\..\target\debug\oma-load.exe");
        if !exe.is_file() {
            eprintln!("skipped: build oma-load first (cargo build -p oma-load)");
            return;
        }
        let (tx, rx) = mpsc::channel();
        let host = imp::LoadHost::start(&exe, None, tx).expect("start");
        assert_eq!(host.hello().protocol_version, LOAD_PROTOCOL_VERSION);
        let mut cores: Vec<u32> = Vec::new();
        for l in &host.topology().logical {
            if !cores.contains(&l.core) && cores.len() < 2 {
                cores.push(l.core);
            }
        }
        assert!(!cores.is_empty());
        let plan = Plan {
            seed: 1,
            ram_bytes: 0,
            phases: vec![Phase {
                kernel: KernelId::K5,
                alt_kernel: None,
                isa: host.hello().isa[0],
                size: DataSize::L2,
                mode: LoadMode::Steady,
                placement: Placement::OnePerCore,
                duration_s: 2,
                per_core_s: None,
                both_smt: false,
                cores: Some(cores),
                patterns: vec![],
                stop_on_error: false,
            }],
        };
        host.send(&LoadMessage::Run(RunRequest { plan })).unwrap();
        loop {
            match rx.recv_timeout(Duration::from_secs(15)).expect("event") {
                imp::HostEvent::Message(LoadMessage::Finished(f)) => {
                    assert_eq!(f.reason, FinishReason::Completed);
                    break;
                }
                imp::HostEvent::Message(_) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
        drop(host);
    }
}
