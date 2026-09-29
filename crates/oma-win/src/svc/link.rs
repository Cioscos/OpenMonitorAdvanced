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
//! The thread blocks on one queue until the next event: commands from the
//! shell and, for the open connection, the messages and the close that its
//! reader forwards through a [`LinkSink`] all arrive on it. Between events it
//! sleeps until the machine's deadline (or for good when there is none), so a
//! connected link costs no timer wake-ups.
//!
//! Limits, by design:
//! - The anti-cheat preference is per user and only restrains this app: it
//!   stops the service once, verifies the stop, and then leaves the service
//!   alone. If another user, an administrator or another client starts it
//!   again, the app does not fight back with a loop of STOPs (on a PC with
//!   several users logged on, one can already stop the service for the
//!   others: the accepted multi-user limit of spec §2.2).
//! - After `Incompatible` or `PidMismatch` the link does not reconnect on its
//!   own: every connection would reset the service's idle timer, so a service
//!   that cannot be served would never idle out. It keeps asking the SCM at
//!   each `retry` and connects again only on a `Start` command or when the SCM
//!   shows the service running as something new (a restart, a new process).
//! - The reconnect loop never starts the service: only the launch probe, a
//!   `Start` command and leaving anti-cheat mode call `start()`. The launch
//!   probe starts the service only when its first conclusive answer is
//!   `Stopped` (ruling R21): a service found missing, refused, on its way
//!   down or in any other state at launch may be in the hands of an
//!   installer or an administrator, and a later `Stopped` is only reported.
//! - [`ServiceLink::shutdown`] interrupts every wait of the thread (Hello,
//!   first sample, stream, retry, STOP verification) at once, but not a call
//!   already running: a pipe write can block for up to 2 s (the client's
//!   write timeout) and an SCM call for as long as the SCM takes. `shutdown`
//!   waits for the thread at most [`JOIN_WAIT`]; past that it detaches it,
//!   and the thread exits by itself as soon as the call returns, without
//!   touching the connection again.

use std::collections::{HashSet, VecDeque};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oma_ipc::{Message, Subscribe, WireError, WireSchema, WireSnapshot, PROTOCOL_VERSION};

use super::feed::SvcFeed;
use super::pipe::{CloseReason, ConnectError, PipeClient, PipeEvent, PipeReader};
use super::scm::{RunState, ServiceControl, ServiceQuery};
use super::status::{ServiceDetail, ServiceState, ServiceStatus, ServiceStatusTable};

/// Longest [`ServiceLink::shutdown`] waits for the thread before detaching it.
pub const JOIN_WAIT: Duration = Duration::from_millis(500);

/// Win32 codes the rules tell apart.
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;
const ERROR_SERVICE_CANNOT_ACCEPT_CTRL: u32 = 1061;

/// Launch probe: how many times a transient query error is retried before
/// it counts as the answer (ruling R21).
const LAUNCH_QUERY_RETRIES: u8 = 3;

/// `WireError::code` of a service that does not speak our protocol version.
const UNSUPPORTED_VERSION: &str = "unsupported_version";

/// An open connection to the sensor pipe.
pub trait Connection: Send {
    /// PID of the process serving the pipe, read on this connection.
    fn server_pid(&self) -> Option<u32>;
    fn send(&mut self, msg: &Message) -> std::io::Result<()>;
}

/// Where a connection delivers what it receives: the link thread's queue,
/// tagged with the connection's id so the thread can discard what arrives
/// late from a connection it has already dropped.
///
/// Delivery never blocks. The queue is not bounded: the thread drains it
/// between short effects, and the service sends one snapshot per interval.
#[derive(Clone)]
pub struct LinkSink {
    id: u64,
    tx: Sender<Input>,
}

impl LinkSink {
    #[cfg(test)]
    fn new(id: u64, tx: Sender<Input>) -> Self {
        Self { id, tx }
    }

    /// Hands over a message; `false` when the link is gone.
    pub fn message(&self, msg: Message) -> bool {
        self.tx.send(Input::Message(self.id, msg)).is_ok()
    }

    /// Reports that the connection ended. Sent at most once per connection.
    pub fn closed(&self, reason: CloseReason) {
        let _ = self.tx.send(Input::Closed(self.id, reason));
    }
}

/// Opens a [`Connection`] to the named pipe; its reader delivers to `sink`.
pub type Connector =
    Arc<dyn Fn(&str, LinkSink) -> Result<Box<dyn Connection>, ConnectError> + Send + Sync>;

/// The real connector: a [`PipeClient`] whose reader thread forwards
/// messages and the close to the link's queue.
pub fn pipe_connector() -> Connector {
    Arc::new(|pipe_name: &str, sink: LinkSink| {
        let client = PipeClient::connect(pipe_name)?;
        let reader = client.start_reader_with(move |event| match event {
            PipeEvent::Message(msg) => sink.message(msg),
            PipeEvent::Closed(reason) => {
                sink.closed(reason);
                true
            }
        });
        Ok(Box::new(PipeConnection {
            reader: Some(reader),
            client,
        }) as Box<dyn Connection>)
    })
}

/// [`PipeClient`] and its reader as a [`Connection`]. Dropping it stops the
/// reader and then releases the client: the kernel handle closes only when
/// both are gone.
struct PipeConnection {
    reader: Option<PipeReader>,
    client: PipeClient,
}

impl Connection for PipeConnection {
    fn server_pid(&self) -> Option<u32> {
        self.client.server_pid()
    }

