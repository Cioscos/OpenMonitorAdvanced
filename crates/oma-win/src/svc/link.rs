//! The app's link to the sensor service: one thread that starts the service
//! once at launch, connects to its pipe, checks who serves it, subscribes,
//! and hands schema and snapshots to the [`SvcFeed`]; or, in anti-cheat
//! compatible mode, stops the service and keeps away from it (spec §2.2, §6).
//!
//! The rules live in [`Machine::decide`], a pure function of the current
//! state, the event and the time, which returns the effects to run (SCM
//! calls, connection, feed updates). The thread only waits for events and
//! runs effects, so the rules are tested without threads or clocks.
//!
//! Limits, by design:
//! - The anti-cheat preference is per user and only restrains this app: it
//!   stops the service once, verifies the stop, and then leaves the service
//!   alone. If another user, an administrator or another client starts it
//!   again, the app does not fight back with a loop of STOPs (on a PC with
//!   several users logged on, one can already stop the service for the
//!   others: the accepted multi-user limit of spec §2.2).
//! - The reconnect loop never starts the service: only the launch probe, a
//!   `Start` command and leaving anti-cheat mode call `start()`.
//! - [`ServiceLink::shutdown`] interrupts every wait of the thread (Hello,
//!   first sample, stream, retry, STOP verification) at once, but not a call
//!   already running: a pipe write can block for up to 2 s (the client's
//!   write timeout) and an SCM call for as long as the SCM takes. `shutdown`
//!   waits for the thread at most [`JOIN_WAIT`]; past that it detaches it,
//!   and the thread exits by itself as soon as the call returns, without
//!   touching the connection again.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, sync_channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oma_ipc::{Message, Subscribe, WireError, WireSchema, WireSnapshot, PROTOCOL_VERSION};

use super::feed::SvcFeed;
use super::pipe::{CloseReason, ConnectError, PipeClient, PipeEvent, PipeReader};
use super::scm::{RunState, ServiceControl, ServiceQuery};
use super::status::{ServiceDetail, ServiceState, ServiceStatus, ServiceStatusTable};

/// Longest a wait on an open connection goes without looking at commands.
const POLL_SLICE: Duration = Duration::from_millis(50);

/// Longest [`ServiceLink::shutdown`] waits for the thread before detaching it.
pub const JOIN_WAIT: Duration = Duration::from_millis(500);

/// Capacity of the pipe reader's channel (see [`PipeClient::start_reader`]).
const READER_CHANNEL: usize = 8;

/// Win32 codes the rules tell apart.
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;
const ERROR_SERVICE_CANNOT_ACCEPT_CTRL: u32 = 1061;

/// `WireError::code` of a service that does not speak our protocol version.
const UNSUPPORTED_VERSION: &str = "unsupported_version";

/// An open connection to the sensor pipe.
pub trait Connection: Send {
    /// PID of the process serving the pipe, read on this connection.
    fn server_pid(&self) -> Option<u32>;
    fn send(&mut self, msg: &Message) -> std::io::Result<()>;
    /// The next message, `Ok(None)` when `timeout` passes without one, or
    /// `Err` once the connection has closed.
    fn recv_timeout(&mut self, timeout: Duration) -> Result<Option<Message>, CloseReason>;
}

/// Opens a [`Connection`] to the named pipe.
pub type Connector = Arc<dyn Fn(&str) -> Result<Box<dyn Connection>, ConnectError> + Send + Sync>;

/// The real connector: a [`PipeClient`] with its reader behind a
/// `sync_channel(8)`.
pub fn pipe_connector() -> Connector {
    Arc::new(|pipe_name: &str| {
        let client = PipeClient::connect(pipe_name)?;
        let (tx, events) = sync_channel(READER_CHANNEL);
        let reader = client.start_reader(tx);
        Ok(Box::new(PipeConnection {
            reader: Some(reader),
            client,
            events,
        }) as Box<dyn Connection>)
    })
}

/// [`PipeClient`] and its reader as a [`Connection`]. Dropping it stops the
/// reader and then releases the client: the kernel handle closes only when
/// both are gone.
struct PipeConnection {
    reader: Option<PipeReader>,
    client: PipeClient,
    events: Receiver<PipeEvent>,
}

impl Connection for PipeConnection {
    fn server_pid(&self) -> Option<u32> {
        self.client.server_pid()
    }

    fn send(&mut self, msg: &Message) -> std::io::Result<()> {
        self.client.send(msg)
    }

    fn recv_timeout(&mut self, timeout: Duration) -> Result<Option<Message>, CloseReason> {
        match self.events.recv_timeout(timeout) {
            Ok(PipeEvent::Message(msg)) => Ok(Some(msg)),
            Ok(PipeEvent::Closed(reason)) => Err(reason),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            // The reader is gone without a Closed event: its channel overflowed.
            Err(RecvTimeoutError::Disconnected) => Err(CloseReason::Disconnected),
        }
    }
}

impl Drop for PipeConnection {
    fn drop(&mut self) {
        if let Some(reader) = self.reader.take() {
            reader.stop();
        }
    }
}

/// What the shell asks of the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkCommand {
    /// Turns the anti-cheat compatible mode on (stop the service and keep
    /// away) or off (start it once and connect).
    SetAntiCheat(bool),
    /// Starts the service once and connects, unless in anti-cheat mode.
    Start,
}

/// Timing and target of the link.
#[derive(Debug, Clone)]
pub struct LinkSettings {
    pub pipe_name: String,
    /// Subscribed interval; the service accepts 250–5000 ms.
    pub interval_ms: u32,
    /// Pause between reconnection attempts and SCM probes (5 s).
    pub retry: Duration,
    /// How long after a `start()` (or a first `StartPending`) a missing pipe
    /// still reads as `Starting` (10 s).
    pub start_grace: Duration,
    /// Wait for `Hello` on a new connection (2 s).
    pub hello_timeout: Duration,
    /// Wait for the first snapshot after `Subscribe` (30 s: opening
    /// LibreHardwareMonitor alone takes about 4.5 s).
    pub first_sample_timeout: Duration,
    /// Anti-cheat mode: how long the stop may take to be confirmed (30 s).
    pub stop_timeout: Duration,
    /// Anti-cheat mode: pause between the queries that verify the stop.
    pub stop_poll: Duration,
}

impl LinkSettings {
    /// The production timings for `pipe_name` and `interval_ms`.
    pub fn new(pipe_name: impl Into<String>, interval_ms: u32) -> Self {
        Self {
            pipe_name: pipe_name.into(),
            interval_ms,
            retry: Duration::from_secs(5),
            start_grace: Duration::from_secs(10),
            hello_timeout: Duration::from_secs(2),
            first_sample_timeout: Duration::from_secs(30),
            stop_timeout: Duration::from_secs(30),
            stop_poll: Duration::from_millis(250),
        }
    }

    fn interval(&self) -> Duration {
        Duration::from_millis(u64::from(self.interval_ms))
    }
}

/// Checks what the provider relies on before a schema reaches the feed:
/// non-empty device ids without `/`, unique; every sensor on an existing
/// device, with a non-empty name without `/`.
pub fn validate_schema(schema: &WireSchema) -> Result<(), String> {
    let mut devices = HashSet::with_capacity(schema.devices.len());
    for device in &schema.devices {
        if device.id.is_empty() || device.id.contains('/') {
            return Err(format!("invalid device id {:?}", device.id));
        }
        if !devices.insert(device.id.as_str()) {
            return Err(format!("duplicate device id {:?}", device.id));
        }
    }
    for sensor in &schema.sensors {
        if !devices.contains(sensor.device_id.as_str()) {
            return Err(format!(
                "sensor {:?} refers to unknown device {:?}",
                sensor.name, sensor.device_id
            ));
        }
        if sensor.name.is_empty() || sensor.name.contains('/') {
            return Err(format!("invalid sensor name {:?}", sensor.name));
        }
    }
    Ok(())
}

/// What the link thread reacts to.
#[derive(Debug)]
enum Event {
    Command(LinkCommand),
    /// The machine's deadline has passed.
    Timer,
    Queried(ServiceQuery),
    Started(Result<(), u32>),
    StopSent(Result<(), u32>),
    /// A connection opened (with the server PID read on it) or failed.
    Connected(Result<Option<u32>, ConnectError>),
    Sent(bool),
    Message(Message),
    Closed(CloseReason),
}

