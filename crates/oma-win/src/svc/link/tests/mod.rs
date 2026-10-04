use std::collections::BTreeMap;
use std::sync::atomic::AtomicUsize;
use std::sync::{Condvar, Mutex};

use oma_ipc::{
    DriveState, Hello, PawnIoStatus, Reconfiguration, SourceDrive, Subscribe, WireDevice,
    WireDrive, WireError, WireSchema, WireSensor, WireServiceState, WireSnapshot, PROTOCOL_VERSION,
};

use super::machine::{Phase, ERROR_ACCESS_DENIED, UNSUPPORTED_VERSION};
use super::*;
use crate::storage::{DriveEntry, DriveIdTable};
use crate::svc::feed::SourceRequest;
use crate::svc::pipe::{CloseReason, ConnectError};
use crate::svc::scm::{RunState, ServiceQuery};
use crate::svc::status::{ServiceDetail, ServiceState, ServiceStatus, ServiceStatusTable};

pub(super) const PID: u32 = 4242;
const WAIT: Duration = Duration::from_secs(1);

pub(super) fn st(state: ServiceState, detail: Option<ServiceDetail>) -> ServiceStatus {
    ServiceStatus::new(state, detail)
}

/// The `Subscribe` of a client that asks for every source.
pub(super) fn subscribe_request(interval_ms: u32) -> Subscribe {
    Subscribe {
        interval_ms,
        disabled_modules: Vec::new(),
        smart_disabled_drives: Vec::new(),
        smart_enabled_drives: Vec::new(),
    }
}

pub(super) fn running() -> ServiceQuery {
    ServiceQuery::State {
        state: RunState::Running,
        pid: PID,
    }
}

pub(super) fn in_state(state: RunState) -> ServiceQuery {
    ServiceQuery::State { state, pid: 0 }
}

// ---- messages ----

pub(super) fn hello(version: u32) -> Message {
    Message::Hello(Hello {
        protocol_version: version,
        service_version: "test".to_owned(),
        pawn_io: "ok".to_owned(),
    })
}

pub(super) fn wire_schema(sensors: usize) -> WireSchema {
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

pub(super) fn schema(sensors: usize) -> Message {
    Message::Schema(wire_schema(sensors))
}

pub(super) fn wire_snapshot(seq: u64, values: usize) -> WireSnapshot {
    WireSnapshot {
        seq,
        timestamp_ms: seq * 1000,
        values: vec![Some(40.0); values],
        held: vec![false; values],
    }
}

pub(super) fn snapshot(seq: u64, values: usize) -> Message {
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
    gate: Arc<Gate>,
}

/// Holds the writes of a scripted connection while closed, so a test can
/// keep the link thread inside an effect for as long as it needs.
#[derive(Default)]
struct Gate {
    closed: Mutex<bool>,
    opened: Condvar,
    /// Writes started so far.
    entered: AtomicUsize,
}

impl Gate {
    fn pass(&self) {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let mut closed = self.closed.lock().unwrap();
        while *closed {
            closed = self.opened.wait(closed).unwrap();
        }
    }

    fn set(&self, closed: bool) {
        *self.closed.lock().unwrap() = closed;
        self.opened.notify_all();
    }
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
        self.gate.pass();
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
    gate: Arc<Gate>,
}

impl ConnCtl {
    /// From now on the link's writes on this connection wait for
    /// [`open_writes`](Self::open_writes).
    fn hold_writes(&self) {
        self.gate.set(true);
    }

    fn open_writes(&self) {
        self.gate.set(false);
    }

    /// Waits until the link has started `count` writes on this connection.
    fn wait_for_writes(&self, count: usize) {
        let end = Instant::now() + WAIT;
        while self.gate.entered.load(Ordering::SeqCst) < count {
            assert!(Instant::now() < end, "the link never wrote");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Snapshots the link's queue had no room for, on this connection.
    fn dropped_snapshots(&self) -> u64 {
        let pipe = self.pipe.lock().unwrap();
        pipe.sink.as_ref().map_or(0, LinkSink::dropped_snapshots)
    }

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
    let gate = Arc::new(Gate::default());
    let conn = FakeConn {
        pid,
        pipe: Arc::clone(&pipe),
        sent: Arc::clone(&sent),
        send_block: Duration::ZERO,
        alive: Arc::clone(&alive),
        gate: Arc::clone(&gate),
    };
    (
        conn,
        ConnCtl {
            pipe,
            sent,
            alive,
            gate,
        },
    )
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

pub(super) fn test_settings() -> LinkSettings {
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
        self.link
            .as_ref()
            .unwrap()
            .send(command)
            .expect("the link queue has room");
    }

    fn try_send(&self, command: LinkCommand) -> Result<(), LinkBusy> {
        self.link.as_ref().unwrap().send(command)
    }

    fn wait_for(&self, what: impl Fn(&ServiceStatus) -> bool) -> ServiceStatus {
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
    what: impl Fn(&ServiceStatus) -> bool,
    timeout: Duration,
) -> ServiceStatus {
    let end = Instant::now() + timeout;
    loop {
        let status = table.get().1;
        if what(&status) {
            return status;
        }
        if Instant::now() >= end {
            panic!("status never matched; last {status:?}");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The state and detail of a status: while connected it also carries
/// PawnIO and the sources, which most tests do not care about.
pub(super) fn shows(status: &ServiceStatus) -> (ServiceState, Option<ServiceDetail>) {
    (status.state, status.detail)
}

/// Matches on the state and its detail: while connected the status also
/// carries PawnIO and the sources, which most tests do not care about.
fn is(expected: ServiceStatus) -> impl Fn(&ServiceStatus) -> bool {
    move |s| s.state == expected.state && s.detail == expected.detail
}

pub(super) fn connected() -> ServiceStatus {
    st(ServiceState::Connected, None)
}

pub(super) fn disconnected() -> ServiceStatus {
    st(ServiceState::Unreachable, Some(ServiceDetail::Disconnected))
}

/// Lets a few retry periods (20 ms each) go by.
fn cycles(n: u32) {
    std::thread::sleep(Duration::from_millis(20) * n + Duration::from_millis(30));
}

pub(super) fn machine(anti_cheat: bool) -> (Machine, Instant) {
    let now = Instant::now();
    let mut m = Machine::new(test_settings(), anti_cheat);
    let effects = m.launch(now);
    assert_eq!(effects, vec![Effect::Query]);
    (m, now)
}

/// A machine that has just verified the connection and got `Hello`.
pub(super) fn subscribed(now: Instant) -> Machine {
    subscribed_with(
        test_settings(),
        now,
        Message::Subscribe(subscribe_request(1000)),
    )
}

/// Like [`subscribed`], for a machine with these `settings`, whose
/// `Subscribe` must be `expected`.
pub(super) fn subscribed_with(settings: LinkSettings, now: Instant, expected: Message) -> Machine {
    let mut m = Machine::new(settings, false);
    assert_eq!(m.launch(now), vec![Effect::Query]);
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
            Effect::Send(expected),
        ]
    );
    assert!(!m.reads_connection());
    assert_eq!(m.decide(Event::Sent(true), now), vec![]);
    assert!(m.reads_connection());
    m
}

mod anti_cheat;
mod launch;
mod lifecycle;
mod stream;
