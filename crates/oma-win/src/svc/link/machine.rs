use std::collections::HashSet;
use std::time::{Duration, Instant};

use oma_ipc::{
    DriveState, FramesConfigure, FramesTarget, Message, PawnIoStatus, Reconfiguration,
    ServiceSources, SourceDrive, Subscribe, WireError, WireSchema, WireServiceState, WireSnapshot,
    PROTOCOL_VERSION,
};

use super::super::drives::{request_keys, wire_drive_for};
use super::super::feed::SourceRequest;
use super::super::frames_feed::FramesEvent;
use super::super::pipe::{CloseReason, ConnectError};
use super::super::scm::{RunState, ServiceQuery};
use super::super::status::{ServiceDetail, ServiceState, ServiceStatus};
use crate::storage::{core_id_for_key, DriveIdTable, DriveIds};

/// Win32 codes the rules tell apart.
pub(super) const ERROR_ACCESS_DENIED: u32 = 5;
pub(super) const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;
pub(super) const ERROR_SERVICE_CANNOT_ACCEPT_CTRL: u32 = 1061;

/// Launch probe: how many times a transient query error is retried before
/// it counts as the answer (ruling R21).
pub(super) const LAUNCH_QUERY_RETRIES: u8 = 3;

/// `WireError::code` of a service that does not speak our protocol version.
pub(super) const UNSUPPORTED_VERSION: &str = "unsupported_version";