/// What the machine asks the thread to do, in order.
#[derive(Debug, PartialEq)]
enum Effect {
    Query,
    Start,
    Stop,
    Connect,
    Send(Message),
    /// Drop the connection (stops its reader).
    Close,
    ClearFeed,
    SetInterval(Duration),
    SetSchema(WireSchema),
    SetSnapshot(WireSnapshot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Launch probe: the query is out.
    Probing,
    /// Launch probe: query again at the deadline.
    ProbeWait,
    /// A `start()` is out.
    StartSent,
    /// Connect at the deadline.
    ConnectWait,
    /// The connector is out.
    Connecting,
    /// Connected; the query that checks the server PID is out.
    Verifying { server_pid: Option<u32> },
    /// Waiting for `Hello` until the deadline.
    Hello,
    /// `Subscribe` is being sent.
    Subscribing,
    /// Subscribed; waiting for the first snapshot until the deadline.
    FirstSample { schema_len: Option<usize> },
    /// Snapshots are arriving; the deadline is the silence limit.
    Streaming { schema_len: usize },
    /// Anti-cheat stop: the query is out.
    StopQuery { since: Instant, stop_sent: bool },
    /// Anti-cheat stop: `stop()` is out.
    StopSent { since: Instant },
    /// Anti-cheat stop: query again at the deadline.
    StopWait { since: Instant, stop_sent: bool },
    /// Anti-cheat mode, stop confirmed or failed: nothing to do.
    AntiCheatIdle,
}

impl Phase {
    /// A connection is open in this phase.
    fn has_connection(self) -> bool {
        matches!(
            self,
            Phase::Verifying { .. }
                | Phase::Hello
                | Phase::Subscribing
                | Phase::FirstSample { .. }
                | Phase::Streaming { .. }
        )
    }
}

fn status(state: ServiceState, detail: Option<ServiceDetail>) -> ServiceStatus {
    ServiceStatus::new(state, detail)
}

fn disconnected() -> ServiceStatus {
    status(ServiceState::Unreachable, Some(ServiceDetail::Disconnected))
}

/// The link's rules as a state machine without I/O.
struct Machine {
    settings: LinkSettings,
    anti_cheat: bool,
    status: ServiceStatus,
    phase: Phase,
    /// When [`Event::Timer`] fires.
    deadline: Option<Instant>,
    /// A missing pipe reads as `Starting` until then.
    grace_until: Option<Instant>,
}

impl Machine {
    fn new(settings: LinkSettings, anti_cheat: bool) -> Self {
        let status = if anti_cheat {
            status(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
        } else {
            status(ServiceState::Starting, None)
        };
        Self {
            settings,
            anti_cheat,
            status,
            phase: Phase::Probing,
            deadline: None,
            grace_until: None,
        }
    }

    /// The first effects, at launch: the verified stop in anti-cheat mode,
    /// the launch probe otherwise.
    fn launch(&mut self, now: Instant) -> Vec<Effect> {
        if self.anti_cheat {
            self.begin_stop(now)
        } else {
            self.go(Phase::Probing, None);
            vec![Effect::Query]
        }
    }

    /// Whether the thread should read the connection while it waits.
    fn reads_connection(&self) -> bool {
        matches!(
            self.phase,
            Phase::Hello | Phase::FirstSample { .. } | Phase::Streaming { .. }
        )
    }

    fn go(&mut self, phase: Phase, deadline: Option<Instant>) {
        self.phase = phase;
        self.deadline = deadline;
    }

    fn decide(&mut self, event: Event, now: Instant) -> Vec<Effect> {
        match (event, self.phase) {
            (Event::Command(command), _) => self.on_command(command, now),
            (Event::Timer, _) => self.on_timer(now),
            (Event::Queried(q), Phase::Probing) => self.on_probe(q, now),
            (Event::Queried(q), Phase::Verifying { server_pid }) => {
                self.on_verify(q, server_pid, now)
            }
            (Event::Queried(q), Phase::StopQuery { since, stop_sent }) => {
                self.on_stop_query(q, since, stop_sent, now)
            }
            (Event::Started(result), Phase::StartSent) => self.on_started(result, now),
            (Event::StopSent(result), Phase::StopSent { since }) => {
                self.on_stop_sent(result, since, now)
            }
            (Event::Connected(result), Phase::Connecting) => self.on_connected(result, now),
            (Event::Sent(ok), Phase::Subscribing) => {
                if ok {
                    let until = now + self.settings.first_sample_timeout;
                    self.go(Phase::FirstSample { schema_len: None }, Some(until));
                    Vec::new()
                } else {
                    self.close(disconnected(), now)
                }
            }
            (Event::Message(msg), _) if self.reads_connection() => self.on_message(msg, now),
            (Event::Closed(reason), phase) if phase.has_connection() => {
                tracing::info!("sensor service connection closed: {reason:?}");
                self.close(disconnected(), now)
            }
            // A result or message that no longer applies.
            _ => Vec::new(),
        }
    }

    fn on_command(&mut self, command: LinkCommand, now: Instant) -> Vec<Effect> {
        match command {
            LinkCommand::SetAntiCheat(true) if !self.anti_cheat => {
                self.anti_cheat = true;
                let mut effects = Vec::new();
                if self.phase.has_connection() {
                    effects.push(Effect::Close);
                }
                effects.push(Effect::ClearFeed);
                effects.extend(self.begin_stop(now));
                effects
            }
            LinkCommand::SetAntiCheat(false) if self.anti_cheat => {
                self.anti_cheat = false;
                self.start()
            }
            LinkCommand::Start if !self.anti_cheat && !self.phase.has_connection() => self.start(),
            // Same preference again, or Start while anti-cheat or connected.
            _ => Vec::new(),
        }
    }

    fn start(&mut self) -> Vec<Effect> {
        self.go(Phase::StartSent, None);
        vec![Effect::Start]
    }

    fn connect_now(&mut self) -> Vec<Effect> {
        self.go(Phase::Connecting, None);
        vec![Effect::Connect]
    }

    fn connect_later(&mut self, now: Instant) {
        self.go(Phase::ConnectWait, Some(now + self.settings.retry));
    }

    fn probe_later(&mut self, now: Instant) {
        self.go(Phase::ProbeWait, Some(now + self.settings.retry));
    }

    /// Closes the connection and empties the feed; reconnects after `retry`,
    /// never starting the service.
    fn close(&mut self, status: ServiceStatus, now: Instant) -> Vec<Effect> {
        self.status = status;
        self.connect_later(now);
        vec![Effect::Close, Effect::ClearFeed]
    }

    fn on_timer(&mut self, now: Instant) -> Vec<Effect> {
        if self.deadline.is_none_or(|d| now < d) {
            return Vec::new();
        }
        match self.phase {
            Phase::ProbeWait => {
                self.go(Phase::Probing, None);
                vec![Effect::Query]
            }
            Phase::ConnectWait => self.connect_now(),
            Phase::Hello => {
                tracing::info!("sensor service sent no Hello in time");
                self.close(disconnected(), now)
            }
            Phase::FirstSample { .. } => {
                tracing::info!("sensor service sent no snapshot in time");
                self.close(disconnected(), now)
            }
            Phase::Streaming { .. } => {
                tracing::info!("sensor service went silent for three intervals");
                self.close(disconnected(), now)
            }
            Phase::StopWait { since, stop_sent } => {
                if now >= since + self.settings.stop_timeout {
                    self.stop_failed("the stop was not confirmed in time")
                } else {
                    self.go(Phase::StopQuery { since, stop_sent }, None);
                    vec![Effect::Query]
                }
            }
            _ => {
                self.deadline = None;
                Vec::new()
            }
        }
    }

    /// The launch probe: start a stopped service once, wait for one that is
    /// missing or on its way down, connect to one that runs.
    fn on_probe(&mut self, query: ServiceQuery, now: Instant) -> Vec<Effect> {
        match query {
            ServiceQuery::NotInstalled => {
                self.status = status(ServiceState::NotInstalled, None);
                self.probe_later(now);
                Vec::new()
            }
            ServiceQuery::AccessDenied => {
                self.status = status(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied));
                self.probe_later(now);
                Vec::new()
            }
            ServiceQuery::Error(code) => {
                tracing::warn!("cannot query the sensor service: error {code}");
                self.status = status(ServiceState::Unreachable, None);
                self.probe_later(now);
                Vec::new()
            }
            ServiceQuery::State { state, .. } => match state {
                RunState::Stopped => self.start(),
                RunState::StartPending => {
                    self.status = status(ServiceState::Starting, None);
                    self.grace_until = Some(now + self.settings.start_grace);
                    self.connect_now()
                }
                RunState::Running => {
                    self.status = status(ServiceState::Starting, None);
                    self.connect_now()
                }
                // On its way down (or paused): look again, and start it once stopped.
                RunState::StopPending | RunState::Other(_) => {
                    self.status = status(ServiceState::Starting, None);
                    self.probe_later(now);
                    Vec::new()
                }
            },
        }
    }

    fn on_started(&mut self, result: Result<(), u32>, now: Instant) -> Vec<Effect> {
        match result {
            Ok(()) => {
                self.status = status(ServiceState::Starting, None);
                self.grace_until = Some(now + self.settings.start_grace);
                self.connect_now()
            }
            Err(ERROR_SERVICE_DOES_NOT_EXIST) => {
                self.status = status(ServiceState::NotInstalled, None);
                self.probe_later(now);
                Vec::new()
            }
            Err(code) => {
                tracing::warn!("cannot start the sensor service: error {code}");
                let detail = if code == ERROR_ACCESS_DENIED {
                    ServiceDetail::AccessDenied
                } else {
                    ServiceDetail::StartFailed
                };
                self.status = status(ServiceState::Unreachable, Some(detail));
                self.connect_later(now);
                Vec::new()
            }
        }
    }

