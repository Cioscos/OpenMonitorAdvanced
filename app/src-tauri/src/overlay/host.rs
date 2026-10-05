//! Supervision of the `oma-overlay.exe` child process (M7c).
//!
//! [`Supervisor`] is the pure lifecycle: when to start the overlay, when to
//! stop it, the restart backoff (1, 2, 4… s up to 60 s, back to 1 s after a
//! healthy minute) and the crash budget (5 unexpected exits in 10 minutes
//! give `Failed { Crashing }`). [`OverlayHost`] (Windows) runs it on the
//! `oma-overlay-host` thread:
//!
//! 1. a fresh random pipe name and the only instance of that pipe
//!    ([`OverlayPipeServer::create`]: user-only DACL, first instance, no
//!    remote clients);
//! 2. `oma-overlay.exe --pipe <name>`, from the app's own folder, without a
//!    console;
//! 3. `accept` for at most 5 s, then the client PID must be the PID of the
//!    child we spawned, compared while we still own the child and it has not
//!    exited (so its PID cannot have been reused). Nothing is sent before
//!    that: on a mismatch the child is killed, the pipe is dropped and the
//!    exit counts as a crash, so the next start uses a new name;
//! 4. `Hello` both ways within 5 s; an incompatible one fails at once.
//!
//! [`OverlayHost::send`] never blocks: messages go to a bounded queue (64)
//! drained by the `oma-overlay-writer` thread, and when it is full `Values`
//! and `FrameTimes` go first. The queue only takes messages while the
//! overlay is `Running`; the controller sends the profile again on each
//! `Running` it is told about.
//!
//! The overlay's exit codes: 0 pipe closed, 1 bad arguments, 2 connection
//! failed, 3 incompatible `Hello`, 4 device lost or window failure. Every
//! exit we did not ask for is a crash, except 3, which is `Incompatible`.
//!
//! [`OverlayPipeServer::create`]: oma_win::overlay_pipe::OverlayPipeServer::create

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};

use oma_ipc::overlay::{overlay_compatible, OverlayMessage};

/// The overlay's file name, next to `oma-app.exe`.
pub const OVERLAY_EXE: &str = "oma-overlay.exe";

/// First restart delay after a crash.
const FIRST_RETRY_MS: u64 = 1_000;
/// Longest restart delay.
const MAX_RETRY_MS: u64 = 60_000;
/// A process that ran longer than this before it fell resets the backoff.
const HEALTHY_RUN_MS: u64 = 60_000;
/// Crashes within [`CRASH_WINDOW_MS`] that give `Failed { Crashing }`.
const CRASHES_TO_FAIL: usize = 5;
const CRASH_WINDOW_MS: u64 = 10 * 60_000;

/// Messages waiting for the writer.
const QUEUE_CAPACITY: usize = 64;

/// The overlay exits with this code after an incompatible `Hello`.
const EXIT_INCOMPATIBLE: i32 = 3;

/// Why the overlay process stays down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostFailure {
    /// Too many crashes in ten minutes.
    Crashing,
    /// The overlay speaks another protocol version.
    Incompatible,
}

/// The overlay process as the controller sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostState {
    /// Not wanted.
    Off,
    /// Wanted: being started, or waiting for the next attempt.
    Starting,
    /// Connected and past the handshake.
    Running,
    /// Wanted but given up on, until `retry` or a new `set_wanted(true)`.
    Failed { reason: HostFailure },
}

/// What the runner must do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorAction {
    Start,
    Stop,
}