/// What the shell asks of the link.
#[derive(Debug, Clone, PartialEq)]
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
    /// The sources the user turned off changed: a connected link subscribes
    /// again at once, any other link uses them on its next connection.
    SetSources(SourceRequest),
    /// The frame engine's configuration changed: sent once the link streams
    /// (after the first snapshot of each connection), and at once while it
    /// does, unless it equals the last one sent on this connection.
    ConfigureFrames(FramesConfigure),
    /// The process whose frames the app wants (`None` for none): sent like
    /// [`ConfigureFrames`](Self::ConfigureFrames), and only while frames are
    /// enabled.
    SetFramesTarget(Option<u32>),
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
    /// What the user turned off when the link starts (later changes arrive
    /// as [`LinkCommand::SetSources`]).
    pub sources: SourceRequest,
    /// The disks the storage provider knows: where core ids become the keys
    /// the service knows, and back.
    pub drives: DriveIdTable,
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
            sources: SourceRequest::default(),
            drives: DriveIdTable::default(),
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
pub(super) enum Event {
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
pub(super) enum Effect {
    Query,
    Start,
    Stop,
    Connect,
    Send(Message),
    /// Drop the connection (stops its reader).
    Close,
    ClearFeed,
    SetInterval(Duration),
    /// The request the provider filters by.
    SetRequest(SourceRequest),
    SetSchema(WireSchema),
    SetSnapshot(WireSnapshot),
    /// Frame data for the [`FramesFeed`](super::super::FramesFeed).
    Frames(FramesEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
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

pub(super) fn status(state: ServiceState, detail: Option<ServiceDetail>) -> ServiceStatus {
    ServiceStatus::new(state, detail)
}

pub(super) fn disconnected() -> ServiceStatus {
    status(ServiceState::Unreachable, Some(ServiceDetail::Disconnected))
}

/// The link's rules as a state machine without I/O.
pub(super) struct Machine {
    pub(super) settings: LinkSettings,
    anti_cheat: bool,
    pub(super) status: ServiceStatus,
    pub(super) phase: Phase,
    /// When [`Event::Timer`] fires.
    pub(super) deadline: Option<Instant>,
    /// A missing pipe reads as `Starting` until then.
    grace_until: Option<Instant>,
    /// The service PID confirmed by the last accepted connection: what the
    /// SCM showed when the service turned out to be incompatible.
    verified_pid: Option<u32>,
    /// `Hello.service_version` of the last service that answered, kept after
    /// it disconnects (the About page shows it).
    pub(super) service_version: Option<String>,
    /// What the user turned off: the last request, sent with every `Subscribe`.
    request: SourceRequest,
    /// The service's block of the last schema of this connection.
    service: Option<WireServiceState>,
    /// A `Subscribe` that may change the service's sources went out and no
    /// schema has answered it yet: the sources read as pending meanwhile.
    ///
    /// Accepted trade-off: a single boolean can read `Applied` briefly if an
    /// `applied` schema was already on the pipe when a second `Subscribe` went
    /// out; the service's delivery gate keeps that window narrow, and a flag
    /// that could stay set forever would be worse.
    awaiting: bool,
    /// The generation of the drive table the last `Subscribe` was built from
    /// (written only by `subscribe_message` and `on_drives_changed`, never by
    /// the translation of the published sources, which has no say in whether
    /// the service was told about a disk).
    drive_generation: u64,
    /// The drive keys of the last `Subscribe`: those switched off, then those switched on.
    sent_keys: (Vec<String>, Vec<String>),
    /// The last frame configuration the shell asked for (`None` until it asks:
    /// frames off).
    frames_config: Option<FramesConfigure>,
    /// The last frame target the shell asked for.
    frames_target: Option<u32>,
    /// What this connection's service session has been told about frames.
    frames_sent: FramesSent,
}

/// The frame requests sent on the current connection (`None`: not yet).
#[derive(Debug, Default)]
struct FramesSent {
    config: Option<FramesConfigure>,
    target: Option<Option<u32>>,
}

impl Machine {
    pub(super) fn new(settings: LinkSettings, anti_cheat: bool) -> Self {
        let request = settings.sources.clone();
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
            service_version: None,
            request,
            service: None,
            awaiting: false,
            drive_generation: 0,
            sent_keys: (Vec::new(), Vec::new()),
            frames_config: None,
            frames_target: None,
            frames_sent: FramesSent::default(),
        }
    }

    /// The first effects, at launch: the verified stop in anti-cheat mode,
    /// the launch probe otherwise.
    pub(super) fn launch(&mut self, now: Instant) -> Vec<Effect> {
        if self.anti_cheat {
            self.begin_stop(now)
        } else {
            self.go(Phase::Probing { errors: 0 }, None);
            vec![Effect::Query]
        }
    }

    /// Whether the thread should read the connection while it waits.
    pub(super) fn reads_connection(&self) -> bool {
        matches!(
            self.phase,
            Phase::Hello | Phase::FirstSample { .. } | Phase::Streaming { .. }
        )
    }

    fn go(&mut self, phase: Phase, deadline: Option<Instant>) {
        self.phase = phase;
        self.deadline = deadline;
    }

    pub(super) fn decide(&mut self, event: Event, now: Instant) -> Vec<Effect> {
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
            LinkCommand::SetSources(request) => self.set_sources(request),
            LinkCommand::ConfigureFrames(config) => {
                self.frames_config = Some(config);
                self.frames_if_streaming()
            }
            LinkCommand::SetFramesTarget(pid) => {
                self.frames_target = pid;
                self.frames_if_streaming()
            }
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
        let drives = self.settings.drives.get();
        vec![
            Effect::SetInterval(self.settings.interval()),
            Effect::Send(self.subscribe_message(&drives)),
        ]
    }

    /// Remembers what the user turned off. While a subscription is active it
    /// is renewed at once, and the sources read as pending until the service
    /// answers with a schema; otherwise the next connection sends it.
    fn set_sources(&mut self, request: SourceRequest) -> Vec<Effect> {
        if request == self.request {
            return Vec::new();
        }
        self.request = request.clone();
        let mut effects = vec![Effect::SetRequest(request)];
        if matches!(
            self.phase,
            Phase::FirstSample { .. } | Phase::Streaming { .. }
        ) {
            let drives = self.settings.drives.get();
            effects.push(Effect::Send(self.subscribe_message(&drives)));
            self.awaiting = true;
            self.refresh_sources(&drives);
        }
        effects
    }

    /// The frame requests still owed to the service, if the link streams; the
    /// rest waits for the first snapshot (the service takes them only after
    /// `Subscribe`).
    fn frames_if_streaming(&mut self) -> Vec<Effect> {
        if matches!(self.phase, Phase::Streaming { .. }) {
            self.sync_frames()
        } else {
            Vec::new()
        }
    }

    /// Sends what the service session has not been told yet: the
    /// configuration when it changed (a first one only if enabled: a session
    /// starts with frames off), then the target when it changed, only while
    /// frames are enabled (the target means nothing otherwise, and the next
    /// enabling sends it).
    fn sync_frames(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        let Some(config) = &self.frames_config else {
            return effects;
        };
        let sent = &mut self.frames_sent;
        if sent.config.as_ref() != Some(config) && (config.enabled || sent.config.is_some()) {
            sent.config = Some(config.clone());
            effects.push(Effect::Send(Message::FramesConfigure(config.clone())));
        }
        if config.enabled && sent.target != Some(self.frames_target) {
            sent.target = Some(self.frames_target);
            effects.push(Effect::Send(Message::FramesTarget(FramesTarget {
                pid: self.frames_target,
            })));
        }
        effects
    }

    /// The drive keys of the request, switched off and switched on, translated with `drives`.
    fn request_keys(&self, drives: &DriveIds) -> (Vec<String>, Vec<String>) {
        request_keys(&self.request, drives)
    }

    /// The `Subscribe` for the current interval and request, with `drives`
    /// turning core ids into keys; remembers what it was built from.
    fn subscribe_message(&mut self, drives: &DriveIds) -> Message {
        let (disabled, enabled) = self.request_keys(drives);
        self.drive_generation = drives.generation;
        self.sent_keys = (disabled.clone(), enabled.clone());
        Message::Subscribe(Subscribe {
            interval_ms: self.settings.interval_ms,
            disabled_modules: self.request.disabled_modules.clone(),
            smart_disabled_drives: disabled,
            smart_enabled_drives: enabled,
        })
    }

    /// Rebuilds the published sources from the service's last block, this
    /// client's pending request and `drives`.
    fn refresh_sources(&mut self, drives: &DriveIds) {
        let Some(block) = &self.service else {
            self.status.sources = None;
            return;
        };
        let wire = Reconfiguration::from_wire(&block.reconfiguration);
        let reconfiguration = if self.awaiting && wire != Reconfiguration::Failed {
            Reconfiguration::Pending
        } else {
            wire
        };
        let source_drives: Vec<SourceDrive> = block
            .drives
            .iter()
            .map(|drive| SourceDrive {
                physical_drive: drive.physical_drive,
                device_id: drives
                    .drives
                    .iter()
                    .find(|entry| {
                        wire_drive_for(entry, drives, block)
                            .is_some_and(|matched| std::ptr::eq(matched, drive))
                    })
                    .map(|entry| entry.device_id.clone()),
                model: drive.model.clone(),
                state: DriveState::from_wire(&drive.state),
                blocks_smart: drive.blocks_smart,
            })
            .collect();
        self.status.sources = Some(ServiceSources {
            active_modules: block.active_modules.clone(),
            requested_disabled_modules: self.request.disabled_modules.clone(),
            smart_disabled_drives: block
                .smart_disabled_drives
                .iter()
                .filter_map(|key| core_id_for_key(key, &drives.drives))
                .map(str::to_owned)
                .collect(),
            reconfiguration,
            drives: source_drives,
        });
    }

    /// The storage provider published another disk list: the keys of the
    /// request and the names in the sources may have changed with it.
    fn on_drives_changed(&mut self) -> Vec<Effect> {
        let drives = self.settings.drives.get();
        self.drive_generation = drives.generation;
        let mut effects = Vec::new();
        let names_drives = !self.request.smart_disabled_drives.is_empty()
            || !self.request.smart_enabled_drives.is_empty();
        if names_drives && self.request_keys(&drives) != self.sent_keys {
            effects.push(Effect::Send(self.subscribe_message(&drives)));
            self.awaiting = true;
        }
        self.refresh_sources(&drives);
        effects
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
    /// say nothing. A service uninstalled or stopped meanwhile is shown as
    /// such, so `Incompatible` or `PidMismatch` does not outlive it.
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
            // Gone or stopped: say so, as the connect loop would, but stay
            // held (only a `Start` or a new running service reconnects).
            ServiceQuery::NotInstalled => {
                self.status = status(ServiceState::NotInstalled, None);
                baseline = Some(query);
            }
            ServiceQuery::State {
                state: RunState::Stopped,
                ..
            } => {
                self.status = disconnected();
                baseline = Some(query);
            }
            ServiceQuery::State { .. } => baseline = Some(query),
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
                self.on_service_block(&schema.service);
                self.phase = match self.phase {
                    Phase::Streaming { .. } => Phase::Streaming { schema_len: len },
                    _ => Phase::FirstSample {
                        schema_len: Some(len),
                    },
                };
                vec![Effect::SetSchema(schema)]
            }
            Message::Snapshot(snapshot) if schema_len == Some(snapshot.values.len()) => {
                // Only the state changes: PawnIO and the sources stay.
                self.status.state = ServiceState::Connected;
                self.status.detail = None;
                let len = snapshot.values.len();
                let silence = self.settings.interval() * 3;
                let first = matches!(self.phase, Phase::FirstSample { .. });
                self.go(Phase::Streaming { schema_len: len }, Some(now + silence));
                let mut effects = vec![Effect::SetSnapshot(snapshot)];
                if self.settings.drives.generation() != self.drive_generation {
                    effects.extend(self.on_drives_changed());
                }
                if first {
                    effects.extend(self.sync_frames());
                }
                effects
            }
            Message::Snapshot(snapshot) => {
                tracing::warn!(
                    "sensor snapshot with {} values for a schema of {schema_len:?} sensors",
                    snapshot.values.len()
                );
                self.close(disconnected(), now)
            }
            Message::Error(error) => self.on_error(error, now),
            Message::FramesStatus(status) => vec![Effect::Frames(FramesEvent::Status(status))],
            Message::PresentingProcesses(list) => {
                vec![Effect::Frames(FramesEvent::Processes(list))]
            }
            Message::FrameBatch(batch) => vec![Effect::Frames(FramesEvent::Batch(batch))],
            other => {
                tracing::warn!("unexpected message from the sensor service: {other:?}");
                self.close(disconnected(), now)
            }
        }
    }

    /// Takes the service's block from a schema: an answer to a request in
    /// flight ends the wait unless the service reports it is still working.
    fn on_service_block(&mut self, block: &WireServiceState) {
        if Reconfiguration::from_wire(&block.reconfiguration) != Reconfiguration::Pending {
            self.awaiting = false;
        }
        self.service = Some(block.clone());
        let drives = self.settings.drives.get();
        self.refresh_sources(&drives);
    }

    fn on_hello(&mut self, msg: Message, now: Instant) -> Vec<Effect> {
        if let Message::Hello(hello) = &msg {
            self.service_version = Some(hello.service_version.clone());
        }
        match msg {
            Message::Hello(hello) if hello.protocol_version == PROTOCOL_VERSION => {
                self.status.pawn_io = Some(PawnIoStatus::from_wire(&hello.pawn_io));
                self.service = None;
                self.awaiting = true;
                // A new service session: it knows nothing of our frames.
                self.frames_sent = FramesSent::default();
                self.go(Phase::Subscribing, None);
                let drives = self.settings.drives.get();
                vec![
                    Effect::SetInterval(self.settings.interval()),
                    Effect::Send(self.subscribe_message(&drives)),
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

#[cfg(test)]
#[path = "machine_tests.rs"]
mod tests;