    fn on_connected(
        &mut self,
        result: Result<Option<u32>, ConnectError>,
        now: Instant,
    ) -> Vec<Effect> {
        match result {
            Ok(server_pid) => {
                self.go(Phase::Verifying { server_pid }, None);
                vec![Effect::Query]
            }
            Err(error) => {
                if error == ConnectError::AccessDenied {
                    self.status =
                        status(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied));
                } else if self.grace_until.is_some_and(|g| now < g) {
                    self.status = status(ServiceState::Starting, None);
                } else if self.status.state == ServiceState::Starting {
                    self.status = disconnected();
                }
                // Otherwise keep the known reason (Incompatible, AccessDenied, ...).
                self.connect_later(now);
                Vec::new()
            }
        }
    }

    /// Accepts the connection only if the registered service process, read
    /// again now, is the one serving the pipe.
    fn on_verify(
        &mut self,
        query: ServiceQuery,
        server_pid: Option<u32>,
        now: Instant,
    ) -> Vec<Effect> {
        match query {
            ServiceQuery::State {
                state: RunState::Running,
                pid,
            } if pid != 0 && server_pid == Some(pid) => {
                self.go(Phase::Hello, Some(now + self.settings.hello_timeout));
                Vec::new()
            }
            ServiceQuery::AccessDenied => self.close(
                status(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied)),
                now,
            ),
            other => {
                let mismatch = status(ServiceState::Unreachable, Some(ServiceDetail::PidMismatch));
                if self.status != mismatch {
                    tracing::warn!(
                        "sensor pipe served by pid {server_pid:?}, not by the service \
                         ({other:?}); disconnecting"
                    );
                }
                self.close(mismatch, now)
            }
        }
    }

    fn on_message(&mut self, msg: Message, now: Instant) -> Vec<Effect> {
        let schema_len = match self.phase {
            Phase::Hello => return self.on_hello(msg, now),
            Phase::FirstSample { schema_len } => schema_len,
            Phase::Streaming { schema_len } => Some(schema_len),
            _ => return Vec::new(),
        };
        match msg {
            Message::Schema(schema) => {
                if let Err(why) = validate_schema(&schema) {
                    tracing::warn!("sensor service sent an invalid schema: {why}");
                    return self.close(disconnected(), now);
                }
                let len = schema.sensors.len();
                self.phase = match self.phase {
                    Phase::Streaming { .. } => Phase::Streaming { schema_len: len },
                    _ => Phase::FirstSample {
                        schema_len: Some(len),
                    },
                };
                vec![Effect::SetSchema(schema)]
            }
            Message::Snapshot(snapshot) if schema_len == Some(snapshot.values.len()) => {
                self.status = status(ServiceState::Connected, None);
                let len = snapshot.values.len();
                let silence = self.settings.interval() * 3;
                self.go(Phase::Streaming { schema_len: len }, Some(now + silence));
                vec![Effect::SetSnapshot(snapshot)]
            }
            Message::Snapshot(snapshot) => {
                tracing::warn!(
                    "sensor snapshot with {} values for a schema of {schema_len:?} sensors",
                    snapshot.values.len()
                );
                self.close(disconnected(), now)
            }
            Message::Error(error) => self.on_error(error, now),
            other => {
                tracing::warn!("unexpected message from the sensor service: {other:?}");
                self.close(disconnected(), now)
            }
        }
    }

    fn on_hello(&mut self, msg: Message, now: Instant) -> Vec<Effect> {
        match msg {
            Message::Hello(hello) if hello.protocol_version == PROTOCOL_VERSION => {
                self.go(Phase::Subscribing, None);
                vec![
                    Effect::SetInterval(self.settings.interval()),
                    Effect::Send(Message::Subscribe(Subscribe {
                        interval_ms: self.settings.interval_ms,
                    })),
                ]
            }
            Message::Hello(hello) => {
                if self.status.state != ServiceState::Incompatible {
                    tracing::warn!(
                        "sensor service {} speaks protocol {}, not {PROTOCOL_VERSION}",
                        hello.service_version,
                        hello.protocol_version
                    );
                }
                self.close(status(ServiceState::Incompatible, None), now)
            }
            Message::Error(error) => self.on_error(error, now),
            other => {
                tracing::warn!("sensor service did not start with Hello: {other:?}");
                self.close(disconnected(), now)
            }
        }
    }

    fn on_error(&mut self, error: WireError, now: Instant) -> Vec<Effect> {
        tracing::warn!("sensor service error {}: {}", error.code, error.message);
        if error.code == UNSUPPORTED_VERSION {
            self.close(status(ServiceState::Incompatible, None), now)
        } else {
            self.close(disconnected(), now)
        }
    }

    // ---- anti-cheat: the verified stop ----

    fn begin_stop(&mut self, now: Instant) -> Vec<Effect> {
        self.status = status(ServiceState::AntiCheat, Some(ServiceDetail::Stopping));
        self.grace_until = None;
        self.go(
            Phase::StopQuery {
                since: now,
                stop_sent: false,
            },
            None,
        );
        vec![Effect::Query]
    }

    fn on_stop_query(
        &mut self,
        query: ServiceQuery,
        since: Instant,
        stop_sent: bool,
        now: Instant,
    ) -> Vec<Effect> {
        match query {
            ServiceQuery::NotInstalled
            | ServiceQuery::State {
                state: RunState::Stopped,
                ..
            } => self.stopped(),
            ServiceQuery::AccessDenied => self.stop_failed("cannot query the service"),
            ServiceQuery::State {
                state: RunState::Running | RunState::Other(_),
                ..
            } if !stop_sent => {
                self.go(Phase::StopSent { since }, None);
                vec![Effect::Stop]
            }
            // StartPending, StopPending, a STOP already accepted, or a transient error.
            _ => self.stop_wait(since, stop_sent, now),
        }
    }

    fn on_stop_sent(
        &mut self,
        result: Result<(), u32>,
        since: Instant,
        now: Instant,
    ) -> Vec<Effect> {
        match result {
            // Accepted: the service is on its way down, not yet stopped.
            Ok(()) => self.stop_wait(since, true, now),
            // Not in a state that takes controls: wait for one that does.
            Err(ERROR_SERVICE_CANNOT_ACCEPT_CTRL) => self.stop_wait(since, false, now),
            Err(ERROR_SERVICE_DOES_NOT_EXIST) => self.stopped(),
            Err(code) => self.stop_failed(&format!("stop refused with error {code}")),
        }
    }

    fn stop_wait(&mut self, since: Instant, stop_sent: bool, now: Instant) -> Vec<Effect> {
        let limit = since + self.settings.stop_timeout;
        if now >= limit {
            return self.stop_failed("the stop was not confirmed in time");
        }
        let next = (now + self.settings.stop_poll).min(limit);
        self.go(Phase::StopWait { since, stop_sent }, Some(next));
        Vec::new()
    }

    fn stopped(&mut self) -> Vec<Effect> {
        self.status = status(ServiceState::AntiCheat, None);
        self.go(Phase::AntiCheatIdle, None);
        Vec::new()
    }

    /// The preference stays on: no connection and no start until it is
    /// turned off, and no further STOP either.
    fn stop_failed(&mut self, why: &str) -> Vec<Effect> {
        tracing::warn!("anti-cheat mode could not stop the sensor service: {why}");
        self.status = status(ServiceState::AntiCheat, Some(ServiceDetail::StopFailed));
        self.go(Phase::AntiCheatIdle, None);
        Vec::new()
    }
}

/// What reaches the thread on its command channel.
enum Input {
    Command(LinkCommand),
    Shutdown,
}

/// The link thread: waits for events and runs the machine's effects.
struct Driver {
    machine: Machine,
    control: Arc<dyn ServiceControl>,
    connector: Connector,
    status: ServiceStatusTable,
    feed: SvcFeed,
    commands: Receiver<Input>,
    stop: Arc<AtomicBool>,
    conn: Option<Box<dyn Connection>>,
    /// Results of effects, handled before any new wait.
    pending: VecDeque<Event>,
}

impl Driver {
    fn run(mut self) {
        let effects = self.machine.launch(Instant::now());
        self.apply(effects);
        while !self.stopping() {
            let event = match self.pending.pop_front() {
                Some(event) => event,
                None => match self.next_event() {
                    Some(event) => event,
                    None => break,
                },
            };
            let effects = self.machine.decide(event, Instant::now());
            self.apply(effects);
        }
        // Stops the pipe reader and closes the handle.
        self.conn = None;
    }