/// The overlay's lifecycle, without processes or clocks: the runner feeds it
/// the time (monotonic milliseconds) and what happened, and polls it for
/// what to do.
#[derive(Debug)]
pub struct Supervisor {
    wanted: bool,
    /// A process was started and has not been reported gone.
    alive: bool,
    /// The live process finished its handshake.
    running: bool,
    /// When the live process was started.
    started_at: u64,
    failed: Option<HostFailure>,
    next_start_at: u64,
    next_delay_ms: u64,
    /// Times of the recent crashes, oldest first.
    crashes: VecDeque<u64>,
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Supervisor {
    pub fn new() -> Self {
        Self {
            wanted: false,
            alive: false,
            running: false,
            started_at: 0,
            failed: None,
            next_start_at: 0,
            next_delay_ms: FIRST_RETRY_MS,
            crashes: VecDeque::new(),
        }
    }

    /// Turns the overlay on or off. Turning it on after it was off starts
    /// afresh: no failure, no crash history, no backoff.
    pub fn set_wanted(&mut self, wanted: bool, now_ms: u64) {
        if wanted && !self.wanted {
            self.reset(now_ms);
        }
        self.wanted = wanted;
    }

    /// The process exited (or was killed) without being asked to: one
    /// crash. Ignored when no process is expected, so the exit of a wanted
    /// `Stop` never counts.
    pub fn on_exit(&mut self, now_ms: u64) {
        if !self.alive {
            return;
        }
        self.alive = false;
        if self.running && now_ms.saturating_sub(self.started_at) > HEALTHY_RUN_MS {
            self.next_delay_ms = FIRST_RETRY_MS;
        }
        self.running = false;
        self.crashes.push_back(now_ms);
        while self
            .crashes
            .front()
            .is_some_and(|&t| now_ms.saturating_sub(t) >= CRASH_WINDOW_MS)
        {
            self.crashes.pop_front();
        }
        if self.crashes.len() >= CRASHES_TO_FAIL {
            self.failed = Some(HostFailure::Crashing);
            return;
        }
        self.next_start_at = now_ms + self.next_delay_ms;
        self.next_delay_ms = (self.next_delay_ms * 2).min(MAX_RETRY_MS);
    }

    /// The process speaks another protocol: no restart until `retry` or a
    /// new `set_wanted(true)`.
    pub fn on_incompatible(&mut self) {
        self.alive = false;
        self.running = false;
        self.failed = Some(HostFailure::Incompatible);
    }

    /// The process finished its handshake.
    pub fn on_running(&mut self) {
        if self.alive {
            self.running = true;
        }
    }

    /// «Retry» after a failure: starts afresh, like a new `set_wanted(true)`.
    pub fn retry(&mut self, now_ms: u64) {
        if self.failed.is_some() {
            self.reset(now_ms);
        }
    }

    /// The next action, at most one per call.
    pub fn poll(&mut self, now_ms: u64) -> Option<SupervisorAction> {
        if !self.wanted && self.alive {
            self.alive = false;
            self.running = false;
            return Some(SupervisorAction::Stop);
        }
        if self.next_start_ms().is_some_and(|at| now_ms >= at) {
            self.alive = true;
            self.running = false;
            self.started_at = now_ms;
            return Some(SupervisorAction::Start);
        }
        None
    }

    /// When the next `Start` is due, if one is waiting.
    pub fn next_start_ms(&self) -> Option<u64> {
        (self.wanted && !self.alive && self.failed.is_none()).then_some(self.next_start_at)
    }

    pub fn state(&self) -> HostState {
        if !self.wanted {
            HostState::Off
        } else if let Some(reason) = self.failed {
            HostState::Failed { reason }
        } else if self.alive && self.running {
            HostState::Running
        } else {
            HostState::Starting
        }
    }

    fn reset(&mut self, now_ms: u64) {
        self.failed = None;
        self.crashes.clear();
        self.next_delay_ms = FIRST_RETRY_MS;
        self.next_start_at = now_ms;
    }
}

/// Errors of the overlay host.
#[derive(Debug)]
pub enum HostError {
    /// The pipe client is not the overlay we spawned.
    ForeignClient { expected: u32, actual: u32 },
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ForeignClient { expected, actual } => write!(
                f,
                "the overlay pipe client is process {actual}, not the overlay {expected}"
            ),
        }
    }
}

impl std::error::Error for HostError {}

/// Accepts the pipe client only when it is the process we spawned.
pub(crate) fn check_client(expected_pid: u32, client_pid: u32) -> Result<(), HostError> {
    if expected_pid == client_pid {
        Ok(())
    } else {
        Err(HostError::ForeignClient {
            expected: expected_pid,
            actual: client_pid,
        })
    }
}

/// `oma-overlay.exe` in the folder of the app's executable `current_exe`.
pub(crate) fn overlay_exe(current_exe: &Path) -> PathBuf {
    current_exe.with_file_name(OVERLAY_EXE)
}

/// What a message received during the handshake means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelloOutcome {
    Compatible,
    Incompatible,
    NotHello,
}

/// Classifies a received (already validated) message for the handshake.
pub(crate) fn hello_outcome(msg: &OverlayMessage) -> HelloOutcome {
    match msg {
        OverlayMessage::Hello(hello) if overlay_compatible(hello) => HelloOutcome::Compatible,
        OverlayMessage::Hello(_) => HelloOutcome::Incompatible,
        _ => HelloOutcome::NotHello,
    }
}

/// How an exit we did not ask for is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitKind {
    Crash,
    Incompatible,
}

/// Classifies the overlay's exit code (`None`: unknown, or we killed it).
pub(crate) fn exit_kind(code: Option<i32>) -> ExitKind {
    if code == Some(EXIT_INCOMPATIBLE) {
        ExitKind::Incompatible
    } else {
        ExitKind::Crash
    }
}

/// Data messages: only the newest matter, so they are dropped first.
fn is_droppable(msg: &OverlayMessage) -> bool {
    matches!(
        msg,
        OverlayMessage::Values(_) | OverlayMessage::FrameTimes(_)
    )
}