    fn send(&mut self, msg: &Message) -> std::io::Result<()> {
        self.client.send(msg)
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
    /// The sampling interval changed (milliseconds): a connected link
    /// subscribes again at once, any other link uses it on its next
    /// connection.
    SetInterval(u32),
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
/// device, with a non-empty kind and name without `/`, and a unique
/// `device_id/kind/name`.
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
    let mut sensors = HashSet::with_capacity(schema.sensors.len());
    for sensor in &schema.sensors {
        if !devices.contains(sensor.device_id.as_str()) {
            return Err(format!(
                "sensor {:?} refers to unknown device {:?}",
                sensor.name, sensor.device_id
            ));
        }
        if sensor.kind.is_empty() || sensor.kind.contains('/') {
            return Err(format!("invalid sensor kind {:?}", sensor.kind));
        }
        if sensor.name.is_empty() || sensor.name.contains('/') {
            return Err(format!("invalid sensor name {:?}", sensor.name));
        }
        let id = (
            sensor.device_id.as_str(),
            sensor.kind.as_str(),
            sensor.name.as_str(),
        );
        if !sensors.insert(id) {
            return Err(format!(
                "duplicate sensor id {}/{}/{}",
                sensor.device_id, sensor.kind, sensor.name
            ));
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
    /// Launch probe: the query is out; `errors` transient errors so far.
    Probing { errors: u8 },
    /// Launch probe: query again at the deadline.
    ProbeWait { errors: u8 },
    /// A `start()` is out.
    StartSent,
    /// Connect at the deadline.
    ConnectWait,
    /// The connector is out.
    Connecting,
    /// The connection failed; the query that tells why is out.
    Refreshing { error: ConnectError },
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
    /// Anti-cheat stop: the query is out. In these three phases a cleared
    /// `anti_cheat` means the preference was turned off while a STOP was on
    /// its way: the one `start()` follows the confirmed stop (or its timeout).
    StopQuery { since: Instant, stop_sent: bool },
    /// Anti-cheat stop: `stop()` is out.
    StopSent { since: Instant },
    /// Anti-cheat stop: query again at the deadline.
    StopWait { since: Instant, stop_sent: bool },
    /// Anti-cheat mode, stop confirmed or failed: nothing to do.
    AntiCheatIdle,
    /// `Incompatible` or `PidMismatch`: no reconnection by itself. At the
    /// deadline the SCM is asked whether the service changed since `baseline`
    /// (unknown until the first answer).
    Held { baseline: Option<ServiceQuery> },
    /// Held; the SCM query is out.
    HeldQuery { baseline: Option<ServiceQuery> },
}

impl Phase {
    /// The verified stop is in progress.
    fn is_stopping(self) -> bool {
        matches!(
            self,
            Phase::StopQuery { .. } | Phase::StopSent { .. } | Phase::StopWait { .. }
        )
    }

    /// A STOP has been accepted (or is out) and the service may still be
    /// on its way down.
    fn stop_in_flight(self) -> bool {
        matches!(
            self,
            Phase::StopSent { .. }
                | Phase::StopQuery {
                    stop_sent: true,
                    ..
                }
                | Phase::StopWait {
                    stop_sent: true,
                    ..
                }
        )
    }

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

/// The `Subscribe` this client sends. Until the settings wire them in, it asks for every
/// module and every disk's SMART.
fn subscribe_request(interval_ms: u32) -> Subscribe {
    Subscribe {
        interval_ms,
        disabled_modules: Vec::new(),
        smart_disabled_drives: Vec::new(),
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
    /// The service PID confirmed by the last accepted connection: what the
    /// SCM showed when the service turned out to be incompatible.
    verified_pid: Option<u32>,
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
            phase: Phase::Probing { errors: 0 },
            deadline: None,
            grace_until: None,
            verified_pid: None,
        }
    }

    /// The first effects, at launch: the verified stop in anti-cheat mode,
    /// the launch probe otherwise.
    fn launch(&mut self, now: Instant) -> Vec<Effect> {
        if self.anti_cheat {
            self.begin_stop(now)
        } else {
            self.go(Phase::Probing { errors: 0 }, None);
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
            (Event::Queried(q), Phase::Probing { errors }) => self.on_probe(q, errors, now),
            (Event::Queried(q), Phase::Refreshing { error }) => self.on_refresh(q, error, now),
            (Event::Queried(q), Phase::Verifying { server_pid }) => {
                self.on_verify(q, server_pid, now)
            }
            (Event::Queried(q), Phase::StopQuery { since, stop_sent }) => {
                self.on_stop_query(q, since, stop_sent, now)
            }
            (Event::Queried(q), Phase::HeldQuery { baseline }) => {
                self.on_held_query(q, baseline, now)
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
            // A renewed subscription (`SetInterval`) that could not be written.
            (Event::Sent(false), Phase::FirstSample { .. } | Phase::Streaming { .. }) => {
                self.close(disconnected(), now)
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
            LinkCommand::SetAntiCheat(true) if !self.anti_cheat && self.phase.is_stopping() => {
                // Back on before the pending start: the stop simply goes on.
                self.anti_cheat = true;
                self.status = status(ServiceState::AntiCheat, Some(ServiceDetail::Stopping));
                Vec::new()
            }
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
                if self.phase.stop_in_flight() {
                    // Wait for the stop to be confirmed, then start once.
                    self.status = status(ServiceState::Starting, None);
                    Vec::new()
                } else {
                    self.start()
                }
            }
            LinkCommand::Start
                if !self.anti_cheat
                    && !self.phase.has_connection()
                    && !self.phase.is_stopping() =>
            {
                self.start()
            }
            LinkCommand::SetInterval(ms) => self.set_interval(ms, now),
            // Same preference again, or Start while anti-cheat, connected or
            // already due after a pending stop.
            _ => Vec::new(),
        }
    }

    /// Remembers the new interval. If a subscription is active it is renewed
    /// at once (the service accepts a second `Subscribe`), and while
    /// streaming the silence limit restarts from now at three new intervals:
    /// without that, a longer interval could trip the limit of the old one
    /// before the service has sent its next snapshot at the new pace. The
    /// deadline before the first snapshot is left alone.
    fn set_interval(&mut self, ms: u32, now: Instant) -> Vec<Effect> {
        if ms == self.settings.interval_ms {
            return Vec::new();
        }
        self.settings.interval_ms = ms;
        match self.phase {
            Phase::FirstSample { .. } => {}
            Phase::Streaming { .. } => {
                self.deadline = Some(now + self.settings.interval() * 3);
            }
            // Not subscribed: `on_hello` sends the stored value.
            _ => return Vec::new(),
        }
        vec![
            Effect::SetInterval(self.settings.interval()),
            Effect::Send(Message::Subscribe(subscribe_request(ms))),
        ]
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

    /// Closes the connection and empties the feed; reconnects after `retry`,
    /// never starting the service. Ends any start grace.
    fn close(&mut self, status: ServiceStatus, now: Instant) -> Vec<Effect> {
        self.status = status;
        self.grace_until = None;
        self.connect_later(now);
        vec![Effect::Close, Effect::ClearFeed]
    }

    /// Closes the connection and empties the feed like [`close`](Self::close),
    /// but does not reconnect by itself: see [`Phase::Held`].
    fn hold(
        &mut self,
        status: ServiceStatus,
        baseline: Option<ServiceQuery>,
        now: Instant,
    ) -> Vec<Effect> {
        self.status = status;
        self.grace_until = None;
        self.go(Phase::Held { baseline }, Some(now + self.settings.retry));
        vec![Effect::Close, Effect::ClearFeed]
    }

    /// The SCM's answer while held: connect again only if the service is
    /// running (or starting) as something new. Anything else just becomes the
    /// new baseline, so that the next `Running` counts as a change; errors
    /// say nothing.
    fn on_held_query(
        &mut self,
        query: ServiceQuery,
        baseline: Option<ServiceQuery>,
        now: Instant,
    ) -> Vec<Effect> {
        let mut baseline = baseline;
        match query {
            ServiceQuery::State {
                state: RunState::Running | RunState::StartPending,
                ..
            } if baseline.is_some_and(|b| b != query) => {
                tracing::info!("sensor service changed ({query:?}); connecting again");
                return self.connect_now();
            }
            ServiceQuery::State { .. } | ServiceQuery::NotInstalled => baseline = Some(query),
            ServiceQuery::Error(_) | ServiceQuery::AccessDenied => {}
        }
        self.go(Phase::Held { baseline }, Some(now + self.settings.retry));
        Vec::new()
    }

    fn on_timer(&mut self, now: Instant) -> Vec<Effect> {
        if self.deadline.is_none_or(|d| now < d) {
            return Vec::new();
        }
        match self.phase {
            Phase::ProbeWait { errors } => {
                self.go(Phase::Probing { errors }, None);
                vec![Effect::Query]
            }
            Phase::ConnectWait => self.connect_now(),
            Phase::Held { baseline } => {
                self.go(Phase::HeldQuery { baseline }, None);
                vec![Effect::Query]
            }
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

    /// The launch probe (ruling R21): the service is started only when the
    /// first conclusive answer is `Stopped`; a transient error is asked
    /// again at most [`LAUNCH_QUERY_RETRIES`] times. Any other answer leads
    /// to the connect loop, which never starts the service.
    fn on_probe(&mut self, query: ServiceQuery, errors: u8, now: Instant) -> Vec<Effect> {
        match query {
            ServiceQuery::Error(code) if errors < LAUNCH_QUERY_RETRIES => {
                tracing::info!("cannot query the sensor service (error {code}); asking again");
                self.go(
                    Phase::ProbeWait { errors: errors + 1 },
                    Some(now + self.settings.retry),
                );
                return Vec::new();
            }
            ServiceQuery::Error(code) => {
                tracing::warn!("cannot query the sensor service: error {code}");
                self.status = status(ServiceState::Unreachable, None);
            }
            ServiceQuery::NotInstalled => {
                self.status = status(ServiceState::NotInstalled, None);
            }
            ServiceQuery::AccessDenied => {
                self.status = status(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied));
            }
            ServiceQuery::State { state, .. } => match state {
                RunState::Stopped => return self.start(),
                RunState::StartPending => {
                    self.status = status(ServiceState::Starting, None);
                    self.grace_until = Some(now + self.settings.start_grace);
                    return self.connect_now();
                }
                RunState::Running => {
                    self.status = status(ServiceState::Starting, None);
                    return self.connect_now();
                }
                // On its way down or in a state we do not drive: leave it be.
                RunState::StopPending | RunState::Other(_) => {
                    tracing::info!("sensor service is {state:?} at launch; not starting it");
                    self.status = disconnected();
                }
            },
        }
        self.connect_later(now);
        Vec::new()
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
                self.connect_later(now);
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
            Err(_) if self.grace_until.is_some_and(|g| now < g) => {
                self.status = status(ServiceState::Starting, None);
                self.connect_later(now);
                Vec::new()
            }
            Err(error) => {
                self.go(Phase::Refreshing { error }, None);
                vec![Effect::Query]
            }
        }
    }

    /// No pipe (or no access to it): the SCM says why. Queries only, never
    /// a start; the connect loop goes on after `retry`.
    fn on_refresh(
        &mut self,
        query: ServiceQuery,
        error: ConnectError,
        now: Instant,
    ) -> Vec<Effect> {
        let denied = status(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied));
        match query {
            ServiceQuery::NotInstalled => self.status = status(ServiceState::NotInstalled, None),
            ServiceQuery::AccessDenied => self.status = denied,
            // Transient: nothing new to say.
            ServiceQuery::Error(code) => {
                tracing::debug!("cannot query the sensor service: error {code}");
            }
            ServiceQuery::State { state, .. } => match state {
                RunState::Running if error == ConnectError::AccessDenied => self.status = denied,
                // The service still speaks another protocol, as far as we know.
                RunState::Running if self.status.state == ServiceState::Incompatible => {}
                // Started by someone else: a first sighting opens a grace.
                RunState::StartPending if self.grace_until.is_none() => {
                    self.grace_until = Some(now + self.settings.start_grace);
                    self.status = status(ServiceState::Starting, None);
                }
                // Stopped after our start() failed: that failure still says why.
                RunState::Stopped
                    if self.status.state == ServiceState::Unreachable
                        && matches!(
                            self.status.detail,
                            Some(ServiceDetail::AccessDenied | ServiceDetail::StartFailed)
                        ) => {}
                _ => self.status = disconnected(),
            },
        }
        self.connect_later(now);
        Vec::new()
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
                self.grace_until = None;
                self.verified_pid = Some(pid);
                self.go(Phase::Hello, Some(now + self.settings.hello_timeout));
                Vec::new()
            }
            ServiceQuery::AccessDenied => self.close(
                status(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied)),
                now,
            ),
            ServiceQuery::NotInstalled => {
                tracing::warn!(
                    "sensor pipe served by pid {server_pid:?} while the service is not \
                     installed; disconnecting"
                );
                self.close(status(ServiceState::NotInstalled, None), now)
            }
            ServiceQuery::Error(code) => {
                tracing::warn!("cannot verify the sensor pipe server: query error {code}");
                self.close(disconnected(), now)
            }
            other => {
                let mismatch = status(ServiceState::Unreachable, Some(ServiceDetail::PidMismatch));
                if self.status != mismatch {
                    tracing::warn!(
                        "sensor pipe served by pid {server_pid:?}, not by the service \
                         ({other:?}); disconnecting"
                    );
                }
                // Only a change at the SCM (or "Avvia") is worth another try.
                self.hold(mismatch, Some(other), now)
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
                    Effect::Send(Message::Subscribe(subscribe_request(
                        self.settings.interval_ms,
                    ))),
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
                self.hold_incompatible(now)
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
            self.hold_incompatible(now)
        } else {
            self.close(disconnected(), now)
        }
    }

    /// The service speaks another protocol: it stays so until it is
    /// restarted, which the SCM shows as a new process.
    fn hold_incompatible(&mut self, now: Instant) -> Vec<Effect> {
        let baseline = self.verified_pid.map(|pid| ServiceQuery::State {
            state: RunState::Running,
            pid,
        });
        self.hold(status(ServiceState::Incompatible, None), baseline, now)
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
            // Turned off before any STOP went out: nothing to wait for.
            ServiceQuery::State {
                state: RunState::Running | RunState::Other(_),
                ..
            } if !stop_sent && !self.anti_cheat => self.start(),
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
        if !self.anti_cheat {
            // Turned off while the STOP was on its way: the one start.
            return self.start();
        }
        self.status = status(ServiceState::AntiCheat, None);
        self.go(Phase::AntiCheatIdle, None);
        Vec::new()
    }

    /// The preference stays on: no connection and no start until it is
    /// turned off, and no further STOP either. If it was turned off while
    /// waiting, the one `start()` follows.
    fn stop_failed(&mut self, why: &str) -> Vec<Effect> {
        tracing::warn!("anti-cheat mode could not stop the sensor service: {why}");
        if !self.anti_cheat {
            // Turned off meanwhile: the wait is over, start once anyway.
            return self.start();
        }
        self.status = status(ServiceState::AntiCheat, Some(ServiceDetail::StopFailed));
        self.go(Phase::AntiCheatIdle, None);
        Vec::new()
    }
}

/// What reaches the link thread on its one queue.
enum Input {
    Command(LinkCommand),
    Shutdown,
    /// A message of connection `.0`.
    Message(u64, Message),
    /// Connection `.0` ended.
    Closed(u64, CloseReason),
}

/// Ids of connections, unique across links, so that an event tagged with
/// the id of a dropped connection can be told from the current one's.
static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

/// The link thread: waits for events and runs the machine's effects.
struct Driver {
    machine: Machine,
    control: Arc<dyn ServiceControl>,
    connector: Connector,
    status: ServiceStatusTable,
    feed: SvcFeed,
    /// The one queue: commands, shutdown, and what the connection's reader
    /// forwards.
    inbox: Receiver<Input>,
    /// A sender on `inbox`, for the sinks of new connections.
    sender: Sender<Input>,
    stop: Arc<AtomicBool>,
    conn: Option<Box<dyn Connection>>,
    /// Id of the current connection (the one `conn` holds, or that is being
    /// opened): events tagged with another id are discarded.
    conn_id: Option<u64>,
    /// Events of the current connection that arrived while the machine was
    /// not reading it (for instance the server's `Hello` during the PID
    /// check), in order.
    unread: VecDeque<Event>,
    /// Results of effects, handled before any new wait.
    pending: VecDeque<Event>,
    /// How many times the thread went to sleep on `inbox`.
    #[cfg(test)]
    waits: Arc<AtomicUsize>,
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
    /// Sleeps on the queue until the machine's deadline, or until something
    /// arrives when there is no deadline.
    fn next_event(&mut self) -> Option<Event> {
        loop {
            if self.stopping() {
                return None;
            }
            if self.machine.reads_connection() {
                if let Some(event) = self.unread.pop_front() {
                    return Some(event);
                }
            }
            // What is already queued comes before a deadline that falls due
            // at the same time: a command is not delayed by a retry.
            let input = match self.inbox.try_recv() {
                Ok(input) => input,
                Err(TryRecvError::Disconnected) => return None,
                Err(TryRecvError::Empty) => {
                    let left = match self.machine.deadline {
                        Some(deadline) => {
                            let now = Instant::now();
                            if now >= deadline {
                                return Some(Event::Timer);
                            }
                            Some(deadline - now)
                        }
                        None => None,
                    };
                    #[cfg(test)]
                    self.waits.fetch_add(1, Ordering::Relaxed);
                    match left {
                        Some(left) => match self.inbox.recv_timeout(left) {
                            Ok(input) => input,
                            Err(RecvTimeoutError::Timeout) => continue,
                            Err(RecvTimeoutError::Disconnected) => return None,
                        },
                        None => self.inbox.recv().ok()?,
                    }
                }
            };
            match input {
                Input::Command(command) => return Some(Event::Command(command)),
                Input::Shutdown => return None,
                Input::Message(id, msg) if self.conn_id == Some(id) => {
                    self.unread.push_back(Event::Message(msg));
                }
                Input::Closed(id, reason) if self.conn_id == Some(id) => {
                    self.unread.push_back(Event::Closed(reason));
                }
                // A late event of a connection that is already gone.
                Input::Message(..) | Input::Closed(..) => {}
            }
        }
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Query => self.pending.push_back(Event::Queried(self.control.query())),
                Effect::Start => self.pending.push_back(Event::Started(self.control.start())),
                Effect::Stop => self.pending.push_back(Event::StopSent(self.control.stop())),
                Effect::Connect => {
                    let id = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
                    self.conn_id = Some(id);
                    self.unread.clear();
                    let sink = LinkSink {
                        id,
                        tx: self.sender.clone(),
                    };
                    let result = match (self.connector)(&self.machine.settings.pipe_name, sink) {
                        Ok(conn) => {
                            let pid = conn.server_pid();
                            self.conn = Some(conn);
                            Ok(pid)
                        }
                        Err(e) => {
                            self.conn_id = None;
                            Err(e)
                        }
                    };
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
                Effect::Close => {
                    self.conn = None;
                    self.conn_id = None;
                    self.unread.clear();
                }
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
    #[cfg(test)]
    waits: Arc<AtomicUsize>,
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
        let (commands, inbox) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let waits = Arc::new(AtomicUsize::new(0));
        let driver = Driver {
            machine,
            control,
            connector,
            status,
            feed,
            inbox,
            sender: commands.clone(),
            stop: Arc::clone(&stop),
            conn: None,
            conn_id: None,
            unread: VecDeque::new(),
            pending: VecDeque::new(),
            #[cfg(test)]
            waits: Arc::clone(&waits),
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
            #[cfg(test)]
            waits,
        }
    }

    /// Never blocks.
    pub fn send(&self, command: LinkCommand) {
        let _ = self.commands.send(Input::Command(command));
    }

    /// How many times the thread has gone to sleep waiting for an event.
    #[cfg(test)]
    fn waits(&self) -> usize {
        self.waits.load(Ordering::Relaxed)
    }

    /// Tells the thread to stop. The thread holds a sender of its own queue,
    /// so it would not notice the link being dropped otherwise.
    fn signal_stop(&self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.commands.send(Input::Shutdown);
    }

    /// Stops the thread, closing any connection. Returns within
    /// [`JOIN_WAIT`] (plus a few milliseconds): a thread still inside a
    /// blocking call by then is detached (see the module notes).
    pub fn shutdown(mut self) {
        self.signal_stop();
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

impl Drop for ServiceLink {
    /// A link dropped without [`shutdown`](Self::shutdown) still stops its
    /// thread (which is then left to exit by itself).
    fn drop(&mut self) {
        self.signal_stop();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicUsize;
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
            pawn_io: "ok".to_owned(),
        })
    }

    fn wire_schema(sensors: usize) -> WireSchema {
        WireSchema {
            service: Default::default(),
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

    /// Where a scripted connection's events go: nowhere until the link opens
    /// the connection and hands over its sink, which flushes what was queued.
    #[derive(Default)]
    struct FakePipe {
        sink: Option<LinkSink>,
        queued: Vec<Step>,
    }

    impl FakePipe {
        fn deliver(&mut self, step: Step) {
            match &self.sink {
                Some(sink) => match step {
                    Step::Msg(m) => {
                        sink.message(m);
                    }
                    Step::Close(r) => sink.closed(r),
                },
                None => self.queued.push(step),
            }
        }
    }

    struct FakeConn {
        pid: Option<u32>,
        pipe: Arc<Mutex<FakePipe>>,
        sent: Arc<Mutex<Vec<Message>>>,
        send_block: Duration,
        alive: Arc<AtomicBool>,
    }

    impl FakeConn {
        /// The link opened this connection: events flow from now on.
        fn attach(&self, sink: LinkSink) {
            let mut pipe = self.pipe.lock().unwrap();
            pipe.sink = Some(sink);
            for step in std::mem::take(&mut pipe.queued) {
                pipe.deliver(step);
            }
        }
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
    }

    /// The test's end of a scripted connection. It keeps working after the
    /// link dropped the connection, like a reader that still has events in
    /// flight.
    #[derive(Clone)]
    struct ConnCtl {
        pipe: Arc<Mutex<FakePipe>>,
        sent: Arc<Mutex<Vec<Message>>>,
        alive: Arc<AtomicBool>,
    }

    impl ConnCtl {
        fn push(&self, msg: Message) {
            self.pipe.lock().unwrap().deliver(Step::Msg(msg));
        }

        fn close(&self) {
            self.pipe
                .lock()
                .unwrap()
                .deliver(Step::Close(CloseReason::Disconnected));
        }

        fn sent(&self) -> Vec<Message> {
            self.sent.lock().unwrap().clone()
        }

        fn is_alive(&self) -> bool {
            self.alive.load(Ordering::SeqCst)
        }
    }

    fn fake_conn(pid: Option<u32>) -> (FakeConn, ConnCtl) {
        let pipe = Arc::new(Mutex::new(FakePipe::default()));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let conn = FakeConn {
            pid,
            pipe: Arc::clone(&pipe),
            sent: Arc::clone(&sent),
            send_block: Duration::ZERO,
            alive: Arc::clone(&alive),
        };
        (conn, ConnCtl { pipe, sent, alive })
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
            Arc::new(move |_name: &str, sink: LinkSink| {
                *script.connects.lock().unwrap() += 1;
                let next = script.conns.lock().unwrap().pop_front();
                next.unwrap_or(Err(ConnectError::NotFound)).map(|c| {
                    c.attach(sink);
                    Box::new(c) as Box<dyn Connection>
                })
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

    /// A retry long enough to observe a status before the next attempt
    /// refreshes it.
    fn slow_retry() -> LinkSettings {
        LinkSettings {
            retry: Duration::from_millis(300),
            ..test_settings()
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
        assert!(h.control.queries() >= 3, "the query repeats at every retry");
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
        let h = Harness::spawn_with(control, Script::with(vec![conn]), false, slow_retry());
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
        let h = Harness::spawn_with(control, Script::with(vec![conn]), false, slow_retry());
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
        let h = Harness::spawn_with(control, Script::with(vec![conn]), false, slow_retry());
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
        // The SCM is asked at every retry, but the service is not connected
        // again (each connection would reset its idle timer).
        let queries = h.control.queries();
        cycles(5);
        assert!(h.control.queries() >= queries + 3, "the SCM is still asked");
        assert_eq!(h.script.connects(), 1, "no reconnection by itself");
        assert_eq!(h.status(), incompatible);

        // "Avvia" tries again.
        h.send(LinkCommand::Start);
        cycles(2);
        assert!(h.script.connects() >= 2, "the command connects again");
        assert_eq!(h.control.starts(), 1, "started once, at the command");
    }

    #[test]
    fn a_v1_service_hello_on_the_wire_is_incompatible_not_disconnected() {
        // The bytes a protocol v1 service sends (no `pawn_io`), decoded the way the pipe
        // reader does, must reach the version check.
        const V1_HELLO: [u8; 58] = [
            0x82, 0xa4, 0x74, 0x79, 0x70, 0x65, 0xa5, 0x68, 0x65, 0x6c, 0x6c, 0x6f, 0xa4, 0x62,
            0x6f, 0x64, 0x79, 0x82, 0xb0, 0x70, 0x72, 0x6f, 0x74, 0x6f, 0x63, 0x6f, 0x6c, 0x5f,
            0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x1, 0xaf, 0x73, 0x65, 0x72, 0x76, 0x69,
            0x63, 0x65, 0x5f, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0xa5, 0x30, 0x2e, 0x31,
            0x2e, 0x30,
        ];
        let old_hello = oma_ipc::decode_payload(&V1_HELLO).expect("a v1 hello decodes");
        let control = FakeControl::new(running());
        let (conn, ctl) = fake_conn(Some(PID));
        ctl.push(old_hello);
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(st(ServiceState::Incompatible, None)));
        assert!(ctl.sent().is_empty(), "no Subscribe to a v1 service");
    }

    #[test]
    fn incompatible_is_not_retried_until_start() {
        // Fake time: the rules alone, no clock.
        let t0 = Instant::now();
        let (mut m, _) = machine(false);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Connected(Ok(Some(PID))), t0);
        m.decide(Event::Queried(running()), t0);
        assert_eq!(
            m.decide(Event::Message(hello(PROTOCOL_VERSION + 1)), t0),
            vec![Effect::Close, Effect::ClearFeed]
        );
        assert_eq!(m.status, st(ServiceState::Incompatible, None));

        // 20 s of retries with the same running service: the SCM is asked
        // each time, and no connection is made.
        let retry = m.settings.retry;
        let mut t = t0;
        while t < t0 + Duration::from_secs(20) {
            t += retry;
            assert_eq!(
                m.decide(Event::Timer, t),
                vec![Effect::Query],
                "at {:?}",
                t - t0
            );
            assert_eq!(m.decide(Event::Queried(running()), t), vec![]);
        }
        assert_eq!(m.status, st(ServiceState::Incompatible, None));

        // "Avvia": the service is started (a no-op if running) and connected once.
        assert_eq!(
            m.decide(Event::Command(LinkCommand::Start), t),
            vec![Effect::Start]
        );
        assert_eq!(m.decide(Event::Started(Ok(())), t), vec![Effect::Connect]);
    }

    #[test]
    fn a_restarted_incompatible_service_is_connected_again() {
        let t0 = Instant::now();
        let (mut m, _) = machine(false);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Connected(Ok(Some(PID))), t0);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Message(hello(PROTOCOL_VERSION + 1)), t0);

        let retry = m.settings.retry;
        let t1 = t0 + retry;
        m.decide(Event::Timer, t1);
        assert_eq!(
            m.decide(Event::Queried(in_state(RunState::Stopped)), t1),
            vec![]
        );
        // Upgraded and started by someone else: a new process.
        let t2 = t1 + retry;
        assert_eq!(m.decide(Event::Timer, t2), vec![Effect::Query]);
        let upgraded = ServiceQuery::State {
            state: RunState::Running,
            pid: PID + 9,
        };
        assert_eq!(
            m.decide(Event::Queried(upgraded), t2),
            vec![Effect::Connect]
        );
    }

    #[test]
    fn pid_mismatch_waits_for_an_scm_change() {
        let t0 = Instant::now();
        let (mut m, _) = machine(false);
        let pid1 = ServiceQuery::State {
            state: RunState::Running,
            pid: 1,
        };
        let pid2 = ServiceQuery::State {
            state: RunState::Running,
            pid: 2,
        };
        assert_eq!(m.decide(Event::Queried(pid1), t0), vec![Effect::Connect]);
        // Someone else serves the pipe.
        m.decide(Event::Connected(Ok(Some(99))), t0);
        assert_eq!(
            m.decide(Event::Queried(pid1), t0),
            vec![Effect::Close, Effect::ClearFeed]
        );
        assert_eq!(
            m.status,
            st(ServiceState::Unreachable, Some(ServiceDetail::PidMismatch))
        );

        let retry = m.settings.retry;
        let mut t = t0;
        let mut step = |m: &mut Machine, answer: ServiceQuery| {
            t += retry;
            assert_eq!(m.decide(Event::Timer, t), vec![Effect::Query]);
            m.decide(Event::Queried(answer), t)
        };
        // Unchanged, then stopped: still no connection.
        assert_eq!(step(&mut m, pid1), vec![]);
        assert_eq!(step(&mut m, pid1), vec![]);
        assert_eq!(step(&mut m, in_state(RunState::Stopped)), vec![]);
        assert_eq!(step(&mut m, in_state(RunState::Stopped)), vec![]);
        // A transient error says nothing.
        assert_eq!(step(&mut m, ServiceQuery::Error(1115)), vec![]);
        // Running again as a new process: one new connection.
        assert_eq!(step(&mut m, pid2), vec![Effect::Connect]);
        assert_eq!(m.phase, Phase::Connecting);
    }

    #[test]
    fn a_pid_mismatch_is_retried_after_the_service_finishes_starting() {
        // The pipe answered while the SCM still said StartPending.
        let t0 = Instant::now();
        let (mut m, _) = machine(false);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Connected(Ok(Some(PID))), t0);
        let pending = ServiceQuery::State {
            state: RunState::StartPending,
            pid: PID,
        };
        m.decide(Event::Queried(pending), t0);
        let t1 = t0 + m.settings.retry;
        m.decide(Event::Timer, t1);
        assert_eq!(
            m.decide(Event::Queried(running()), t1),
            vec![Effect::Connect]
        );
    }

    #[test]
    fn a_threaded_pid_mismatch_reconnects_only_on_an_scm_change() {
        let control = FakeControl::new(ServiceQuery::State {
            state: RunState::Running,
            pid: 1,
        });
        let (impostor, _ctl) = streaming_conn(Some(77));
        let h = Harness::spawn(Arc::clone(&control), Script::with(vec![impostor]), false);
        h.wait_for(is(st(
            ServiceState::Unreachable,
            Some(ServiceDetail::PidMismatch),
        )));
        cycles(6);
        assert_eq!(h.script.connects(), 1, "no reconnection while unchanged");

        control.set_query(in_state(RunState::Stopped));
        cycles(4);
        assert_eq!(h.script.connects(), 1, "a stopped service is not connected");
        let (real, _ctl2) = streaming_conn(Some(2));
        h.script.add(real);
        control.set_query(ServiceQuery::State {
            state: RunState::Running,
            pid: 2,
        });
        h.wait_for(is(connected()));
        assert_eq!(h.script.connects(), 2);
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
            vec![Message::Subscribe(subscribe_request(1000))]
        );
        assert_eq!(h.control.starts(), 0, "a running service is not started");

        ctl.push(snapshot(2, 2));
        let end = Instant::now() + WAIT;
        while h.feed.view().snapshot.map(|(_, s)| s.seq) != Some(2) {
            assert!(Instant::now() < end, "the second snapshot never arrived");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Waits until the connection has received `count` messages.
    fn wait_for_sent(ctl: &ConnCtl, count: usize) -> Vec<Message> {
        let end = Instant::now() + WAIT;
        loop {
            let sent = ctl.sent();
            if sent.len() >= count {
                return sent;
            }
            assert!(Instant::now() < end, "only {} messages sent", sent.len());
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn set_interval_resubscribes_when_connected() {
        let control = FakeControl::new(running());
        let (conn, ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));

        h.send(LinkCommand::SetInterval(2000));
        let sent = wait_for_sent(&ctl, 2);
        assert_eq!(
            sent,
            vec![
                Message::Subscribe(subscribe_request(1000)),
                Message::Subscribe(subscribe_request(2000)),
            ]
        );
        let end = Instant::now() + WAIT;
        while h.feed.view().interval != Duration::from_millis(2000) {
            assert!(Instant::now() < end, "the feed interval never changed");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(h.status(), connected(), "the connection is kept");
        assert_eq!(h.script.connects(), 1);

        // The same interval again is not sent twice.
        h.send(LinkCommand::SetInterval(2000));
        cycles(2);
        assert_eq!(ctl.sent().len(), 2);
    }

    #[test]
    fn a_resubscribe_does_not_make_the_provider_rediscover() {
        let control = FakeControl::new(running());
        let (conn, ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));
        let generation = h.feed.view().generation;

        h.send(LinkCommand::SetInterval(2000));
        wait_for_sent(&ctl, 2);
        // The service answers with its schema again, then a snapshot.
        ctl.push(schema(2));
        ctl.push(snapshot(2, 2));
        let end = Instant::now() + WAIT;
        while h.feed.view().snapshot.map(|(_, s)| s.seq) != Some(2) {
            assert!(Instant::now() < end, "the second snapshot never arrived");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            h.feed.view().generation,
            generation,
            "the same schema must not invalidate what the provider bound"
        );
    }

    #[test]
    fn set_interval_while_disconnected_is_used_on_connect() {
        let control = FakeControl::new(running());
        let h = Harness::spawn(control, Script::with(vec![]), false);
        // No pipe yet: the link keeps retrying.
        h.wait_for(is(disconnected()));
        h.send(LinkCommand::SetInterval(2000));
        cycles(2);
        let (conn, ctl) = streaming_conn(Some(PID));
        h.script.add(conn);
        h.wait_for(is(connected()));
        assert_eq!(
            ctl.sent(),
            vec![Message::Subscribe(subscribe_request(2000))]
        );
        assert_eq!(h.feed.view().interval, Duration::from_millis(2000));
    }

    #[test]
    fn the_silence_limit_follows_the_new_interval() {
        let control = FakeControl::new(running());
        let (conn, _ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));
        // With 1000 ms the link would wait 3 s (much longer than WAIT); with
        // 40 ms it gives up after 120 ms.
        h.send(LinkCommand::SetInterval(40));
        h.wait_for(is(disconnected()));
    }

    #[test]
    fn set_interval_moves_the_silence_deadline_of_a_streaming_link() {
        let now = Instant::now();
        let mut machine = Machine::new(LinkSettings::new("p", 1000), false);
        machine.phase = Phase::Streaming { schema_len: 2 };
        machine.deadline = Some(now + Duration::from_secs(3));

        let effects = machine.decide(Event::Command(LinkCommand::SetInterval(5000)), now);
        assert_eq!(
            effects,
            vec![
                Effect::SetInterval(Duration::from_millis(5000)),
                Effect::Send(Message::Subscribe(subscribe_request(5000))),
            ]
        );
        assert_eq!(machine.deadline, Some(now + Duration::from_secs(15)));
        assert_eq!(machine.phase, Phase::Streaming { schema_len: 2 });

        // Same value: nothing to do.
        assert!(machine
            .decide(Event::Command(LinkCommand::SetInterval(5000)), now)
            .is_empty());
    }

    #[test]
    fn set_interval_before_the_first_sample_keeps_the_first_sample_deadline() {
        let now = Instant::now();
        let mut machine = Machine::new(LinkSettings::new("p", 1000), false);
        let deadline = now + Duration::from_secs(30);
        machine.phase = Phase::FirstSample { schema_len: None };
        machine.deadline = Some(deadline);
        let effects = machine.decide(Event::Command(LinkCommand::SetInterval(500)), now);
        assert_eq!(
            effects,
            vec![
                Effect::SetInterval(Duration::from_millis(500)),
                Effect::Send(Message::Subscribe(subscribe_request(500))),
            ]
        );
        assert_eq!(machine.deadline, Some(deadline));
    }

    #[test]
    fn a_failed_resubscribe_disconnects() {
        let now = Instant::now();
        let mut machine = Machine::new(LinkSettings::new("p", 1000), false);
        machine.phase = Phase::Streaming { schema_len: 2 };
        machine.deadline = Some(now + Duration::from_secs(3));
        assert!(machine.decide(Event::Sent(true), now).is_empty());
        assert_eq!(machine.phase, Phase::Streaming { schema_len: 2 });
        let effects = machine.decide(Event::Sent(false), now);
        assert_eq!(effects, vec![Effect::Close, Effect::ClearFeed]);
        assert_eq!(machine.phase, Phase::ConnectWait);
    }

    #[test]
    fn set_interval_in_the_other_phases_only_stores_the_value() {
        let now = Instant::now();
        for phase in [
            Phase::ConnectWait,
            Phase::Hello,
            Phase::AntiCheatIdle,
            Phase::Probing { errors: 0 },
        ] {
            let mut machine = Machine::new(LinkSettings::new("p", 1000), false);
            machine.phase = phase;
            let effects = machine.decide(Event::Command(LinkCommand::SetInterval(3000)), now);
            assert!(effects.is_empty(), "{phase:?}");
            assert_eq!(machine.settings.interval_ms, 3000, "{phase:?}");
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

    // ---- one queue, no wake-ups ----

    /// A driver around `machine` for tests that call `next_event` directly.
    fn bare_driver(machine: Machine) -> (Driver, Sender<Input>) {
        let (tx, rx) = mpsc::channel();
        let driver = Driver {
            machine,
            control: FakeControl::new(running()),
            connector: Script::with(vec![]).connector(),
            status: ServiceStatusTable::default(),
            feed: SvcFeed::default(),
            inbox: rx,
            sender: tx.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            conn: None,
            conn_id: Some(7),
            unread: VecDeque::new(),
            pending: VecDeque::new(),
            waits: Arc::new(AtomicUsize::new(0)),
        };
        (driver, tx)
    }

    #[test]
    fn connected_link_does_not_wake_between_events() {
        let control = FakeControl::new(running());
        let (conn, ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));
        let waits = h.link.as_ref().unwrap().waits();

        // Interval 1000 ms, no message: the silence limit is 3 s away.
        std::thread::sleep(Duration::from_secs(1));
        let idle = h.link.as_ref().unwrap().waits() - waits;
        assert!(idle <= 3, "the thread woke {idle} times in 1 s");
        assert_eq!(h.status(), connected());

        // And it is still listening.
        ctl.push(snapshot(2, 2));
        let end = Instant::now() + WAIT;
        while h.feed.view().snapshot.map(|(_, s)| s.seq) != Some(2) {
            assert!(Instant::now() < end, "the second snapshot never arrived");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn messages_and_commands_share_one_queue() {
        // In order, without waiting between them.
        let t0 = Instant::now();
        let (mut driver, tx) = bare_driver(subscribed(t0));
        tx.send(Input::Message(7, schema(2))).unwrap();
        tx.send(Input::Command(LinkCommand::SetInterval(2000)))
            .unwrap();
        tx.send(Input::Message(7, snapshot(1, 2))).unwrap();
        assert!(matches!(
            driver.next_event(),
            Some(Event::Message(Message::Schema(_)))
        ));
        assert!(matches!(
            driver.next_event(),
            Some(Event::Command(LinkCommand::SetInterval(2000)))
        ));
        assert!(matches!(
            driver.next_event(),
            Some(Event::Message(Message::Snapshot(_)))
        ));
        assert_eq!(
            driver.waits.load(Ordering::Relaxed),
            0,
            "nothing to wait for"
        );
    }

    #[test]
    fn a_command_sent_while_a_snapshot_arrives_is_handled_at_once() {
        let control = FakeControl::new(running());
        let (conn, ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));

        // The old 50 ms read slice would take up to 50 ms; here the bound is
        // generous for a loaded machine but still under one slice.
        let start = Instant::now();
        ctl.push(snapshot(2, 2));
        h.send(LinkCommand::SetInterval(2000));
        wait_for_sent(&ctl, 2);
        let took = start.elapsed();
        assert!(took < Duration::from_millis(45), "took {took:?}");
    }

    #[test]
    fn late_events_of_a_closed_connection_are_ignored() {
        let control = FakeControl::new(running());
        let (first, ctl1) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![first]), false);
        h.wait_for(is(connected()));
        let (second, _ctl2) = streaming_conn(Some(PID));
        h.script.add(second);
        ctl1.close();
        h.wait_for(is(disconnected()));
        h.wait_for(is(connected()));
        let generation = h.feed.view().generation;

        // The first connection's reader still had events in flight.
        ctl1.push(schema(5));
        ctl1.push(snapshot(9, 5));
        ctl1.close();
        cycles(3);
        assert_eq!(h.status(), connected(), "the new connection is untouched");
        let view = h.feed.view();
        assert_eq!(view.generation, generation);
        assert_eq!(view.schema.map(|s| s.sensors.len()), Some(2));
        assert_eq!(view.snapshot.map(|(_, s)| s.seq), Some(1));
    }

    #[test]
    fn events_of_another_connection_are_dropped_by_next_event() {
        let t0 = Instant::now();
        let (mut driver, tx) = bare_driver(subscribed(t0));
        tx.send(Input::Message(6, schema(9))).unwrap();
        tx.send(Input::Closed(6, CloseReason::Disconnected))
            .unwrap();
        tx.send(Input::Message(7, schema(2))).unwrap();
        assert!(matches!(
            driver.next_event(),
            Some(Event::Message(Message::Schema(s))) if s.sensors.len() == 2
        ));
    }

    #[test]
    fn a_hello_that_arrives_while_verifying_is_not_lost() {
        // The server greets at once; the machine is still checking the PID.
        let control = FakeControl::new(running());
        let (conn, ctl) = fake_conn(Some(PID));
        ctl.push(hello(PROTOCOL_VERSION));
        ctl.push(schema(2));
        ctl.push(snapshot(1, 2));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));
        assert_eq!(h.feed.view().snapshot.map(|(_, s)| s.seq), Some(1));
    }

    #[test]
    fn unread_messages_wait_for_the_machine_and_keep_their_order() {
        let t0 = Instant::now();
        let (m, _) = machine(false);
        let (mut driver, tx) = bare_driver(m);
        // Verifying: not reading the connection yet.
        driver.machine.phase = Phase::Verifying {
            server_pid: Some(PID),
        };
        driver.machine.deadline = Some(t0 + Duration::from_millis(30));
        tx.send(Input::Message(7, hello(PROTOCOL_VERSION))).unwrap();
        tx.send(Input::Closed(7, CloseReason::Disconnected))
            .unwrap();
        assert!(matches!(driver.next_event(), Some(Event::Timer)));

        driver.machine.phase = Phase::Hello;
        driver.machine.deadline = None;
        assert!(matches!(
            driver.next_event(),
            Some(Event::Message(Message::Hello(_)))
        ));
        assert!(matches!(driver.next_event(), Some(Event::Closed(_))));
    }

    #[test]
    fn dropping_the_link_without_shutdown_stops_the_thread() {
        let control = FakeControl::new(running());
        let (conn, ctl) = streaming_conn(Some(PID));
        let mut h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));
        assert!(ctl.is_alive());
        drop(h.link.take());
        let end = Instant::now() + WAIT;
        while ctl.is_alive() {
            assert!(Instant::now() < end, "the thread never stopped");
            std::thread::sleep(Duration::from_millis(1));
        }
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
        // No retry fires during the test: only the command can use the
        // scripted connection.
        let settings = LinkSettings {
            retry: Duration::from_secs(10),
            ..test_settings()
        };
        let h = Harness::spawn_with(Arc::clone(&control), Script::with(vec![]), false, settings);
        h.wait_for(is(st(
            ServiceState::Unreachable,
            Some(ServiceDetail::StartFailed),
        )));
        control.with(|s| s.start_result = Ok(()));
        let (conn, _ctl) = streaming_conn(Some(PID));
        h.script.add(conn);
        h.send(LinkCommand::Start);
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

    // ---- fix round 1: launch rule R21 and fresh reasons ----

    /// The service turns up stopped; the link reports it and never starts it.
    fn assert_never_started(h: &Harness, expected: ServiceStatus) {
        h.control.set_query(in_state(RunState::Stopped));
        h.wait_for(is(expected));
        cycles(5);
        assert_eq!(h.control.starts(), 0);
        assert_eq!(h.status(), expected);
    }

    #[test]
    fn a_service_being_stopped_at_launch_is_not_restarted() {
        let control = FakeControl::new(in_state(RunState::StopPending));
        let h = Harness::spawn(control, Script::with(vec![]), false);
        h.wait_for(is(disconnected()));
        assert_never_started(&h, disconnected());
    }

    #[test]
    fn a_service_in_another_state_at_launch_is_not_started() {
        let control = FakeControl::new(in_state(RunState::Other(7)));
        let h = Harness::spawn(control, Script::with(vec![]), false);
        h.wait_for(is(disconnected()));
        assert_never_started(&h, disconnected());
    }

    #[test]
    fn a_service_installed_after_launch_is_not_started_automatically() {
        let control = FakeControl::new(ServiceQuery::NotInstalled);
        let h = Harness::spawn(control, Script::with(vec![]), false);
        h.wait_for(is(st(ServiceState::NotInstalled, None)));
        assert_never_started(&h, disconnected());
    }

    #[test]
    fn access_denied_at_launch_is_never_followed_by_a_start() {
        let control = FakeControl::new(ServiceQuery::AccessDenied);
        let h = Harness::spawn(control, Script::with(vec![]), false);
        let denied = st(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied));
        h.wait_for(is(denied));
        assert_never_started(&h, denied);
    }

    #[test]
    fn transient_query_error_at_launch_is_retried_at_most_three_times() {
        let control = FakeControl::new(ServiceQuery::Error(1115));
        let h = Harness::spawn(control, Script::with(vec![]), false);
        h.wait_for(is(st(ServiceState::Unreachable, None)));
        assert!(h.control.queries() >= 4);
        assert_never_started(&h, disconnected());
    }

    #[test]
    fn a_vanished_impostor_is_reported_as_disconnected() {
        let control = FakeControl::new(running());
        let (conn, _ctl) = streaming_conn(Some(PID + 1));
        let h = Harness::spawn_with(control, Script::with(vec![conn]), false, slow_retry());
        h.wait_for(is(st(
            ServiceState::Unreachable,
            Some(ServiceDetail::PidMismatch),
        )));
        // No automatic retry now: the user's "Avvia" tries again, and finds
        // no pipe (the SCM says why).
        cycles(1);
        h.send(LinkCommand::Start);
        wait_for(&h.status, is(disconnected()), Duration::from_secs(2));
    }

    #[test]
    fn an_uninstalled_service_is_reported_as_not_installed() {
        let control = FakeControl::new(running());
        let (conn, ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(control, Script::with(vec![conn]), false);
        h.wait_for(is(connected()));
        h.control.set_query(ServiceQuery::NotInstalled);
        ctl.close();
        h.wait_for(is(st(ServiceState::NotInstalled, None)));
        assert_eq!(h.control.starts(), 0);
    }

    #[test]
    fn disabling_anti_cheat_during_the_stop_waits_then_starts_once() {
        let control = FakeControl::new(running());
        control.with(|s| s.after_stop = Some(in_state(RunState::StopPending)));
        let (conn, _ctl) = streaming_conn(Some(PID));
        let h = Harness::spawn(Arc::clone(&control), Script::with(vec![conn]), true);
        let end = Instant::now() + WAIT;
        while control.stops() == 0 {
            assert!(Instant::now() < end, "STOP never sent");
            std::thread::sleep(Duration::from_millis(1));
        }

        h.send(LinkCommand::SetAntiCheat(false));
        h.wait_for(is(st(ServiceState::Starting, None)));
        cycles(3);
        assert_eq!(control.starts(), 0, "no start while the STOP is pending");
        assert_eq!(h.script.connects(), 0);

        control.set_query(in_state(RunState::Stopped));
        h.wait_for(is(connected()));
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

        // Idle in anti-cheat mode: stop confirmed, no deadline at all.
        let control = FakeControl::new(in_state(RunState::Stopped));
        let mut h = Harness::spawn(control, Script::with(vec![]), true);
        h.wait_for(is(st(ServiceState::AntiCheat, None)));
        assert_quick_shutdown(&mut h, "idle in anti-cheat mode");

        // In the retry wait.
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
        // The same name under another kind is another sensor id.
        let mut schema = wire_schema(2);
        schema.sensors[1].name = schema.sensors[0].name.clone();
        schema.sensors[1].kind = "load".to_owned();
        assert_eq!(validate_schema(&schema), Ok(()));
    }

    #[test]
    fn validate_schema_rejects_what_the_provider_cannot_bind() {
        type Spoil = fn(&mut WireSchema);
        let cases: [(&str, Spoil); 9] = [
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
            ("duplicate sensor id", |s| {
                s.sensors.push(s.sensors[1].clone())
            }),
            ("empty sensor kind", |s| s.sensors[0].kind.clear()),
            ("slash in sensor kind", |s| {
                s.sensors[0].kind = "a/b".to_owned()
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
                Effect::Send(Message::Subscribe(subscribe_request(1000))),
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
        // Past the grace the reason comes from the SCM.
        assert_eq!(m.decide(not_found(), t2), vec![Effect::Query]);
        assert_eq!(m.decide(Event::Queried(running()), t2), vec![]);
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
        assert_eq!(
            m.decide(Event::Connected(Err(ConnectError::NotFound)), t0),
            vec![Effect::Query]
        );
        m.decide(Event::Queried(running()), t0);
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
        // Not a plain retry: the SCM must show a change before the next try.
        assert_eq!(
            m.phase,
            Phase::Held {
                baseline: Some(restarted)
            }
        );
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
    fn decide_incompatible_is_kept_across_an_explicit_start() {
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Connected(Ok(Some(PID))), t0);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Message(hello(PROTOCOL_VERSION + 1)), t0);
        assert_eq!(m.status, st(ServiceState::Incompatible, None));

        let t1 = t0 + Duration::from_millis(20);
        assert_eq!(
            m.decide(Event::Command(LinkCommand::Start), t1),
            vec![Effect::Start]
        );
        assert_eq!(m.decide(Event::Started(Ok(())), t1), vec![Effect::Connect]);
        assert_eq!(
            m.decide(Event::Connected(Ok(Some(PID))), t1),
            vec![Effect::Query]
        );
        m.decide(Event::Queried(running()), t1);
        // Still the same service: the state is reported again, and held again.
        assert_eq!(
            m.decide(Event::Message(hello(PROTOCOL_VERSION + 1)), t1),
            vec![Effect::Close, Effect::ClearFeed]
        );
        assert_eq!(m.status, st(ServiceState::Incompatible, None));
        assert!(matches!(m.phase, Phase::Held { .. }));
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

    #[test]
    fn decide_launch_starts_only_when_the_first_answer_is_stopped() {
        let denied = st(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied));
        for (first, status, later) in [
            (
                ServiceQuery::NotInstalled,
                st(ServiceState::NotInstalled, None),
                disconnected(),
            ),
            (ServiceQuery::AccessDenied, denied, denied),
            (
                in_state(RunState::StopPending),
                disconnected(),
                disconnected(),
            ),
            (in_state(RunState::Other(7)), disconnected(), disconnected()),
        ] {
            let (mut m, t0) = machine(false);
            assert_eq!(m.decide(Event::Queried(first), t0), vec![], "{first:?}");
            assert_eq!(m.status, status, "{first:?}");
            // Later a stopped service is reported, never started.
            let t1 = t0 + Duration::from_millis(20);
            assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Connect]);
            assert_eq!(
                m.decide(Event::Connected(Err(ConnectError::NotFound)), t1),
                vec![Effect::Query]
            );
            assert_eq!(
                m.decide(Event::Queried(in_state(RunState::Stopped)), t1),
                vec![]
            );
            assert_eq!(m.status, later, "{first:?} then Stopped");
        }
    }

    #[test]
    fn decide_launch_error_is_retried_three_times_then_conclusive() {
        let (mut m, t0) = machine(false);
        let error = || Event::Queried(ServiceQuery::Error(1115));
        let mut t = t0;
        for _ in 0..3 {
            assert_eq!(m.decide(error(), t), vec![]);
            assert_eq!(m.status, st(ServiceState::Starting, None));
            t += Duration::from_millis(20);
            assert_eq!(m.decide(Event::Timer, t), vec![Effect::Query]);
        }
        assert_eq!(m.decide(error(), t), vec![]);
        assert_eq!(m.status, st(ServiceState::Unreachable, None));
        t += Duration::from_millis(20);
        // The connect loop now, not the probe: a stopped service stays stopped.
        assert_eq!(m.decide(Event::Timer, t), vec![Effect::Connect]);
        assert_eq!(
            m.decide(Event::Connected(Err(ConnectError::NotFound)), t),
            vec![Effect::Query]
        );
        assert_eq!(
            m.decide(Event::Queried(in_state(RunState::Stopped)), t),
            vec![]
        );
        assert_eq!(m.status, disconnected());
    }

    #[test]
    fn decide_stopped_after_transient_errors_is_the_first_answer() {
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(ServiceQuery::Error(1115)), t0);
        let t1 = t0 + Duration::from_millis(20);
        assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Query]);
        assert_eq!(
            m.decide(Event::Queried(in_state(RunState::Stopped)), t1),
            vec![Effect::Start]
        );
    }

    #[test]
    fn decide_explicit_starts_never_reenter_the_launch_probe() {
        // A Start command on a service that turns out not installed.
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(ServiceQuery::NotInstalled), t0);
        let t1 = t0 + Duration::from_millis(5);
        assert_eq!(
            m.decide(Event::Command(LinkCommand::Start), t1),
            vec![Effect::Start]
        );
        assert_eq!(
            m.decide(Event::Started(Err(ERROR_SERVICE_DOES_NOT_EXIST)), t1),
            vec![]
        );
        assert_eq!(m.status, st(ServiceState::NotInstalled, None));
        let t2 = t1 + Duration::from_millis(20);
        assert_eq!(m.decide(Event::Timer, t2), vec![Effect::Connect]);
        assert_eq!(
            m.decide(Event::Connected(Err(ConnectError::NotFound)), t2),
            vec![Effect::Query]
        );
        assert_eq!(
            m.decide(Event::Queried(in_state(RunState::Stopped)), t2),
            vec![]
        );
    }

    #[test]
    fn decide_connect_loop_refreshes_the_reason_from_the_scm() {
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Connected(Err(ConnectError::NotFound)), t0);
        m.decide(Event::Queried(running()), t0);
        let denied = st(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied));
        assert_eq!(m.status, disconnected());
        let mut t = t0;
        for (answer, expected) in [
            (in_state(RunState::Stopped), disconnected()),
            (
                ServiceQuery::NotInstalled,
                st(ServiceState::NotInstalled, None),
            ),
            (ServiceQuery::AccessDenied, denied),
            // A transient error says nothing new.
            (ServiceQuery::Error(1115), denied),
            (running(), disconnected()),
            (in_state(RunState::StopPending), disconnected()),
            (
                in_state(RunState::StartPending),
                st(ServiceState::Starting, None),
            ),
        ] {
            t += Duration::from_millis(20);
            assert_eq!(m.decide(Event::Timer, t), vec![Effect::Connect]);
            assert_eq!(
                m.decide(Event::Connected(Err(ConnectError::NotFound)), t),
                vec![Effect::Query]
            );
            assert_eq!(m.decide(Event::Queried(answer), t), vec![], "{answer:?}");
            assert_eq!(m.status, expected, "{answer:?}");
        }
        // StartPending opened a grace: no query while it lasts.
        t += Duration::from_millis(20);
        m.decide(Event::Timer, t);
        assert_eq!(
            m.decide(Event::Connected(Err(ConnectError::NotFound)), t),
            vec![]
        );
        assert_eq!(m.status, st(ServiceState::Starting, None));
    }

    #[test]
    fn decide_a_refused_pipe_on_a_running_service_is_access_denied() {
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(running()), t0);
        assert_eq!(
            m.decide(Event::Connected(Err(ConnectError::AccessDenied)), t0),
            vec![Effect::Query]
        );
        m.decide(Event::Queried(running()), t0);
        assert_eq!(
            m.status,
            st(ServiceState::Unreachable, Some(ServiceDetail::AccessDenied))
        );
    }

    #[test]
    fn decide_grace_ends_when_a_connection_is_accepted_or_closed() {
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(in_state(RunState::Stopped)), t0);
        m.decide(Event::Started(Ok(())), t0);
        assert!(m.grace_until.is_some());
        m.decide(Event::Connected(Ok(Some(PID))), t0);
        m.decide(Event::Queried(running()), t0);
        assert_eq!(m.grace_until, None, "accepted");
        m.decide(Event::Closed(CloseReason::Disconnected), t0);
        // Well within the old grace, a missing pipe is no longer "starting".
        let t1 = t0 + Duration::from_millis(20);
        m.decide(Event::Timer, t1);
        assert_eq!(
            m.decide(Event::Connected(Err(ConnectError::NotFound)), t1),
            vec![Effect::Query]
        );

        // Closed before being accepted: an impostor right after our start().
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(in_state(RunState::Stopped)), t0);
        m.decide(Event::Started(Ok(())), t0);
        m.decide(Event::Connected(Ok(Some(PID + 1))), t0);
        m.decide(Event::Queried(running()), t0);
        assert_eq!(m.grace_until, None, "closed");
    }

    #[test]
    fn decide_query_error_during_verification_is_disconnected() {
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Connected(Ok(Some(PID))), t0);
        assert_eq!(
            m.decide(Event::Queried(ServiceQuery::Error(1115)), t0),
            vec![Effect::Close, Effect::ClearFeed]
        );
        assert_eq!(m.status, disconnected());
    }

    #[test]
    fn decide_a_service_gone_during_verification_is_not_installed() {
        let (mut m, t0) = machine(false);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::Connected(Ok(Some(PID))), t0);
        assert_eq!(
            m.decide(Event::Queried(ServiceQuery::NotInstalled), t0),
            vec![Effect::Close, Effect::ClearFeed]
        );
        assert_eq!(m.status, st(ServiceState::NotInstalled, None));
    }

    #[test]
    fn decide_disabling_anti_cheat_waits_for_a_pending_stop() {
        let (mut m, t0) = machine(true);
        assert_eq!(m.decide(Event::Queried(running()), t0), vec![Effect::Stop]);
        assert_eq!(m.decide(Event::StopSent(Ok(())), t0), vec![]);
        assert_eq!(
            m.decide(Event::Command(LinkCommand::SetAntiCheat(false)), t0),
            vec![]
        );
        assert_eq!(m.status, st(ServiceState::Starting, None));
        // A Start command changes nothing: the start is already due.
        assert_eq!(m.decide(Event::Command(LinkCommand::Start), t0), vec![]);
        let t1 = t0 + Duration::from_millis(5);
        assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Query]);
        assert_eq!(
            m.decide(Event::Queried(in_state(RunState::StopPending)), t1),
            vec![]
        );
        let t2 = t1 + Duration::from_millis(5);
        assert_eq!(m.decide(Event::Timer, t2), vec![Effect::Query]);
        assert_eq!(
            m.decide(Event::Queried(in_state(RunState::Stopped)), t2),
            vec![Effect::Start]
        );
        assert_eq!(m.decide(Event::Started(Ok(())), t2), vec![Effect::Connect]);
    }

    #[test]
    fn decide_a_stop_timeout_after_disabling_still_starts_once() {
        let (mut m, t0) = machine(true);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::StopSent(Ok(())), t0);
        m.decide(Event::Command(LinkCommand::SetAntiCheat(false)), t0);
        let late = t0 + m.settings.stop_timeout;
        assert_eq!(m.decide(Event::Timer, late), vec![Effect::Start]);
    }

    #[test]
    fn decide_disabling_anti_cheat_before_any_stop_starts_at_once() {
        let (mut m, t0) = machine(true);
        // Still StartPending: no STOP has gone out, nothing to wait for.
        m.decide(Event::Queried(in_state(RunState::StartPending)), t0);
        assert_eq!(
            m.decide(Event::Command(LinkCommand::SetAntiCheat(false)), t0),
            vec![Effect::Start]
        );
    }

    #[test]
    fn decide_reenabling_anti_cheat_cancels_the_pending_start() {
        let (mut m, t0) = machine(true);
        m.decide(Event::Queried(running()), t0);
        m.decide(Event::StopSent(Ok(())), t0);
        m.decide(Event::Command(LinkCommand::SetAntiCheat(false)), t0);
        assert_eq!(
            m.decide(Event::Command(LinkCommand::SetAntiCheat(true)), t0),
            vec![]
        );
        assert_eq!(
            m.status,
            st(ServiceState::AntiCheat, Some(ServiceDetail::Stopping))
        );
        let t1 = t0 + Duration::from_millis(5);
        assert_eq!(m.decide(Event::Timer, t1), vec![Effect::Query]);
        assert_eq!(
            m.decide(Event::Queried(in_state(RunState::Stopped)), t1),
            vec![]
        );
        assert_eq!(m.status, st(ServiceState::AntiCheat, None));
    }

    // ---- the real pipe ----

    #[test]
    fn pipe_connector_reads_and_writes_the_real_pipe() {
        let server = FakeServer::new();
        let connect = pipe_connector();
        let (tx, inbox) = mpsc::channel();
        let mut conn = connect(&server.name, LinkSink::new(3, tx)).expect("connect");
        server.accept();
        assert_eq!(conn.server_pid(), Some(std::process::id()));

        server.send(&hello(PROTOCOL_VERSION));
        match inbox.recv_timeout(Duration::from_secs(5)).expect("open") {
            Input::Message(3, msg) => assert_eq!(msg, hello(PROTOCOL_VERSION)),
            _ => panic!("expected the Hello of connection 3"),
        }
        assert!(inbox.recv_timeout(Duration::from_millis(10)).is_err());

        let subscribe = Message::Subscribe(subscribe_request(1000));
        conn.send(&subscribe).expect("send");
        assert_eq!(server.recv(), subscribe);

        server.disconnect();
        match inbox
            .recv_timeout(Duration::from_secs(5))
            .expect("the close")
        {
            Input::Closed(3, _) => {}
            _ => panic!("expected the close of connection 3"),
        }
    }
}