    fn stopping(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    /// The next command, message, close or deadline; `None` on shutdown.
    fn next_event(&mut self) -> Option<Event> {
        loop {
            if self.stopping() {
                return None;
            }
            // Commands first: a queued command is not delayed by a retry
            // that happens to fall due at the same time.
            match self.commands.try_recv() {
                Ok(input) => return Self::command_event(input),
                Err(TryRecvError::Disconnected) => return None,
                Err(TryRecvError::Empty) => {}
            }
            let now = Instant::now();
            let left = match self.machine.deadline {
                Some(deadline) if now >= deadline => return Some(Event::Timer),
                Some(deadline) => Some(deadline - now),
                None => None,
            };
            match (&mut self.conn, self.machine.reads_connection()) {
                // Reads in short slices so commands and shutdown are seen in time.
                (Some(conn), true) => {
                    let slice = left.map_or(POLL_SLICE, |left| left.min(POLL_SLICE));
                    match conn.recv_timeout(slice) {
                        Ok(Some(msg)) => return Some(Event::Message(msg)),
                        Ok(None) => {}
                        Err(reason) => return Some(Event::Closed(reason)),
                    }
                }
                _ => {
                    let input = match left {
                        Some(left) => match self.commands.recv_timeout(left) {
                            Ok(input) => input,
                            Err(RecvTimeoutError::Timeout) => continue,
                            Err(RecvTimeoutError::Disconnected) => return None,
                        },
                        None => self.commands.recv().ok()?,
                    };
                    return Self::command_event(input);
                }
            }
        }
    }

    fn command_event(input: Input) -> Option<Event> {
        match input {
            Input::Command(command) => Some(Event::Command(command)),
            Input::Shutdown => None,
        }
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Query => self.pending.push_back(Event::Queried(self.control.query())),
                Effect::Start => self.pending.push_back(Event::Started(self.control.start())),
                Effect::Stop => self.pending.push_back(Event::StopSent(self.control.stop())),
                Effect::Connect => {
                    let result = (self.connector)(&self.machine.settings.pipe_name).map(|conn| {
                        let pid = conn.server_pid();
                        self.conn = Some(conn);
                        pid
                    });
                    self.pending.push_back(Event::Connected(result));
                }
                Effect::Send(msg) => {
                    // A write can block for seconds: never start one after shutdown.
                    let ok = !self.stopping()
                        && match self.conn.as_mut() {
                            Some(conn) => match conn.send(&msg) {
                                Ok(()) => true,
                                Err(e) => {
                                    tracing::info!("cannot write to the sensor pipe: {e}");
                                    false
                                }
                            },
                            None => false,
                        };
                    self.pending.push_back(Event::Sent(ok));
                }
                Effect::Close => self.conn = None,
                Effect::ClearFeed => self.feed.clear(),
                Effect::SetInterval(interval) => self.feed.set_interval(interval),
                Effect::SetSchema(schema) => self.feed.set_schema(schema),
                Effect::SetSnapshot(snapshot) => self.feed.set_snapshot(snapshot, Instant::now()),
            }
        }
        let status = self.machine.status;
        if self.status.get().1 != status {
            tracing::info!("sensor service status: {status:?}");
            self.status.set(status);
        }
    }
}