/// The bounded queue between [`OverlayHost::send`] and the writer thread.
pub(crate) struct SendQueue {
    items: VecDeque<OverlayMessage>,
}

impl SendQueue {
    pub(crate) fn new() -> Self {
        Self {
            items: VecDeque::with_capacity(QUEUE_CAPACITY),
        }
    }

    /// Queues `msg`. When the queue is full, the oldest data message makes
    /// room; without one, a new data message is dropped (`false`) and a
    /// control message replaces the oldest message.
    pub(crate) fn push(&mut self, msg: OverlayMessage) -> bool {
        if self.items.len() >= QUEUE_CAPACITY {
            if let Some(i) = self.items.iter().position(is_droppable) {
                self.items.remove(i);
            } else if is_droppable(&msg) {
                return false;
            } else {
                self.items.pop_front();
            }
        }
        self.items.push_back(msg);
        true
    }

    pub(crate) fn pop(&mut self) -> Option<OverlayMessage> {
        self.items.pop_front()
    }

    pub(crate) fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(windows)]
// Used by the overlay controller from C16 on.
#[allow(unused_imports)]
pub use imp::OverlayHost;

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
    use std::sync::{Arc, Condvar, Mutex, PoisonError};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use oma_ipc::overlay::{OverlayHello, OverlayMessage, OVERLAY_PROTOCOL_VERSION};
    use oma_win::overlay_pipe::{
        random_pipe_name, OverlayConnection, OverlayPipeServer, PipeEvent, PipeReader,
    };

    use super::{
        check_client, exit_kind, hello_outcome, ExitKind, HelloOutcome, HostState, SendQueue,
        Supervisor, SupervisorAction,
    };

    /// `CREATE_NO_WINDOW`: the child gets no console.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    /// How long the spawned overlay has to connect.
    const ACCEPT_TIMEOUT: Duration = Duration::from_secs(5);
    /// How long the overlay has to send its `Hello` once connected.
    const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
    /// How long a stopped overlay has to exit before it is killed.
    const EXIT_WAIT: Duration = Duration::from_secs(2);
    /// Granularity of the waits that also watch the child and the commands.
    const SLICE: Duration = Duration::from_millis(250);

    /// What the host thread reacts to.
    enum Event {
        Wanted(bool),
        Retry,
        Shutdown,
        /// From the pipe reader of session `session`.
        Pipe {
            session: u64,
            event: Box<PipeEvent<OverlayMessage>>,
        },
        /// The writer of session `session` could not send.
        WriteFailed {
            session: u64,
        },
    }

    /// The send queue, open only while a session is running.
    struct Outbox {
        state: Mutex<OutboxState>,
        ready: Condvar,
    }

    struct OutboxState {
        open: bool,
        queue: SendQueue,
    }

    impl Outbox {
        fn set_open(&self, open: bool) {
            let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            s.open = open;
            s.queue.clear();
            drop(s);
            self.ready.notify_all();
        }

        /// The next message to write, or `None` once the outbox is closed.
        fn next(&self) -> Option<OverlayMessage> {
            let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if !s.open {
                    return None;
                }
                if let Some(msg) = s.queue.pop() {
                    return Some(msg);
                }
                s = self.ready.wait(s).unwrap_or_else(PoisonError::into_inner);
            }
        }
    }

    /// Starts, watches and stops `oma-overlay.exe` (see the module docs).
    pub struct OverlayHost {
        events: Sender<Event>,
        outbox: Arc<Outbox>,
        state: Arc<Mutex<HostState>>,
        thread: Option<JoinHandle<()>>,
    }

    impl OverlayHost {
        /// Starts the `oma-overlay-host` thread for the overlay at `exe`
        /// (see [`super::overlay_exe`]); the overlay itself starts on
        /// `set_wanted(true)`. `on_state` is called on that thread with
        /// every new state, and must not block.
        pub fn start(
            exe: PathBuf,
            on_state: impl Fn(HostState) + Send + 'static,
        ) -> io::Result<Self> {
            let (events_tx, events) = mpsc::channel();
            let outbox = Arc::new(Outbox {
                state: Mutex::new(OutboxState {
                    open: false,
                    queue: SendQueue::new(),
                }),
                ready: Condvar::new(),
            });
            let state = Arc::new(Mutex::new(HostState::Off));
            let runner = Runner {
                exe,
                events,
                events_tx: events_tx.clone(),
                outbox: Arc::clone(&outbox),
                shared_state: Arc::clone(&state),
                on_state: Box::new(on_state),
                reported: HostState::Off,
                sup: Supervisor::new(),
                epoch: Instant::now(),
                session_id: 0,
                session: None,
                shutdown: false,
            };
            let thread = std::thread::Builder::new()
                .name("oma-overlay-host".into())
                .spawn(move || runner.run())?;
            Ok(Self {
                events: events_tx,
                outbox,
                state,
                thread: Some(thread),
            })
        }

        /// Turns the overlay process on or off.
        pub fn set_wanted(&self, wanted: bool) {
            let _ = self.events.send(Event::Wanted(wanted));
        }

        /// «Retry» after `Failed`.
        pub fn retry(&self) {
            let _ = self.events.send(Event::Retry);
        }

        /// Queues `msg` for the overlay without blocking. Dropped unless the
        /// overlay is `Running`; when the queue is full, data messages go
        /// first.
        pub fn send(&self, msg: OverlayMessage) {
            let mut s = self
                .outbox
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if !s.open {
                return;
            }
            if !s.queue.push(msg) {
                tracing::debug!("overlay send queue full: data message dropped");
            }
            drop(s);
            self.outbox.ready.notify_one();
        }

        pub fn state(&self) -> HostState {
            *self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }

        /// Stops the overlay (closes the pipe, waits 2 s for it to exit,
        /// then kills it) and the host thread.
        pub fn stop(mut self) {
            self.shutdown();
        }

        fn shutdown(&mut self) {
            if let Some(thread) = self.thread.take() {
                let _ = self.events.send(Event::Shutdown);
                let _ = thread.join();
            }
        }
    }

    impl Drop for OverlayHost {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    /// A started overlay and its connection.
    struct Session {
        child: Child,
        conn: Option<Arc<OverlayConnection>>,
        reader: Option<PipeReader>,
        writer: Option<JoinHandle<()>>,
    }

    /// How a start attempt ended.
    enum Launch {
        Running(Session),
        Failed(ExitKind),
        /// Stopped, or the app is closing: nothing to count.
        Aborted,
    }

    /// The host thread's state.
    struct Runner {
        exe: PathBuf,
        events: Receiver<Event>,
        events_tx: Sender<Event>,
        outbox: Arc<Outbox>,
        shared_state: Arc<Mutex<HostState>>,
        on_state: Box<dyn Fn(HostState) + Send>,
        reported: HostState,
        sup: Supervisor,
        epoch: Instant,
        /// Numbers the sessions, so events of an old one are ignored.
        session_id: u64,
        session: Option<Session>,
        shutdown: bool,
    }

    impl Runner {
        fn now_ms(&self) -> u64 {
            u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
        }

        fn run(mut self) {
            loop {
                if self.shutdown {
                    self.end_session();
                    let now = self.now_ms();
                    self.sup.set_wanted(false, now);
                    self.report();
                    return;
                }
                let now = self.now_ms();
                match self.sup.poll(now) {
                    Some(SupervisorAction::Start) => {
                        self.report();
                        self.launch();
                        self.report();
                        continue;
                    }
                    Some(SupervisorAction::Stop) => self.end_session(),
                    None => {}
                }
                self.report();

                let event = if self.session.is_some() {
                    self.events.recv_timeout(SLICE)
                } else if let Some(at) = self.sup.next_start_ms() {
                    let wait = at.saturating_sub(self.now_ms());
                    self.events.recv_timeout(Duration::from_millis(wait))
                } else {
                    self.events
                        .recv()
                        .map_err(|_| RecvTimeoutError::Disconnected)
                };
                match event {
                    Ok(event) => self.handle(event),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => self.shutdown = true,
                }
                if let Some(session) = self.session.as_mut() {
                    if !matches!(session.child.try_wait(), Ok(None)) {
                        tracing::warn!("the overlay process exited");
                        self.lose_session();
                    }
                }
            }
        }

        /// Hands a new state to the controller.
        fn report(&mut self) {
            let state = self.sup.state();
            if state != self.reported {
                self.reported = state;
                *self
                    .shared_state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = state;
                (self.on_state)(state);
            }
        }

        /// Applies a command; `true` when it calls off a start in progress.
        fn apply(&mut self, event: Event) -> bool {
            let now = self.now_ms();
            match event {
                Event::Wanted(wanted) => {
                    self.sup.set_wanted(wanted, now);
                    !wanted
                }
                Event::Retry => {
                    self.sup.retry(now);
                    false
                }
                Event::Shutdown => {
                    self.shutdown = true;
                    true
                }
                // Pipe events of an old session.
                Event::Pipe { .. } | Event::WriteFailed { .. } => false,
            }
        }

        fn handle(&mut self, event: Event) {
            let current = self.session_id;
            match event {
                Event::Pipe { session, event } if session == current => match *event {
                    PipeEvent::Closed(reason) => {
                        if self.session.is_some() {
                            tracing::warn!(?reason, "the overlay pipe closed");
                            self.lose_session();
                        }
                    }
                    PipeEvent::Message(msg) => match msg.validate() {
                        Ok(()) => tracing::debug!("unexpected overlay message ignored"),
                        Err(e) => tracing::warn!(error = %e, "invalid overlay message dropped"),
                    },
                },
                Event::WriteFailed { session } if session == current && self.session.is_some() => {
                    self.lose_session();
                }
                other => {
                    self.apply(other);
                }
            }
        }

        /// Ends the running session we did not ask to end, and counts it.
        fn lose_session(&mut self) {
            if let Some(session) = self.session.take() {
                let code = self.teardown(session);
                tracing::warn!(?code, "the overlay stopped unexpectedly");
                self.count(exit_kind(code));
            }
        }

        /// Ends the session the supervisor asked to stop: not counted.
        fn end_session(&mut self) {
            if let Some(session) = self.session.take() {
                let code = self.teardown(session);
                tracing::info!(?code, "overlay stopped");
            }
        }

        fn count(&mut self, kind: ExitKind) {
            match kind {
                ExitKind::Crash => {
                    let now = self.now_ms();
                    self.sup.on_exit(now);
                }
                ExitKind::Incompatible => {
                    tracing::error!("the overlay speaks another protocol version");
                    self.sup.on_incompatible();
                }
            }
        }

        fn launch(&mut self) {
            self.session_id += 1;
            match self.try_launch() {
                Launch::Running(session) => {
                    tracing::info!(pid = session.child.id(), "overlay running");
                    self.session = Some(session);
                    self.sup.on_running();
                }
                Launch::Failed(kind) => self.count(kind),
                Launch::Aborted => {}
            }
        }

        /// Drains the commands that arrived meanwhile; `true` to call off
        /// the start.
        fn drain_commands(&mut self) -> bool {
            let mut abort = false;
            loop {
                match self.events.try_recv() {
                    Ok(event) => abort |= self.apply(event),
                    Err(TryRecvError::Empty) => return abort,
                    Err(TryRecvError::Disconnected) => {
                        self.shutdown = true;
                        return true;
                    }
                }
            }
        }

        fn try_launch(&mut self) -> Launch {
            let name = match random_pipe_name() {
                Ok(name) => name,
                Err(e) => {
                    tracing::error!(error = %e, "cannot draw an overlay pipe name");
                    return Launch::Failed(ExitKind::Crash);
                }
            };
            let server = match OverlayPipeServer::create(&name) {
                Ok(server) => server,
                Err(e) => {
                    tracing::error!(error = %e, "cannot create the overlay pipe");
                    return Launch::Failed(ExitKind::Crash);
                }
            };
            let mut child = match Command::new(&self.exe)
                .arg("--pipe")
                .arg(&name)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
            {
                Ok(child) => child,
                Err(e) => {
                    tracing::error!(error = %e, exe = %self.exe.display(), "cannot start the overlay");
                    return Launch::Failed(ExitKind::Crash);
                }
            };

            // 3. Wait for the client, watching the child and the commands.
            let deadline = Instant::now() + ACCEPT_TIMEOUT;
            let client_pid = loop {
                match server.accept(SLICE) {
                    Ok(pid) => break pid,
                    Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
                    Err(e) => {
                        tracing::error!(error = %e, "overlay pipe accept failed");
                        return Launch::Failed(kill(&mut child));
                    }
                }
                if let Ok(Some(status)) = child.try_wait() {
                    tracing::warn!(code = ?status.code(), "the overlay exited before connecting");
                    return Launch::Failed(exit_kind(status.code()));
                }
                if self.drain_commands() {
                    kill(&mut child);
                    return Launch::Aborted;
                }
                if Instant::now() >= deadline {
                    tracing::error!("the overlay did not connect within 5 s");
                    return Launch::Failed(kill(&mut child));
                }
            };

            // 4. The client must be our child. `Child::id()` is compared while
            // we own the child and it is still running, so its PID has not
            // been reused. On a mismatch nothing is sent: the server is
            // dropped with this function, and the next start draws a new name.
            match child.try_wait() {
                Ok(None) => {}
                Ok(Some(status)) => {
                    tracing::warn!(code = ?status.code(), "the overlay exited while connecting");
                    return Launch::Failed(exit_kind(status.code()));
                }
                Err(e) => {
                    tracing::error!(error = %e, "cannot query the overlay process");
                    return Launch::Failed(kill(&mut child));
                }
            }
            if let Err(e) = check_client(child.id(), client_pid) {
                tracing::error!(error = %e, "overlay pipe client rejected");
                kill(&mut child);
                return Launch::Failed(ExitKind::Crash);
            }

            // 5. Hello both ways.
            let conn = Arc::new(server.into_connection());
            let session_id = self.session_id;
            let tx = self.events_tx.clone();
            let reader = conn.start_reader(move |event| {
                tx.send(Event::Pipe {
                    session: session_id,
                    event: Box::new(event),
                })
                .is_ok()
            });
            let mut session = Session {
                child,
                conn: Some(Arc::clone(&conn)),
                reader: Some(reader),
                writer: None,
            };
            let hello = OverlayMessage::Hello(OverlayHello {
                protocol_version: OVERLAY_PROTOCOL_VERSION,
                version: env!("CARGO_PKG_VERSION").to_owned(),
            });
            if let Err(e) = conn.send(&hello) {
                tracing::error!(error = %e, "cannot send the hello to the overlay");
                self.teardown(session);
                return Launch::Failed(ExitKind::Crash);
            }
            let deadline = Instant::now() + HELLO_TIMEOUT;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    tracing::error!("the overlay sent no hello within 5 s");
                    self.teardown(session);
                    return Launch::Failed(ExitKind::Crash);
                }
                match self.events.recv_timeout(left.min(SLICE)) {
                    Ok(Event::Pipe { session: s, event }) if s == session_id => match *event {
                        PipeEvent::Message(msg) => {
                            if let Err(e) = msg.validate() {
                                tracing::warn!(error = %e, "invalid overlay message dropped");
                                continue;
                            }
                            match hello_outcome(&msg) {
                                HelloOutcome::Compatible => break,
                                HelloOutcome::Incompatible => {
                                    self.teardown(session);
                                    return Launch::Failed(ExitKind::Incompatible);
                                }
                                HelloOutcome::NotHello => {
                                    tracing::warn!("overlay message before its hello ignored")
                                }
                            }
                        }
                        PipeEvent::Closed(reason) => {
                            tracing::warn!(?reason, "the overlay pipe closed during the handshake");
                            let code = self.teardown(session);
                            return Launch::Failed(exit_kind(code));
                        }
                    },
                    Ok(event) => {
                        if self.apply(event) {
                            self.teardown(session);
                            return Launch::Aborted;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if !matches!(session.child.try_wait(), Ok(None)) {
                            let code = self.teardown(session);
                            return Launch::Failed(exit_kind(code));
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        self.shutdown = true;
                        self.teardown(session);
                        return Launch::Aborted;
                    }
                }
            }

            // 6. The writer drains the outbox from now on.
            self.outbox.set_open(true);
            let outbox = Arc::clone(&self.outbox);
            let tx = self.events_tx.clone();
            let spawned = std::thread::Builder::new()
                .name("oma-overlay-writer".into())
                .spawn(move || {
                    while let Some(msg) = outbox.next() {
                        if let Err(e) = conn.send(&msg) {
                            tracing::warn!(error = %e, "cannot write to the overlay");
                            let _ = tx.send(Event::WriteFailed {
                                session: session_id,
                            });
                            return;
                        }
                    }
                });
            match spawned {
                Ok(writer) => session.writer = Some(writer),
                Err(e) => {
                    tracing::error!(error = %e, "cannot start the overlay writer");
                    self.teardown(session);
                    return Launch::Failed(ExitKind::Crash);
                }
            }
            Launch::Running(session)
        }

        /// Closes the outbox and the pipe, gives the overlay [`EXIT_WAIT`]
        /// to exit, then kills it. Returns its exit code, `None` when it
        /// had to be killed.
        fn teardown(&mut self, mut session: Session) -> Option<i32> {
            self.outbox.set_open(false);
            if let Some(writer) = session.writer.take() {
                let _ = writer.join();
            }
            if let Some(reader) = session.reader.take() {
                reader.stop();
            }
            // The last reference: the pipe handle closes here.
            drop(session.conn.take());
            let deadline = Instant::now() + EXIT_WAIT;
            loop {
                match session.child.try_wait() {
                    Ok(Some(status)) => return status.code(),
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50))
                    }
                    _ => {
                        kill(&mut session.child);
                        return None;
                    }
                }
            }
        }
    }

    /// Kills and reaps the child; what is left is a crash.
    fn kill(child: &mut Child) -> ExitKind {
        // `kill` succeeds on a child that already exited; when it fails,
        // waiting could block for good.
        match child.kill() {
            Ok(()) => {
                let _ = child.wait();
            }
            Err(e) => tracing::warn!(error = %e, "cannot kill the overlay"),
        }
        ExitKind::Crash
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use oma_ipc::overlay::{
        FrameTimes, OverlayHello, SetPlacement, Values, OVERLAY_PROTOCOL_VERSION,
    };

    const S: u64 = 1000;
    const MIN: u64 = 60 * S;

    /// A supervisor that was asked to run and has started once at `now`.
    fn started(now: u64) -> Supervisor {
        let mut s = Supervisor::new();
        s.set_wanted(true, now);
        assert_eq!(s.poll(now), Some(SupervisorAction::Start));
        s
    }

    fn hello(version: u32) -> OverlayMessage {
        OverlayMessage::Hello(OverlayHello {
            protocol_version: version,
            version: "0.5.0".into(),
        })
    }

    fn values(at_ms: u64) -> OverlayMessage {
        OverlayMessage::Values(Values {
            at_ms,
            values: Vec::new(),
        })
    }

    fn placement(dpi: u32) -> OverlayMessage {
        OverlayMessage::SetPlacement(SetPlacement { area: None, dpi })
    }

    #[test]
    fn starts_when_wanted() {
        let mut s = Supervisor::new();
        assert_eq!(s.state(), HostState::Off);
        assert_eq!(s.poll(0), None);
        s.set_wanted(true, 10);
        assert_eq!(s.poll(10), Some(SupervisorAction::Start));
        assert_eq!(s.state(), HostState::Starting);
        assert_eq!(s.poll(20), None, "one start per process");
        s.on_running();
        assert_eq!(s.state(), HostState::Running);
    }

    #[test]
    fn restart_backoff_doubles_to_sixty_seconds() {
        let mut now = 0;
        let mut s = started(now);
        for delay_s in [1, 2, 4, 8, 16, 32, 60, 60] {
            // 150 s between exits: never 5 in 10 minutes, never a healthy run.
            now += 150 * S;
            s.on_exit(now);
            assert_eq!(s.state(), HostState::Starting);
            assert_eq!(
                s.poll(now + delay_s * S - 1),
                None,
                "too early for {delay_s} s"
            );
            now += delay_s * S;
            assert_eq!(s.poll(now), Some(SupervisorAction::Start), "{delay_s} s");
        }
    }

    #[test]
    fn healthy_run_resets_the_backoff() {
        let mut now = 0;
        let mut s = started(now);
        for delay_s in [1, 2] {
            now += 150 * S;
            s.on_exit(now);
            now += delay_s * S;
            assert_eq!(s.poll(now), Some(SupervisorAction::Start));
        }
        s.on_running();
        now += 2 * MIN;
        s.on_exit(now);
        assert_eq!(s.poll(now + S), Some(SupervisorAction::Start));
    }

    #[test]
    fn five_exits_in_ten_minutes_fail() {
        let mut now = 0;
        let mut s = started(now);
        for _ in 0..4 {
            now += 10 * S;
            s.on_exit(now);
            now += 60 * S;
            assert_eq!(s.poll(now), Some(SupervisorAction::Start));
        }
        now += 10 * S;
        s.on_exit(now);
        assert_eq!(
            s.state(),
            HostState::Failed {
                reason: HostFailure::Crashing
            }
        );
        assert_eq!(s.poll(now + 60 * MIN), None, "failed stays down");
    }

    #[test]
    fn exits_older_than_ten_minutes_do_not_count() {
        let mut now = 0;
        let mut s = started(now);
        for _ in 0..4 {
            now += 96 * S;
            s.on_exit(now);
            now += 60 * S;
            assert_eq!(s.poll(now), Some(SupervisorAction::Start));
        }
        // Exits at 96, 252, 408, 564 and 720 s: the first is over ten minutes old.
        now += 96 * S;
        s.on_exit(now);
        assert_eq!(s.state(), HostState::Starting);
    }

    #[test]
    fn retry_resets_failed() {
        let mut now = 0;
        let mut s = started(now);
        for _ in 0..5 {
            now += S;
            s.on_exit(now);
            now += 60 * S;
            s.poll(now);
        }
        assert!(matches!(s.state(), HostState::Failed { .. }));
        s.retry(now);
        assert_eq!(s.poll(now), Some(SupervisorAction::Start));
        assert_eq!(s.state(), HostState::Starting);
        // The crash history is gone: one exit does not fail again.
        s.on_exit(now + S);
        assert_eq!(s.state(), HostState::Starting);
        assert_eq!(
            s.poll(now + 2 * S),
            Some(SupervisorAction::Start),
            "backoff reset to 1 s"
        );

        // A new `set_wanted(true)` after `false` resets it too.
        let mut s = started(0);
        s.on_incompatible();
        s.set_wanted(true, 5);
        assert!(
            matches!(s.state(), HostState::Failed { .. }),
            "still wanted"
        );
        s.set_wanted(false, 5);
        assert_eq!(s.state(), HostState::Off);
        s.set_wanted(true, 6);
        assert_eq!(s.poll(6), Some(SupervisorAction::Start));
    }

    #[test]
    fn not_wanted_stops_without_counting_a_crash() {
        let mut now = 0;
        for _ in 0..10 {
            let mut s = started(now);
            s.on_running();
            s.set_wanted(false, now + 1);
            assert_eq!(s.state(), HostState::Off);
            assert_eq!(s.poll(now + 1), Some(SupervisorAction::Stop));
            assert_eq!(s.poll(now + 2), None);
            // The runner may still report the exit it caused: ignored.
            s.on_exit(now + 2);
            assert_eq!(s.state(), HostState::Off);
            now += S;
        }
        let mut s = started(0);
        for i in 1..10 {
            s.set_wanted(false, i * S);
            assert_eq!(s.poll(i * S), Some(SupervisorAction::Stop));
            s.on_exit(i * S);
            s.set_wanted(true, i * S);
            assert_eq!(s.poll(i * S), Some(SupervisorAction::Start), "no backoff");
        }
        assert_eq!(s.state(), HostState::Starting);
    }

    #[test]
    fn foreign_client_pid_is_rejected() {
        assert!(check_client(4242, 4242).is_ok());
        assert!(matches!(
            check_client(4242, 4243),
            Err(HostError::ForeignClient {
                expected: 4242,
                actual: 4243
            })
        ));
        assert!(check_client(4242, 0).is_err());
    }

    #[test]
    fn incompatible_hello_fails_at_once() {
        assert_eq!(
            hello_outcome(&hello(OVERLAY_PROTOCOL_VERSION)),
            HelloOutcome::Compatible
        );
        assert_eq!(
            hello_outcome(&hello(OVERLAY_PROTOCOL_VERSION + 1)),
            HelloOutcome::Incompatible
        );
        assert_eq!(hello_outcome(&values(0)), HelloOutcome::NotHello);
        assert_eq!(exit_kind(Some(3)), ExitKind::Incompatible);
        for code in [Some(0), Some(1), Some(2), Some(4), Some(-1), None] {
            assert_eq!(exit_kind(code), ExitKind::Crash, "{code:?}");
        }

        let mut s = started(0);
        s.on_incompatible();
        assert_eq!(
            s.state(),
            HostState::Failed {
                reason: HostFailure::Incompatible
            }
        );
        assert_eq!(s.poll(60 * MIN), None, "no restart");
    }

    #[test]
    fn overlay_exe_is_next_to_the_app() {
        assert_eq!(
            overlay_exe(Path::new(
                r"C:\Program Files\OpenMonitor Advanced\oma-app.exe"
            )),
            PathBuf::from(r"C:\Program Files\OpenMonitor Advanced\oma-overlay.exe")
        );
        assert_eq!(
            overlay_exe(Path::new(r"C:\repo\target\debug\oma-app.exe")),
            PathBuf::from(r"C:\repo\target\debug\oma-overlay.exe")
        );
    }

    #[test]
    fn full_queue_drops_values_first() {
        let mut q = SendQueue::new();
        assert!(q.push(placement(96)));
        for i in 0..62 {
            assert!(q.push(values(i)));
        }
        assert!(q.push(OverlayMessage::FrameTimes(FrameTimes {
            frames: Vec::new()
        })));
        // Full (64): each control message replaces the oldest data message.
        for dpi in 97..=99 {
            assert!(q.push(placement(dpi)));
        }
        let mut drained = Vec::new();
        while let Some(m) = q.pop() {
            drained.push(m);
        }
        assert_eq!(drained.len(), 64);
        assert_eq!(drained[0], placement(96));
        assert_eq!(drained[1], values(3), "values 0..3 dropped");
        assert_eq!(
            &drained[61..],
            &[placement(97), placement(98), placement(99)]
        );

        // Full of control messages: a new data message is dropped.
        let mut q = SendQueue::new();
        for dpi in 0..64 {
            assert!(q.push(placement(dpi)));
        }
        assert!(!q.push(values(1)));
        assert!(!q.push(OverlayMessage::FrameTimes(FrameTimes {
            frames: Vec::new()
        })));
        // A control message still gets in, at the cost of the oldest one.
        assert!(q.push(placement(64)));
        assert_eq!(q.pop(), Some(placement(1)));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires the built oma-overlay binary"]
    fn spawns_the_built_overlay_and_handshakes() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join(r"..\..\target\debug\oma-overlay.exe");
        assert!(exe.is_file(), "run `cargo build -p oma-overlay` first");
        let (tx, rx) = mpsc::channel();
        let host = OverlayHost::start(exe, move |state| {
            let _ = tx.send(state);
        })
        .unwrap();
        host.set_wanted(true);
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut seen = Vec::new();
        while !seen.contains(&HostState::Running) {
            let left = deadline.saturating_duration_since(Instant::now());
            seen.push(rx.recv_timeout(left).expect("no Running within 15 s"));
        }
        assert_eq!(seen, [HostState::Starting, HostState::Running]);
        assert_eq!(host.state(), HostState::Running);

        // Data flows; without an area the window stays hidden.
        host.send(placement(96));
        host.send(values(1));
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(host.state(), HostState::Running, "the overlay is still up");
        assert!(rx.try_recv().is_err(), "no state change");

        host.stop();
        assert_eq!(rx.recv_timeout(Duration::from_secs(1)), Ok(HostState::Off));
    }
}