/// The running link: a command channel and the thread.
pub struct ServiceLink {
    commands: Sender<Input>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ServiceLink {
    /// Sets the initial status (`AntiCheat` + `Stopping`, or `Starting`) and
    /// starts the thread.
    pub fn spawn(
        control: Arc<dyn ServiceControl>,
        connector: Connector,
        settings: LinkSettings,
        anti_cheat: bool,
        status: ServiceStatusTable,
        feed: SvcFeed,
    ) -> Self {
        let machine = Machine::new(settings, anti_cheat);
        status.set(machine.status);
        let (commands, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let driver = Driver {
            machine,
            control,
            connector,
            status,
            feed,
            commands: rx,
            stop: Arc::clone(&stop),
            conn: None,
            pending: VecDeque::new(),
        };
        let thread = std::thread::Builder::new()
            .name("oma-service-link".to_owned())
            .spawn(move || driver.run());
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(e) => {
                tracing::error!("cannot start the sensor service link: {e}");
                None
            }
        };
        Self {
            commands,
            stop,
            thread,
        }
    }

    pub fn send(&self, command: LinkCommand) {
        let _ = self.commands.send(Input::Command(command));
    }

    /// Stops the thread, closing any connection. Returns within
    /// [`JOIN_WAIT`] (plus a few milliseconds): a thread still inside a
    /// blocking call by then is detached (see the module notes).
    pub fn shutdown(mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.commands.send(Input::Shutdown);
        let Some(thread) = self.thread.take() else {
            return;
        };
        let end = Instant::now() + JOIN_WAIT;
        while !thread.is_finished() {
            if Instant::now() >= end {
                tracing::warn!("sensor service link still busy at shutdown; detaching it");
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = thread.join();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use oma_ipc::{Hello, WireDevice, WireSensor};

    use super::*;
    use crate::svc::fake_server::FakeServer;

    const PID: u32 = 4242;
    const WAIT: Duration = Duration::from_secs(1);

    fn st(state: ServiceState, detail: Option<ServiceDetail>) -> ServiceStatus {
        ServiceStatus::new(state, detail)
    }

    fn running() -> ServiceQuery {
        ServiceQuery::State {
            state: RunState::Running,
            pid: PID,
        }
    }

    fn in_state(state: RunState) -> ServiceQuery {
        ServiceQuery::State { state, pid: 0 }
    }

    // ---- messages ----

    fn hello(version: u32) -> Message {
        Message::Hello(Hello {
            protocol_version: version,
            service_version: "test".to_owned(),
        })
    }

    fn wire_schema(sensors: usize) -> WireSchema {
        WireSchema {
            devices: vec![WireDevice {
                id: "cpu-0".to_owned(),
                kind: "cpu".to_owned(),
                name: "CPU".to_owned(),
                vendor: None,
                properties: BTreeMap::new(),
                hint: None,
            }],
            sensors: (0..sensors)
                .map(|i| WireSensor {
                    device_id: "cpu-0".to_owned(),
                    kind: "temperature".to_owned(),
                    name: format!("core{i}"),
                    unit: "celsius".to_owned(),
                    label_key: "lhm.raw".to_owned(),
                    label_arg: Some(format!("Core #{i}")),
                    category: "temperature".to_owned(),
                })
                .collect(),
        }
    }

    fn schema(sensors: usize) -> Message {
        Message::Schema(wire_schema(sensors))
    }

    fn wire_snapshot(seq: u64, values: usize) -> WireSnapshot {
        WireSnapshot {
            seq,
            timestamp_ms: seq * 1000,
            values: vec![Some(40.0); values],
        }
    }

    fn snapshot(seq: u64, values: usize) -> Message {
        Message::Snapshot(wire_snapshot(seq, values))
    }

    // ---- fake SCM ----

    struct FakeScm {
        query: ServiceQuery,
        start_result: Result<(), u32>,
        /// What `query` answers after a successful `start`.
        after_start: Option<ServiceQuery>,
        stop_result: Result<(), u32>,
        /// What `query` answers after a successful `stop`.
        after_stop: Option<ServiceQuery>,
        starts: usize,
        stops: usize,
        queries: usize,
    }

    struct FakeControl(Mutex<FakeScm>);

    impl FakeControl {
        fn new(query: ServiceQuery) -> Arc<Self> {
            Arc::new(Self(Mutex::new(FakeScm {
                query,
                start_result: Ok(()),
                after_start: Some(running()),
                stop_result: Ok(()),
                after_stop: Some(in_state(RunState::Stopped)),
                starts: 0,
                stops: 0,
                queries: 0,
            })))
        }

        fn with(&self, f: impl FnOnce(&mut FakeScm)) {
            f(&mut self.0.lock().unwrap());
        }

        fn set_query(&self, query: ServiceQuery) {
            self.with(|s| s.query = query);
        }

        fn starts(&self) -> usize {
            self.0.lock().unwrap().starts
        }

        fn stops(&self) -> usize {
            self.0.lock().unwrap().stops
        }

        fn queries(&self) -> usize {
            self.0.lock().unwrap().queries
        }
    }

    impl ServiceControl for FakeControl {
        fn query(&self) -> ServiceQuery {
            let mut s = self.0.lock().unwrap();
            s.queries += 1;
            s.query
        }

        fn start(&self) -> Result<(), u32> {
            let mut s = self.0.lock().unwrap();
            s.starts += 1;
            if s.start_result.is_ok() {
                if let Some(q) = s.after_start {
                    s.query = q;
                }
            }
            s.start_result
        }

        fn stop(&self) -> Result<(), u32> {
            let mut s = self.0.lock().unwrap();
            s.stops += 1;
            if s.stop_result.is_ok() {
                if let Some(q) = s.after_stop {
                    s.query = q;
                }
            }
            s.stop_result
        }
    }

    // ---- scripted connections ----

    enum Step {
        Msg(Message),
        Close(CloseReason),
    }

    struct FakeConn {
        pid: Option<u32>,
        steps: Receiver<Step>,
        /// Keeps `steps` open whatever the test does with its sender.
        _keep: Sender<Step>,
        sent: Arc<Mutex<Vec<Message>>>,
        send_block: Duration,
        alive: Arc<AtomicBool>,
    }

    impl Drop for FakeConn {
        fn drop(&mut self) {
            self.alive.store(false, Ordering::SeqCst);
        }
    }

    impl Connection for FakeConn {
        fn server_pid(&self) -> Option<u32> {
            self.pid
        }

        fn send(&mut self, msg: &Message) -> std::io::Result<()> {
            if !self.send_block.is_zero() {
                std::thread::sleep(self.send_block);
            }
            self.sent.lock().unwrap().push(msg.clone());
            Ok(())
        }

        fn recv_timeout(&mut self, timeout: Duration) -> Result<Option<Message>, CloseReason> {
            match self.steps.recv_timeout(timeout) {
                Ok(Step::Msg(m)) => Ok(Some(m)),
                Ok(Step::Close(r)) => Err(r),
                Err(_) => Ok(None),
            }
        }
    }

    /// The test's end of a scripted connection.
    #[derive(Clone)]
    struct ConnCtl {
        tx: Sender<Step>,
        sent: Arc<Mutex<Vec<Message>>>,
        alive: Arc<AtomicBool>,
    }

    impl ConnCtl {
        fn push(&self, msg: Message) {
            self.tx.send(Step::Msg(msg)).unwrap();
        }

        fn close(&self) {
            self.tx
                .send(Step::Close(CloseReason::Disconnected))
                .unwrap();
        }

        fn sent(&self) -> Vec<Message> {
            self.sent.lock().unwrap().clone()
        }

        fn is_alive(&self) -> bool {
            self.alive.load(Ordering::SeqCst)
        }
    }

    fn fake_conn(pid: Option<u32>) -> (FakeConn, ConnCtl) {
        let (tx, rx) = mpsc::channel();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let conn = FakeConn {
            pid,
            steps: rx,
            _keep: tx.clone(),
            sent: Arc::clone(&sent),
            send_block: Duration::ZERO,
            alive: Arc::clone(&alive),
        };
        (conn, ConnCtl { tx, sent, alive })
    }

    /// A connection that greets, describes two sensors and sends one snapshot.
    fn streaming_conn(pid: Option<u32>) -> (FakeConn, ConnCtl) {
        let (conn, ctl) = fake_conn(pid);
        ctl.push(hello(PROTOCOL_VERSION));
        ctl.push(schema(2));
        ctl.push(snapshot(1, 2));
        (conn, ctl)
    }

    /// Connections handed out in order; `NotFound` once they run out.
    #[derive(Default)]
    struct Script {
        conns: Mutex<VecDeque<Result<FakeConn, ConnectError>>>,
        connects: Mutex<usize>,
    }

    impl Script {
        fn with(conns: Vec<FakeConn>) -> Arc<Self> {
            let script = Self::default();
            script
                .conns
                .lock()
                .unwrap()
                .extend(conns.into_iter().map(Ok));
            Arc::new(script)
        }

        fn add(&self, conn: FakeConn) {
            self.conns.lock().unwrap().push_back(Ok(conn));
        }

        fn connects(&self) -> usize {
            *self.connects.lock().unwrap()
        }

        fn connector(self: &Arc<Self>) -> Connector {
            let script = Arc::clone(self);
            Arc::new(move |_name: &str| {
                *script.connects.lock().unwrap() += 1;
                let next = script.conns.lock().unwrap().pop_front();
                next.unwrap_or(Err(ConnectError::NotFound))
                    .map(|c| Box::new(c) as Box<dyn Connection>)
            })
        }
    }

    // ---- harness ----

    fn test_settings() -> LinkSettings {
        LinkSettings {
            retry: Duration::from_millis(20),
            start_grace: Duration::from_millis(60),
            stop_poll: Duration::from_millis(5),
            stop_timeout: Duration::from_secs(2),
            ..LinkSettings::new("test-pipe", 1000)
        }
    }

    struct Harness {
        control: Arc<FakeControl>,
        script: Arc<Script>,
        status: ServiceStatusTable,
        feed: SvcFeed,
        link: Option<ServiceLink>,
    }

    impl Harness {
        fn spawn(control: Arc<FakeControl>, script: Arc<Script>, anti_cheat: bool) -> Self {
            Self::spawn_with(control, script, anti_cheat, test_settings())
        }

        fn spawn_with(
            control: Arc<FakeControl>,
            script: Arc<Script>,
            anti_cheat: bool,
            settings: LinkSettings,
        ) -> Self {
            let status = ServiceStatusTable::default();
            let feed = SvcFeed::default();
            let link = ServiceLink::spawn(
                Arc::clone(&control) as Arc<dyn ServiceControl>,
                script.connector(),
                settings,
                anti_cheat,
                status.clone(),
                feed.clone(),
            );
            Self {
                control,
                script,
                status,
                feed,
                link: Some(link),
            }
        }

        fn send(&self, command: LinkCommand) {
            self.link.as_ref().unwrap().send(command);
        }

        fn wait_for(&self, what: impl Fn(ServiceStatus) -> bool) -> ServiceStatus {
            wait_for(&self.status, what, WAIT)
        }

        fn status(&self) -> ServiceStatus {
            self.status.get().1
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            if let Some(link) = self.link.take() {
                link.shutdown();
            }
        }
    }

    /// Polls `table` until `what` holds, or panics with the last status.
    fn wait_for(
        table: &ServiceStatusTable,
        what: impl Fn(ServiceStatus) -> bool,
        timeout: Duration,
    ) -> ServiceStatus {
        let end = Instant::now() + timeout;
        loop {
            let status = table.get().1;
            if what(status) {
                return status;
            }
            if Instant::now() >= end {
                panic!("status never matched; last {status:?}");
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn is(expected: ServiceStatus) -> impl Fn(ServiceStatus) -> bool {
        move |s| s == expected
    }

    fn connected() -> ServiceStatus {
        st(ServiceState::Connected, None)
    }

    fn disconnected() -> ServiceStatus {
        st(ServiceState::Unreachable, Some(ServiceDetail::Disconnected))
    }

    /// Lets a few retry periods (20 ms each) go by.
    fn cycles(n: u32) {
        std::thread::sleep(Duration::from_millis(20) * n + Duration::from_millis(30));
    }

    // ---- launch ----

    #[test]
    fn launch_starts_a_stopped_service_once() {
        let control = FakeControl::new(in_state(RunState::Stopped));
        let h = Harness::spawn(control, Script::with(vec![]), false);
        // No pipe: Starting during the grace period, then Unreachable.
        h.wait_for(is(disconnected()));
        cycles(5);
        assert_eq!(h.control.starts(), 1);
        assert!(h.script.connects() >= 3, "the link keeps reconnecting");
        assert_eq!(h.status(), disconnected());
    }

    #[test]
    fn not_installed_is_reported_and_never_started() {
        let control = FakeControl::new(ServiceQuery::NotInstalled);
        let h = Harness::spawn(control, Script::with(vec![]), false);
        h.wait_for(is(st(ServiceState::NotInstalled, None)));
        cycles(5);
        assert_eq!(h.control.starts(), 0);
        assert!(h.control.queries() >= 3, "the probe repeats at every retry");
        assert_eq!(h.script.connects(), 0);
        assert_eq!(h.status(), st(ServiceState::NotInstalled, None));
    }

    #[test]
    fn access_denied_on_start_is_unreachable_with_detail() {
        let control = FakeControl::new(in_state(RunState::Stopped));
        control.with(|s| s.start_result = Err(ERROR_ACCESS_DENIED));
        let h = Harness::spawn(control, Script::with(vec![]), false);
        let denied = st(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied));
        h.wait_for(is(denied));
        cycles(5);
        assert_eq!(h.control.starts(), 1);
        assert_eq!(h.status(), denied, "a missing pipe keeps the reason");
    }

    #[test]
    fn other_start_errors_are_start_failed() {
        let control = FakeControl::new(in_state(RunState::Stopped));
        control.with(|s| s.start_result = Err(1058));
        let h = Harness::spawn(control, Script::with(vec![]), false);
        h.wait_for(is(st(
            ServiceState::Unreachable,
            Some(ServiceDetail::StartFailed),
        )));
        assert_eq!(h.control.starts(), 1);
    }

    #[test]
    fn reconnect_loop_never_starts_the_service() {
        let control = FakeControl::new(in_state(RunState::Stopped));
        let (conn, ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));

        ctl.close();
        h.wait_for(is(disconnected()));
        let connects = h.script.connects();
        cycles(5);
        assert!(
            h.script.connects() >= connects + 3,
            "the link keeps retrying"
        );
        assert_eq!(h.control.starts(), 1, "only the launch started the service");
    }

    // ---- connection checks ----

    #[test]
    fn pid_mismatch_is_unreachable_not_connected() {
        let control = FakeControl::new(running());
        let (conn, ctl) = streaming_conn(Some(PID + 1));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(st(
            ServiceState::Unreachable,
            Some(ServiceDetail::PidMismatch),
        )));
        let view = h.feed.view();
        assert!(view.schema.is_none());
        assert!(view.snapshot.is_none());
        assert!(ctl.sent().is_empty(), "nothing is sent to an impostor");
        cycles(1);
        assert!(!ctl.is_alive(), "the connection is closed");
    }

    #[test]
    fn missing_server_pid_is_rejected() {
        let control = FakeControl::new(running());
        let (conn, ctl) = streaming_conn(None);
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(st(
            ServiceState::Unreachable,
            Some(ServiceDetail::PidMismatch),
        )));
        assert!(h.feed.view().schema.is_none());
        assert!(ctl.sent().is_empty());
    }

    #[test]
    fn a_stopped_service_behind_the_pipe_is_a_pid_mismatch() {
        // Someone else serves the pipe while the service is stopped (pid 0).
        let control = FakeControl::new(in_state(RunState::StartPending));
        let (conn, _ctl) = streaming_conn(Some(0));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(st(
            ServiceState::Unreachable,
            Some(ServiceDetail::PidMismatch),
        )));
        assert!(h.feed.view().schema.is_none());
    }

    #[test]
    fn protocol_version_mismatch_is_incompatible() {
        let control = FakeControl::new(running());
        let (conn, ctl) = fake_conn(Some(PID));
        ctl.push(hello(PROTOCOL_VERSION + 1));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        let incompatible = st(ServiceState::Incompatible, None);
        h.wait_for(is(incompatible));
        assert!(ctl.sent().is_empty(), "no Subscribe after a foreign Hello");
        let connects = h.script.connects();
        cycles(5);
        assert!(h.script.connects() > connects, "the connection is retried");
        assert_eq!(h.status(), incompatible, "retries do not change the state");
    }

    #[test]
    fn unsupported_version_error_is_incompatible() {
        let control = FakeControl::new(running());
        let (conn, ctl) = fake_conn(Some(PID));
        ctl.push(hello(PROTOCOL_VERSION));
        ctl.push(Message::Error(WireError {
            code: UNSUPPORTED_VERSION.to_owned(),
            message: "no".to_owned(),
        }));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(st(ServiceState::Incompatible, None)));
    }

    // ---- the stream ----

    #[test]
    fn schema_and_snapshots_reach_the_feed_and_status_is_connected() {
        let control = FakeControl::new(running());
        let (conn, ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));

        let view = h.feed.view();
        assert_eq!(view.schema.as_deref(), Some(&wire_schema(2)));
        assert_eq!(view.snapshot.map(|(_, s)| s), Some(wire_snapshot(1, 2)));
        assert_eq!(view.interval, Duration::from_millis(1000));
        assert_eq!(
            ctl.sent(),
            vec![Message::Subscribe(Subscribe { interval_ms: 1000 })]
        );
        assert_eq!(h.control.starts(), 0, "a running service is not started");

        ctl.push(snapshot(2, 2));
        let end = Instant::now() + WAIT;
        while h.feed.view().snapshot.map(|(_, s)| s.seq) != Some(2) {
            assert!(Instant::now() < end, "the second snapshot never arrived");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_new_schema_on_the_stream_replaces_the_old_one() {
        let control = FakeControl::new(running());
        let (conn, ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));
        let generation = h.feed.view().generation;

        ctl.push(schema(3));
        ctl.push(snapshot(2, 3));
        let end = Instant::now() + WAIT;
        loop {
            let view = h.feed.view();
            if view.snapshot.as_ref().map(|(_, s)| s.seq) == Some(2) {
                assert!(view.generation > generation);
                assert_eq!(view.schema.map(|s| s.sensors.len()), Some(3));
                break;
            }
            assert!(Instant::now() < end, "the new schema never arrived");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(h.status(), connected());
    }

    #[test]
    fn snapshot_without_schema_or_wrong_length_disconnects() {
        for script in [
            vec![hello(PROTOCOL_VERSION), snapshot(1, 2)],
            vec![hello(PROTOCOL_VERSION), schema(2), snapshot(1, 3)],
        ] {
            let control = FakeControl::new(running());
            let (conn, ctl) = fake_conn(Some(PID));
            for msg in script {
                ctl.push(msg);
            }
            let h = Harness::spawn(control, Script::with(vec![conn]), false);
            h.wait_for(is(disconnected()));
            let view = h.feed.view();
            assert!(view.schema.is_none());
            assert!(view.snapshot.is_none());
            cycles(1);
            assert!(!ctl.is_alive());
        }
    }

    #[test]
    fn invalid_schema_disconnects() {
        let control = FakeControl::new(running());
        let (conn, ctl) = fake_conn(Some(PID));
        let mut bad = wire_schema(2);
        bad.sensors[1].device_id = "nowhere".to_owned();
        ctl.push(hello(PROTOCOL_VERSION));
        ctl.push(Message::Schema(bad));
        ctl.push(snapshot(1, 2));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(disconnected()));
        assert!(h.feed.view().schema.is_none());
        let connects = h.script.connects();
        cycles(2);
        assert!(h.script.connects() > connects, "the link reconnects");
    }

    #[test]
    fn silent_stream_times_out_and_clears_feed() {
        let control = FakeControl::new(running());
        let (conn, _ctl) = streaming_conn(Some(PID));
        let settings = LinkSettings {
            interval_ms: 40,
            ..test_settings()
        };
        let h = Harness::spawn_with(control, Script::with(vec![conn]), false, settings);
        h.wait_for(is(connected()));
        let generation = h.feed.view().generation;

        // Nothing more arrives: after 3 x 40 ms the link gives up.
        h.wait_for(is(disconnected()));
        let view = h.feed.view();
        assert!(view.generation > generation);
        assert!(view.schema.is_none());
        assert!(view.snapshot.is_none());
    }

    #[test]
    fn disconnect_clears_the_feed() {
        let control = FakeControl::new(running());
        let (conn, ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));
        let generation = h.feed.view().generation;

        ctl.close();
        h.wait_for(is(disconnected()));
        let view = h.feed.view();
        assert!(view.generation > generation);
        assert!(view.schema.is_none());
        assert!(view.snapshot.is_none());
    }

    #[test]
    fn the_link_reconnects_after_a_close() {
        let control = FakeControl::new(running());
        let (first, ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![first]), false);
        h.wait_for(is(connected()));
        let (second, _ctl2) = streaming_conn(Some(PID));
        h.script.add(second);
        ctl.close();
        h.wait_for(is(disconnected()));
        h.wait_for(is(connected()));
        assert_eq!(h.control.starts(), 0);
    }

    // ---- anti-cheat mode ----

    #[test]
    fn anti_cheat_stops_and_keeps_the_service_stopped() {
        let control = FakeControl::new(running());
        let (conn, ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));

        h.send(LinkCommand::SetAntiCheat(true));
        h.wait_for(is(st(ServiceState::AntiCheat, None)));
        assert!(h.feed.view().schema.is_none());
        let connects = h.script.connects();
        cycles(10);
        assert!(!ctl.is_alive(), "the connection is closed");
        assert_eq!(h.control.stops(), 1);
        assert_eq!(h.control.starts(), 0);
        assert_eq!(h.script.connects(), connects, "no connection attempts");
        assert_eq!(h.status(), st(ServiceState::AntiCheat, None));
    }

    #[test]
    fn anti_cheat_does_not_fight_other_clients() {
        let control = FakeControl::new(running());
        let h = Harness::spawn(Arc::clone(&control), Script::with(vec![]), true);
        h.wait_for(is(st(ServiceState::AntiCheat, None)));
        // Someone else starts the service again.
        control.set_query(running());
        cycles(10);
        assert_eq!(control.stops(), 1);
        assert_eq!(h.status(), st(ServiceState::AntiCheat, None));
    }

    #[test]
    fn anti_cheat_at_launch_stops_a_running_service() {
        let control = FakeControl::new(running());
        // Hold the stop in StopPending so the launch status can be observed.
        control.with(|s| s.after_stop = Some(in_state(RunState::StopPending)));
        let h = Harness::spawn(Arc::clone(&control), Script::with(vec![]), true);
        let stopping = st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping));
        assert_eq!(h.status(), stopping, "set before the thread runs");
        cycles(2);
        assert_eq!(h.status(), stopping);
        control.set_query(in_state(RunState::Stopped));
        h.wait_for(is(st(ServiceState::AntiCheat, None)));
        assert_eq!(control.stops(), 1);
        assert_eq!(control.starts(), 0);
        assert_eq!(h.script.connects(), 0);
    }

    #[test]
    fn disabling_anti_cheat_starts_once() {
        let control = FakeControl::new(in_state(RunState::Stopped));
        let (conn, _ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![conn]), true);
        h.wait_for(is(st(ServiceState::AntiCheat, None)));
        assert_eq!(h.control.stops(), 0, "a stopped service needs no STOP");
        assert_eq!(h.control.starts(), 0);

        h.send(LinkCommand::SetAntiCheat(false));
        h.wait_for(is(connected()));
        h.send(LinkCommand::SetAntiCheat(false));
        cycles(3);
        assert_eq!(h.control.starts(), 1);
        assert_eq!(h.status(), connected());
    }

    #[test]
    fn start_command_is_ignored_in_anti_cheat_mode() {
        let control = FakeControl::new(in_state(RunState::Stopped));
        let h = Harness::spawn(control, Script::with(vec![]), true);
        h.wait_for(is(st(ServiceState::AntiCheat, None)));
        h.send(LinkCommand::Start);
        cycles(3);
        assert_eq!(h.control.starts(), 0);
        assert_eq!(h.script.connects(), 0);
        assert_eq!(h.status(), st(ServiceState::AntiCheat, None));
    }

    #[test]
    fn start_command_starts_once_and_connects() {
        let control = FakeControl::new(in_state(RunState::Stopped));
        control.with(|s| s.start_result = Err(1058));
        let h = Harness::spawn(Arc::clone(&control), Script::with(vec![]), false);
        h.wait_for(is(st(
            ServiceState::Unreachable,
            Some(ServiceDetail::StartFailed),
        )));
        control.with(|s| s.start_result = Ok(()));
        // Scripted after the command, so a retry cannot pick it up while the
        // fake SCM still reports the service as stopped.
        h.send(LinkCommand::Start);
        let (conn, _ctl) = streaming_conn(Some(PID));
        h.script.add(conn);
        h.wait_for(is(connected()));
        h.send(LinkCommand::Start);
        cycles(2);
        assert_eq!(control.starts(), 2, "the launch start and the command");
    }

    #[test]
    fn stop_is_confirmed_by_query() {
        let control = FakeControl::new(running());
        control.with(|s| s.after_stop = Some(running()));
        let h = Harness::spawn(Arc::clone(&control), Script::with(vec![]), true);
        cycles(3);
        // STOP was accepted but the service still runs: not confirmed yet.
        assert_eq!(control.stops(), 1);
        assert_eq!(
            h.status(),
            st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
        );
        control.set_query(in_state(RunState::StopPending));
        cycles(2);
        assert_eq!(
            h.status(),
            st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
        );
        control.set_query(in_state(RunState::Stopped));
        h.wait_for(is(st(ServiceState::AntiCheat, None)));
        assert_eq!(control.stops(), 1);
    }

    #[test]
    fn start_pending_is_waited_before_stop() {
        let control = FakeControl::new(in_state(RunState::StartPending));
        let h = Harness::spawn(Arc::clone(&control), Script::with(vec![]), true);
        cycles(3);
        assert_eq!(control.stops(), 0, "no STOP while StartPending");
        assert_eq!(
            h.status(),
            st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
        );
        control.set_query(running());
        h.wait_for(is(st(ServiceState::AntiCheat, None)));
        assert_eq!(control.stops(), 1);
    }

    #[test]
    fn stop_timeout_keeps_preference_and_reports_failure() {
        let control = FakeControl::new(running());
        control.with(|s| s.after_stop = None); // accepted, never stops
        let settings = LinkSettings {
            stop_timeout: Duration::from_millis(100),
            ..test_settings()
        };
        let h = Harness::spawn_with(Arc::clone(&control), Script::with(vec![]), true, settings);
        let failed = st(ServiceState::AntiCheat, Some(ServiceDetail::StopFailed));
        h.wait_for(is(failed));
        assert_eq!(control.stops(), 1, "one STOP, not a loop");

        h.send(LinkCommand::Start);
        cycles(5);
        assert_eq!(control.starts(), 0);
        assert_eq!(h.script.connects(), 0);
        assert_eq!(control.stops(), 1);
        assert_eq!(h.status(), failed);
    }

    #[test]
    fn a_refused_stop_is_stop_failed() {
        let control = FakeControl::new(running());
        control.with(|s| s.stop_result = Err(ERROR_ACCESS_DENIED));
        let h = Harness::spawn(Arc::clone(&control), Script::with(vec![]), true);
        h.wait_for(is(st(
            ServiceState::AntiCheat,
            Some(ServiceDetail::StopFailed),
        )));
        cycles(3);
        assert_eq!(control.stops(), 1);
    }

    #[test]
    fn repeated_toggle_is_idempotent() {
        let control = FakeControl::new(running());
        let (conn, _ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(Arc::clone(&control), Script::with(vec![conn]), false);
        h.wait_for(is(connected()));

        h.send(LinkCommand::SetAntiCheat(true));
        h.send(LinkCommand::SetAntiCheat(true));
        h.wait_for(is(st(ServiceState::AntiCheat, None)));
        cycles(2);
        assert_eq!(control.stops(), 1);

        let (conn, _ctl) = streaming_conn(Some(PID));
        h.script.add(conn);
        h.send(LinkCommand::SetAntiCheat(false));
        h.send(LinkCommand::SetAntiCheat(false));
        h.wait_for(is(connected()));
        cycles(2);
        assert_eq!(control.starts(), 1);
        assert_eq!(control.stops(), 1);
    }

    // ---- shutdown ----

    fn assert_quick_shutdown(h: &mut Harness, what: &str) {
        let t = Instant::now();
        h.link.take().unwrap().shutdown();
        let took = t.elapsed();
        assert!(
            took < Duration::from_secs(1),
            "{what}: shutdown took {took:?}"
        );
    }

    #[test]
    fn shutdown_joins_quickly() {
        // Waiting for Hello (2 s timeout).
        let control = FakeControl::new(running());
        let (conn, ctl) = fake_conn(Some(PID));
        let mut h = Harness::spawn(control, Script::with(vec![conn]), false);
        let end = Instant::now() + WAIT;
        while h.script.connects() == 0 {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(30));
        assert_quick_shutdown(&mut h, "during Hello");
        assert!(!ctl.is_alive(), "the connection is released");

        // A write the server never takes (the pipe client gives up after 2 s).
        let control = FakeControl::new(running());
        let (mut conn, ctl) = fake_conn(Some(PID));
        conn.send_block = Duration::from_secs(3);
        ctl.push(hello(PROTOCOL_VERSION));
        let mut h = Harness::spawn(control, Script::with(vec![conn]), false);
        std::thread::sleep(Duration::from_millis(100));
        assert_quick_shutdown(&mut h, "during a blocked write");

        // Verifying a STOP that never completes (30 s timeout).
        let control = FakeControl::new(running());
        control.with(|s| s.after_stop = Some(in_state(RunState::StopPending)));
        let settings = LinkSettings {
            stop_timeout: Duration::from_secs(30),
            stop_poll: Duration::from_millis(250),
            ..test_settings()
        };
        let mut h = Harness::spawn_with(control, Script::with(vec![]), true, settings);
        std::thread::sleep(Duration::from_millis(50));
        assert_quick_shutdown(&mut h, "during the STOP wait");

        // Idle in anti-cheat mode, and in the retry wait.
        let control = FakeControl::new(ServiceQuery::NotInstalled);
        let settings = LinkSettings {
            retry: Duration::from_secs(5),
            ..test_settings()
        };
        let mut h = Harness::spawn_with(control, Script::with(vec![]), false, settings);
        std::thread::sleep(Duration::from_millis(50));
        assert_quick_shutdown(&mut h, "during the retry wait");
    }

    // ---- schema validation ----

    #[test]
    fn validate_schema_accepts_a_good_schema() {
        assert_eq!(validate_schema(&wire_schema(3)), Ok(()));
        assert_eq!(validate_schema(&wire_schema(0)), Ok(()));
    }

    #[test]
    fn validate_schema_rejects_what_the_provider_cannot_bind() {
        type Spoil = fn(&mut WireSchema);
        let cases: [(&str, Spoil); 6] = [
            ("duplicate device", |s| s.devices.push(s.devices[0].clone())),
            ("empty device id", |s| s.devices[0].id.clear()),
            ("slash in device id", |s| {
                s.devices[0].id = "cpu/0".to_owned()
            }),
            ("unknown device", |s| {
                s.sensors[0].device_id = "gpu-0".to_owned()
            }),
            ("empty sensor name", |s| s.sensors[0].name.clear()),
            ("slash in sensor name", |s| {
                s.sensors[0].name = "a/b".to_owned()
            }),
        ];
        for (what, spoil) in cases {
            let mut schema = wire_schema(2);
            spoil(&mut schema);
            assert!(validate_schema(&schema).is_err(), "{what} was accepted");
        }
    }

    // ---- the decision function ----

    fn machine(anti_cheat: bool) -> (Machine, Instant) {
        let now = Instant::now();
        let mut m = Machine::new(test_settings(), anti_cheat);
        let effects = m.launch(now);
        assert_eq!(effects, vec![Effect::Query]);
        (m, now)
    }

    /// A machine that has just verified the connection and got `Hello`.
    fn subscribed(now: Instant) -> Machine {
        let (mut m, _) = machine(false);
        assert_eq!(
            m.decide(Event::Queried(running()), now),
            vec![Effect::Connect]
        );
        assert_eq!(
            m.decide(Event::Connected(Ok(Some(PID))), now),
            vec![Effect::Query]
        );
        assert_eq!(m.decide(Event::Queried(running()), now), vec![]);
        assert!(m.reads_connection());
        assert_eq!(
            m.decide(Event::Message(hello(PROTOCOL_VERSION)), now),
            vec![
                Effect::SetInterval(Duration::from_millis(1000)),
                Effect::Send(Message::Subscribe(Subscribe { interval_ms: 1000 })),
            ]
        );
        assert!(!m.reads_connection());
        assert_eq!(m.decide(Event::Sent(true), now), vec![]);
        assert!(m.reads_connection());
        m
    }

    #[test]
    fn decide_launch_status_follows_the_preference() {
        let (m, _) = machine(false);
        assert_eq!(m.status, st(ServiceState::Starting, None));
        let (m, _) = machine(true);
        assert_eq!(
            m.status,
            st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
        );
    }

    #[test]
    fn decide_a_missing_pipe_is_starting_only_within_the_grace() {
        let (mut m, t0) = machine(false);
        assert_eq!(
            m.decide(Event::Queried(in_state(RunState::Stopped)), t0),
            vec![Effect::Start]
        );
        assert_eq!(m.decide(Event::Started(Ok(())), t0), vec![Effect::Connect]);
        let not_found = || Event::Connected(Err(ConnectError::NotFound));
        assert_eq!(m.decide(not_found(), t0), vec![]);
        assert_eq!(m.status, st(ServiceState::Starting, None));
        assert_eq!(m.deadline, Some(t0 + Duration::from_millis(20)));

        let t1 = t0 + Duration::from_millis(20);
        assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Connect]);
        assert_eq!(m.decide(not_found(), t1), vec![]);
        assert_eq!(m.status, st(ServiceState::Starting, None));

        let t2 = t0 + Duration::from_millis(60);
        assert_eq!(m.decide(Event::Timer, t2), vec![Effect::Connect]);
        assert_eq!(m.decide(not_found(), t2), vec![]);
        assert_eq!(m.status, disconnected());
    }

    #[test]
    fn decide_start_pending_opens_the_grace() {
        let (mut m, t0) = machine(false);
        assert_eq!(
            m.decide(Event::Queried(in_state(RunState::StartPending)), t0),
            vec![Effect::Connect]
        );
        m.decide(Event::Connected(Err(ConnectError::NotFound)), t0);
        assert_eq!(m.status, st(ServiceState::Starting, None));
    }

    #[test]
    fn decide_a_running_service_without_pipe_is_unreachable_at_once() {
        let (mut m, t0) = machine(false);
        assert_eq!(
            m.decide(Event::Queried(running()), t0),
            vec![Effect::Connect]
        );
        m.decide(Event::Connected(Err(ConnectError::NotFound)), t0);
        assert_eq!(m.status, disconnected());
    }

    #[test]
    fn decide_pid_check_uses_the_fresh_query() {
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Connected(Ok(Some(PID))), t0);
        // The service restarted between the launch query and this one.
        let restarted = ServiceQuery::State {
            state: RunState::Running,
            pid: PID + 7,
        };
        assert_eq!(
            m.decide(Event::Queried(restarted), t0),
            vec![Effect::Close, Effect::ClearFeed]
        );
        assert_eq!(
            m.status,
            st(ServiceState::Unreachable, Some(ServiceDetail::PidMismatch))
        );
        assert_eq!(m.phase, Phase::ConnectWait);
    }

    #[test]
    fn decide_snapshot_length_must_match_the_latest_schema() {
        let t0 = Instant::now();
        let mut m = subscribed(t0);
        assert_eq!(
            m.decide(Event::Message(schema(2)), t0),
            vec![Effect::SetSchema(wire_schema(2))]
        );
        assert_eq!(
            m.decide(Event::Message(snapshot(1, 2)), t0),
            vec![Effect::SetSnapshot(wire_snapshot(1, 2))]
        );
        assert_eq!(m.status, connected());
        assert_eq!(m.deadline, Some(t0 + Duration::from_millis(3000)));

        m.decide(Event::Message(schema(3)), t0);
        // A snapshot sized for the old schema is a protocol violation.
        assert_eq!(
            m.decide(Event::Message(snapshot(2, 2)), t0),
            vec![Effect::Close, Effect::ClearFeed]
        );
        assert_eq!(m.status, disconnected());
    }

    #[test]
    fn decide_first_sample_and_silence_deadlines() {
        let t0 = Instant::now();
        let mut m = subscribed(t0);
        assert_eq!(m.deadline, Some(t0 + Duration::from_secs(30)));
        assert_eq!(
            m.decide(Event::Timer, t0 + Duration::from_secs(30)),
            vec![Effect::Close, Effect::ClearFeed]
        );
        assert_eq!(m.status, disconnected());

        let mut m = subscribed(t0);
        m.decide(Event::Message(schema(1)), t0);
        m.decide(Event::Message(snapshot(1, 1)), t0);
        let t1 = t0 + Duration::from_millis(2500);
        m.decide(Event::Message(snapshot(2, 1)), t1);
        assert_eq!(m.deadline, Some(t1 + Duration::from_millis(3000)));
    }

    #[test]
    fn decide_hello_timeout_and_unexpected_messages_close() {
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Connected(Ok(Some(PID))), t0);
        m.decide(Event::Queried(running()), t0);
        assert_eq!(m.deadline, Some(t0 + Duration::from_secs(2)));
        assert_eq!(
            m.decide(Event::Timer, t0 + Duration::from_secs(2)),
            vec![Effect::Close, Effect::ClearFeed]
        );

        let mut m = subscribed(t0);
        assert_eq!(
            m.decide(Event::Message(hello(PROTOCOL_VERSION)), t0),
            vec![Effect::Close, Effect::ClearFeed]
        );
        assert_eq!(m.status, disconnected());
    }

    #[test]
    fn decide_incompatible_is_kept_while_retrying() {
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Connected(Ok(Some(PID))), t0);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Message(hello(PROTOCOL_VERSION + 1)), t0);
        assert_eq!(m.status, st(ServiceState::Incompatible, None));

        let t1 = t0 + Duration::from_millis(20);
        assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Connect]);
        m.decide(Event::Connected(Err(ConnectError::NotFound)), t1);
        assert_eq!(m.status, st(ServiceState::Incompatible, None));
    }

    #[test]
    fn decide_stop_waits_for_a_stoppable_state() {
        let (mut m, t0) = machine(true);
        assert_eq!(
            m.decide(Event::Queried(in_state(RunState::StartPending)), t0),
            vec![]
        );
        assert_eq!(m.deadline, Some(t0 + Duration::from_millis(5)));
        let t1 = t0 + Duration::from_millis(5);
        assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Query]);
        assert_eq!(m.decide(Event::Queried(running()), t1), vec![Effect::Stop]);
        // 1061: the service cannot take the control yet; query and retry.
        assert_eq!(
            m.decide(Event::StopSent(Err(ERROR_SERVICE_CANNOT_ACCEPT_CTRL)), t1),
            vec![]
        );
        let t2 = t1 + Duration::from_millis(5);
        assert_eq!(m.decide(Event::Timer, t2), vec![Effect::Query]);
        assert_eq!(m.decide(Event::Queried(running()), t2), vec![Effect::Stop]);
        assert_eq!(m.decide(Event::StopSent(Ok(())), t2), vec![]);
        let t3 = t2 + Duration::from_millis(5);
        assert_eq!(m.decide(Event::Timer, t3), vec![Effect::Query]);
        // Accepted: no second STOP while it is still running.
        assert_eq!(m.decide(Event::Queried(running()), t3), vec![]);
        assert_eq!(
            m.status,
            st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
        );
        m.decide(Event::Timer, t3 + Duration::from_millis(5));
        m.decide(
            Event::Queried(in_state(RunState::Stopped)),
            t3 + Duration::from_millis(5),
        );
        assert_eq!(m.status, st(ServiceState::AntiCheat, None));
        assert_eq!(m.deadline, None);
    }

    #[test]
    fn decide_stop_gives_up_after_the_timeout() {
        let (mut m, t0) = machine(true);
        m.decide(Event::Queried(in_state(RunState::StopPending)), t0);
        let late = t0 + m.settings.stop_timeout;
        assert_eq!(m.decide(Event::Timer, late), vec![]);
        assert_eq!(
            m.status,
            st(ServiceState::AntiCheat, Some(ServiceDetail::StopFailed))
        );
        assert_eq!(m.phase, Phase::AntiCheatIdle);
        assert_eq!(m.decide(Event::Command(LinkCommand::Start), late), vec![]);
    }

    #[test]
    fn decide_start_is_ignored_while_connected() {
        let t0 = Instant::now();
        let mut m = subscribed(t0);
        assert_eq!(m.decide(Event::Command(LinkCommand::Start), t0), vec![]);
        assert_eq!(
            m.decide(Event::Command(LinkCommand::SetAntiCheat(false)), t0),
            vec![]
        );
        assert_eq!(
            m.decide(Event::Command(LinkCommand::SetAntiCheat(true)), t0),
            vec![Effect::Close, Effect::ClearFeed, Effect::Query]
        );
        assert_eq!(
            m.decide(Event::Command(LinkCommand::SetAntiCheat(true)), t0),
            vec![]
        );
    }

    #[test]
    fn decide_start_errors() {
        for (code, status) in [
            (
                ERROR_ACCESS_DENIED,
                st(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied)),
            ),
            (
                ERROR_SERVICE_DOES_NOT_EXIST,
                st(ServiceState::NotInstalled, None),
            ),
            (
                1058,
                st(ServiceState::Unreachable, Some(ServiceDetail::StartFailed)),
            ),
        ] {
            let (mut m, t0) = machine(false);
            m.decide(Event::Queried(in_state(RunState::Stopped)), t0);
            assert_eq!(m.decide(Event::Started(Err(code)), t0), vec![]);
            assert_eq!(m.status, status, "start error {code}");
        }
    }

    // ---- the real pipe ----

    #[test]
    fn pipe_connector_reads_and_writes_the_real_pipe() {
        let server = FakeServer::new();
        let connect = pipe_connector();
        let mut conn = connect(&server.name).expect("connect to the fake server");
        server.accept();
        assert_eq!(conn.server_pid(), Some(std::process::id()));

        server.send(&hello(PROTOCOL_VERSION));
        let got = conn.recv_timeout(Duration::from_secs(5)).expect("open");
        assert_eq!(got, Some(hello(PROTOCOL_VERSION)));
        assert_eq!(conn.recv_timeout(Duration::from_millis(10)).unwrap(), None);

        let subscribe = Message::Subscribe(Subscribe { interval_ms: 1000 });
        conn.send(&subscribe).expect("send");
        assert_eq!(server.recv(), subscribe);

        server.disconnect();
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            match conn.recv_timeout(Duration::from_millis(50)) {
                Err(_) => break,
                Ok(None) => assert!(Instant::now() < end, "the close never arrived"),
                Ok(Some(m)) => panic!("unexpected {m:?}"),
            }
        }
    }
}
