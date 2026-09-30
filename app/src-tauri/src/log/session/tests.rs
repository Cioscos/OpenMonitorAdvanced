//! Coordinator tests: the real writer thread on the in-memory filesystem of
//! the writer tests (wrapped to delay or block flushes and creations), a fake
//! environment and hand-made ticks.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oma_core::csv::BOM;
use oma_core::engine::TickOutput;
use oma_core::model::{
    Device, DeviceKind, Label, Schema, Sensor, SensorKind, Snapshot, Source, Unit,
};
use oma_core::settings::{Language, Settings};

use super::*;
use crate::i18n::{t, Lang};
use crate::log::fs::fake::{Fault, MemFs};
use crate::log::fs::{LogFile, LogFs};
use crate::log::writer::{WriteFailure, WriterEvent};
use crate::settings::fake_fs::{open_fast, wait_until, FakeFs};
use crate::settings::SettingsStore;

/// 2026-09-29T12:03:12Z; 14:03:12 at +02:00.
const T0: u64 = 1_790_683_392_000;
const OFFSET: i32 = 120;
const DOCS: &str = "D:\\Docs";
const STEM: &str = "oma-2026-09-29_14-03-12";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// --- environment -----------------------------------------------------------

struct FakeEnv {
    docs: Mutex<Option<PathBuf>>,
    now: AtomicU64,
    window: AtomicBool,
    toasts: Mutex<Vec<(String, String)>>,
    emits: Mutex<Vec<LogStatus>>,
}

impl FakeEnv {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            docs: Mutex::new(Some(PathBuf::from(DOCS))),
            now: AtomicU64::new(T0),
            window: AtomicBool::new(true),
            toasts: Mutex::default(),
            emits: Mutex::default(),
        })
    }

    fn toasts(&self) -> Vec<(String, String)> {
        lock(&self.toasts).clone()
    }

    fn emits(&self) -> Vec<LogStatus> {
        lock(&self.emits).clone()
    }

    fn states(&self) -> Vec<LogState> {
        self.emits().iter().map(|s| s.state).collect()
    }

    fn clear(&self) {
        lock(&self.emits).clear();
    }
}

impl LogEnv for FakeEnv {
    fn default_dir(&self) -> io::Result<PathBuf> {
        lock(&self.docs)
            .clone()
            .ok_or_else(|| io::Error::from_raw_os_error(2))
    }

    fn now_ms(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }

    fn offset_minutes(&self, _unix_ms: u64) -> i32 {
        OFFSET
    }

    fn window_open(&self) -> bool {
        self.window.load(Ordering::SeqCst)
    }

    fn toast(&self, title: String, body: String) {
        lock(&self.toasts).push((title, body));
    }

    fn emit(&self, status: &LogStatus) {
        lock(&self.emits).push(status.clone());
    }
}

// --- filesystem with a gate --------------------------------------------------

#[derive(Default)]
struct GateState {
    block_flush: bool,
    block_create: bool,
    flush_delay: Duration,
    flushes_entered: u32,
    creates_entered: u32,
}

/// Blocks or delays flushes and file creations of the writer.
#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    cv: Condvar,
}

impl Gate {
    fn pass_flush(&self) {
        let mut state = lock(&self.state);
        state.flushes_entered += 1;
        self.cv.notify_all();
        while state.block_flush {
            state = self.cv.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
        let delay = state.flush_delay;
        drop(state);
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }

    fn pass_create(&self) {
        let mut state = lock(&self.state);
        state.creates_entered += 1;
        self.cv.notify_all();
        while state.block_create {
            state = self.cv.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn block_flush(&self, on: bool) {
        lock(&self.state).block_flush = on;
        self.cv.notify_all();
    }

    fn block_create(&self, on: bool) {
        lock(&self.state).block_create = on;
        self.cv.notify_all();
    }

    fn flush_delay(&self, delay: Duration) {
        lock(&self.state).flush_delay = delay;
    }

    fn flushes(&self) -> u32 {
        lock(&self.state).flushes_entered
    }

    /// Waits until `count` flushes have started.
    fn wait_flushes(&self, count: u32) {
        let mut state = lock(&self.state);
        while state.flushes_entered < count {
            state = self.cv.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

struct GatedFs {
    mem: Arc<MemFs>,
    gate: Arc<Gate>,
}

impl LogFs for GatedFs {
    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        LogFs::create_dir_all(&*self.mem, dir)
    }

    fn create_new(&self, path: &Path) -> io::Result<Box<dyn LogFile>> {
        self.gate.pass_create();
        let inner = LogFs::create_new(&*self.mem, path)?;
        Ok(Box::new(GatedFile {
            inner,
            gate: self.gate.clone(),
        }))
    }
}

struct GatedFile {
    inner: Box<dyn LogFile>,
    gate: Arc<Gate>,
}

impl LogFile for GatedFile {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_all(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.gate.pass_flush();
        self.inner.flush()
    }
}

// --- schemas and the rig -------------------------------------------------------

fn sensor(name: &str, label: Label) -> Sensor {
    Sensor::new(
        "cpu/0",
        SensorKind::Load,
        name,
        Unit::Percent,
        label,
        Source::Mock,
    )
}

/// CPU sensors labelled "Total load".
fn cpu(revision: u64, names: &[&str]) -> Schema {
    schema_of(
        revision,
        names
            .iter()
            .map(|name| sensor(name, Label::new("cpu.load.total")))
            .collect(),
    )
}

/// `count` thread-load sensors: wide rows.
fn wide(revision: u64, count: usize) -> Schema {
    schema_of(
        revision,
        (0..count)
            .map(|i| {
                sensor(
                    &format!("t{i}"),
                    Label::with_arg("cpu.load.thread", i.to_string()),
                )
            })
            .collect(),
    )
}

fn schema_of(revision: u64, sensors: Vec<Sensor>) -> Schema {
    Schema {
        revision,
        devices: vec![Device {
            id: "cpu/0".into(),
            kind: DeviceKind::Cpu,
            name: "Ryzen 7".into(),
            vendor: None,
            properties: Default::default(),
        }],
        sensors,
    }
}

fn id(name: &str) -> String {
    sensor(name, Label::new("cpu.load.total")).id
}

fn logs_dir() -> PathBuf {
    PathBuf::from(DOCS)
        .join("OpenMonitor Advanced")
        .join("logs")
}

fn file(name: &str) -> PathBuf {
    logs_dir().join(name)
}

fn base() -> PathBuf {
    file(&format!("{STEM}.csv"))
}

fn part2() -> PathBuf {
    file(&format!("{STEM}-part2.csv"))
}

struct Rig {
    log: Arc<LogService>,
    env: Arc<FakeEnv>,
    mem: Arc<MemFs>,
    gate: Arc<Gate>,
    store: Arc<SettingsStore>,
    schema: Schema,
    mono: u64,
    interval: u64,
}

impl Rig {
    fn new() -> Self {
        Self::with_timeout(CLOSE_TIMEOUT)
    }

    fn with_timeout(close_timeout: Duration) -> Self {
        let store = Arc::new(open_fast(&FakeFs::new()));
        store.update_with(|s| s.general.language = Language::En);
        let env = FakeEnv::new();
        let mem = MemFs::new();
        let gate = Arc::new(Gate::default());
        let fs = Arc::new(GatedFs {
            mem: mem.clone(),
            gate: gate.clone(),
        });
        let log = LogService::new(store.clone(), fs, env.clone(), close_timeout);
        Self {
            log,
            env,
            mem,
            gate,
            store,
            schema: cpu(1, &["total"]),
            mono: 1_000_000,
            interval: 1000,
        }
    }

    fn set(&self, change: impl FnOnce(&mut Settings)) {
        self.store.update_with(change);
    }

    fn set_interval(&mut self, ms: u32) {
        self.interval = u64::from(ms);
        self.set(|s| s.general.interval_ms = ms);
    }

    /// The next tick, every sensor at `value`; the clock moves on.
    fn next(&mut self, value: f64) -> TickOutput {
        let values = vec![Some(value); self.schema.sensors.len()];
        let out = TickOutput {
            snapshot: Snapshot {
                revision: self.schema.revision,
                seq: self.mono,
                timestamp_ms: T0 + self.mono,
                values,
            },
            schema: None,
            quality: Vec::new(),
            health: None,
            entries: Vec::new(),
            monotonic_ms: self.mono,
        };
        self.mono += self.interval;
        out
    }

    /// A tick, then a wait until the writer has taken its row and waits
    /// again: the next tick's `try_push_row` cannot meet it on the queue lock.
    fn tick(&mut self, value: f64) {
        self.tick_raw(value);
        wait_until("the writer to drain", || self.log.queue.writer_parked());
    }

    /// A tick without waiting for the writer (which may be blocked).
    fn tick_raw(&mut self, value: f64) {
        let out = self.next(value);
        self.log.on_tick(&out, &self.schema, &self.store.snapshot());
    }

    /// A tick on another thread.
    fn tick_on_thread(&mut self, value: f64) -> JoinHandle<()> {
        let out = self.next(value);
        let log = self.log.clone();
        let schema = self.schema.clone();
        let settings = self.store.snapshot();
        std::thread::spawn(move || log.on_tick(&out, &schema, &settings))
    }

    fn command(&self, run: fn(&LogService) -> LogStatus) -> JoinHandle<LogStatus> {
        let log = self.log.clone();
        std::thread::spawn(move || run(&log))
    }

    fn status(&self) -> LogStatus {
        self.log.status()
    }

    fn text(&self, path: &Path) -> String {
        let bytes = self
            .mem
            .content(path)
            .unwrap_or_else(|| panic!("{path:?} missing"));
        assert!(bytes.starts_with(BOM), "{path:?} starts with the BOM");
        String::from_utf8(bytes[BOM.len()..].to_vec()).unwrap()
    }

    fn lines(&self, path: &Path) -> Vec<String> {
        self.text(path)
            .split("\r\n")
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// The first value of every data row of `path`.
    fn values(&self, path: &Path) -> Vec<String> {
        self.lines(path)[1..]
            .iter()
            .map(|line| line.split(',').nth(1).unwrap_or("").to_owned())
            .collect()
    }

    /// First values of the rows of `path` as they were at its last flush.
    fn flushed_values(&self, path: &Path) -> Vec<String> {
        let bytes = self.mem.flushed(path).unwrap();
        String::from_utf8(bytes[BOM.len()..].to_vec())
            .unwrap()
            .split("\r\n")
            .filter(|line| !line.is_empty())
            .skip(1)
            .map(|line| line.split(',').nth(1).unwrap_or("").to_owned())
            .collect()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.gate.block_flush(false);
        self.gate.block_create(false);
        self.log.shutdown(Duration::from_secs(2));
    }
}

fn path_of(status: &LogStatus) -> Option<PathBuf> {
    status.path.as_deref().map(PathBuf::from)
}

fn key_of(status: &LogStatus) -> Option<&str> {
    status.error.as_ref().map(|e| e.key.as_str())
}

/// A one-shot rendezvous for the hooks: the hooked thread says it arrived
/// and waits for `go`.
struct Rendezvous {
    arrived: Receiver<()>,
    go: Sender<()>,
}

fn hook_at(log: &LogService, at: Point) -> Rendezvous {
    let (arrived_tx, arrived) = channel();
    let (go, go_rx) = channel::<()>();
    let fired = AtomicBool::new(false);
    let parts = Mutex::new((arrived_tx, go_rx));
    log.set_hook(Arc::new(move |point| {
        if point == at && !fired.swap(true, Ordering::SeqCst) {
            let parts = lock(&parts);
            parts.0.send(()).unwrap();
            parts.1.recv().unwrap();
        }
    }));
    Rendezvous { arrived, go }
}

// --- state machine -------------------------------------------------------------

#[test]
fn invalid_commands_return_the_state() {
    let mut rig = Rig::new();
    rig.tick(1.0);
    let before = rig.status();
    assert_eq!(rig.log.pause().state, LogState::Idle);
    assert_eq!(rig.log.resume().state, LogState::Idle);
    assert_eq!(rig.log.stop().state, LogState::Idle);
    assert_eq!(rig.status(), before, "nothing changed");
    assert!(rig.env.emits().is_empty(), "nothing emitted");

    assert_eq!(rig.log.start().state, LogState::Recording);
    assert_eq!(rig.log.resume().state, LogState::Recording);
    assert_eq!(rig.log.start().state, LogState::Recording);
    assert_eq!(rig.log.pause().state, LogState::Paused);
    assert_eq!(rig.log.pause().state, LogState::Paused);
    assert_eq!(rig.log.start().state, LogState::Paused);
    let stopped = rig.log.stop();
    assert_eq!(stopped.state, LogState::Idle);
    assert_eq!(stopped.session, 1, "a refused start opens no session");
    assert_eq!(rig.mem.files(), [base()]);
}

#[test]
fn start_uses_the_default_folder_and_the_start_time() {
    let mut rig = Rig::new();
    rig.tick(1.0);
    let status = rig.log.start();
    assert_eq!(status.state, LogState::Recording);
    assert_eq!(status.session, 1);
    assert_eq!(status.part, 1);
    assert_eq!(path_of(&status), Some(base()));
    assert_eq!(rig.mem.dirs(), [logs_dir()]);
    assert_eq!(rig.mem.files(), [base()]);
    assert_eq!(status.bytes, rig.mem.content(&base()).unwrap().len() as u64);
    assert_eq!(rig.log.folder().unwrap(), logs_dir());
    rig.log.stop();

    // A configured folder wins from the next start.
    rig.set(|s| s.log.folder = Some("E:\\logs".into()));
    rig.env.now.store(T0 + 60_000, Ordering::SeqCst);
    let status = rig.log.start();
    assert_eq!(
        path_of(&status),
        Some(PathBuf::from("E:\\logs").join("oma-2026-09-29_14-04-12.csv"))
    );
    assert_eq!(status.session, 2);
}

#[test]
fn start_without_columns_fails() {
    let mut rig = Rig::new();
    rig.set(|s| s.log.sensors = Some(vec!["gpu/0/load/nope".into()]));
    rig.tick(1.0);
    let status = rig.log.start();
    assert_eq!(status.state, LogState::Error);
    assert_eq!(key_of(&status), Some("log.error.noColumns"));
    assert!(rig.mem.files().is_empty() && rig.mem.dirs().is_empty());
    assert_eq!(rig.env.states(), [LogState::Error]);
}

#[test]
fn start_needs_a_schema() {
    let rig = Rig::new();
    let status = rig.log.start();
    assert_eq!(status.state, LogState::Error);
    assert_eq!(key_of(&status), Some("log.error.schemaUnavailable"));
    assert!(rig.mem.files().is_empty());
}

#[test]
fn schema_over_limit_fails_without_truncating() {
    let mut rig = Rig::new();
    rig.schema = wide(1, 4097);
    rig.tick(1.0);
    let status = rig.log.start();
    assert_eq!(status.state, LogState::Error);
    assert_eq!(key_of(&status), Some("log.error.tooManyColumns"));
    assert!(rig.mem.files().is_empty() && rig.mem.dirs().is_empty());

    // A selection within the limit starts.
    let chosen: Vec<String> = rig.schema.sensors[..10]
        .iter()
        .map(|s| s.id.clone())
        .collect();
    rig.set(|s| s.log.sensors = Some(chosen));
    assert_eq!(rig.log.start().state, LogState::Recording);
}

#[test]
fn start_without_ticks_leaves_a_valid_csv() {
    let mut rig = Rig::new();
    rig.tick(1.0);
    rig.log.start();
    let status = rig.log.stop();
    assert_eq!(status.state, LogState::Idle);
    assert_eq!(
        rig.lines(&base()),
        [format!(
            "Timestamp,Ryzen 7 / Total load [%] {{{}}}",
            id("total")
        )]
    );
    assert_eq!(rig.mem.flushed(&base()), rig.mem.content(&base()));
}

/// Starts a session on wide rows and fails a write while recording.
fn fail_while_recording(window_open: bool) -> Rig {
    let mut rig = Rig::new();
    rig.env.window.store(window_open, Ordering::SeqCst);
    rig.schema = wide(1, 1500);
    rig.tick(1234567.125);
    assert_eq!(rig.log.start().state, LogState::Recording);
    // Write 1 was BOM and header; the first buffer write fails.
    rig.mem.fail_write(2, Fault::Error(112));
    for _ in 0..6 {
        rig.tick(1234567.125);
    }
    wait_until("the session error", || {
        rig.status().state == LogState::Error
    });
    assert_eq!(key_of(&rig.status()), Some("log.error.diskFull"));
    rig
}

#[test]
fn start_after_error_opens_a_new_session() {
    let mut rig = fail_while_recording(true);
    assert_eq!(
        rig.log.stop().state,
        LogState::Error,
        "stop is not valid in error"
    );
    let status = rig.log.start();
    assert_eq!(status.state, LogState::Recording);
    assert_eq!(status.session, 2);
    assert_eq!(status.error, None);
    assert_eq!((status.rows, status.dropped, status.recorded_ms), (0, 0, 0));
    // Same start second: the session suffix.
    let second = file(&format!("{STEM}-2.csv"));
    assert_eq!(path_of(&status), Some(second.clone()));
    rig.tick(7.0);
    assert_eq!(rig.log.stop().state, LogState::Idle);
    assert_eq!(rig.values(&second), ["7"]);
}

// --- rows ------------------------------------------------------------------------

#[test]
fn every_ticks_records_the_first_tick_and_then_every_n() {
    let mut rig = Rig::new();
    rig.set(|s| s.log.every_ticks = 5);
    rig.tick(0.0);
    rig.log.start();
    for n in 1..=11 {
        rig.tick(f64::from(n));
    }
    rig.log.stop();
    assert_eq!(rig.values(&base()), ["1", "6", "11"]);
}

#[test]
fn every_ticks_change_resets_the_phase() {
    let mut rig = Rig::new();
    rig.set(|s| s.log.every_ticks = 5);
    rig.tick(0.0);
    rig.log.start();
    for n in 1..=3 {
        rig.tick(f64::from(n));
    }
    rig.set(|s| s.log.every_ticks = 2);
    for n in 4..=8 {
        rig.tick(f64::from(n));
    }
    rig.log.stop();
    assert_eq!(rig.values(&base()), ["1", "4", "6", "8"]);
}

#[test]
fn pause_writes_nothing_and_resume_records_at_once() {
    let mut rig = Rig::new();
    rig.set(|s| s.log.every_ticks = 5);
    rig.tick(0.0);
    rig.log.start();
    rig.tick(1.0);
    rig.tick(2.0);
    assert_eq!(rig.log.pause().state, LogState::Paused);
    for n in 3..=6 {
        rig.tick(f64::from(n));
    }
    assert_eq!(rig.log.resume().state, LogState::Recording);
    for n in 7..=12 {
        rig.tick(f64::from(n));
    }
    rig.log.stop();
    assert_eq!(rig.values(&base()), ["1", "7", "12"]);
}

#[test]
fn recorded_time_excludes_pauses() {
    let mut rig = Rig::new();
    rig.tick(0.0);
    rig.log.start();
    // The first tick anchors the clock; five intervals follow.
    for _ in 0..6 {
        rig.tick(1.0);
    }
    assert_eq!(rig.status().recorded_ms, 5000);
    assert_eq!(rig.log.pause().recorded_ms, 5000);
    for _ in 0..5 {
        rig.tick(1.0);
    }
    assert_eq!(rig.status().recorded_ms, 5000);
    rig.log.resume();
    rig.tick(1.0);
    assert_eq!(rig.status().recorded_ms, 5000, "resume re-anchors");
    rig.tick(1.0);
    assert_eq!(rig.status().recorded_ms, 6000);
    assert_eq!(rig.log.stop().recorded_ms, 6000);
}

#[test]
fn recorded_time_excludes_a_suspend_gap() {
    let mut rig = Rig::new();
    rig.tick(0.0);
    rig.log.start();
    for _ in 0..3 {
        rig.tick(1.0);
    }
    assert_eq!(rig.status().recorded_ms, 2000);
    rig.mono += 600_000;
    rig.tick(1.0);
    assert_eq!(rig.status().recorded_ms, 2000, "a suspend adds nothing");
    rig.tick(1.0);
    assert_eq!(rig.status().recorded_ms, 3000);
    // Within max(3 × interval, 5 s) a late tick still counts.
    rig.mono += 4000;
    rig.tick(1.0);
    assert_eq!(rig.status().recorded_ms, 8000);
}

#[test]
fn language_change_opens_a_new_part() {
    let mut rig = Rig::new();
    rig.tick(0.0);
    rig.log.start();
    rig.tick(1.0);
    rig.set(|s| s.general.language = Language::It);
    rig.tick(2.0);
    let status = rig.log.stop();
    // `MemFs::files` is sorted: `-part2` before `.csv`.
    assert_eq!(rig.mem.files(), [part2(), base()]);
    assert_eq!(status.part, 2);
    let italian = t(Lang::It, "sensor.cpu.load.total", &[]);
    assert!(!rig.lines(&base())[0].contains(&italian));
    assert!(rig.lines(&part2())[0].contains(&italian));
    assert_eq!(rig.values(&base()), ["1"]);
    assert_eq!(rig.values(&part2()), ["2"]);
}

#[test]
fn schema_change_outside_the_selection_keeps_the_part() {
    let mut rig = Rig::new();
    rig.set(|s| s.log.sensors = Some(vec![id("total")]));
    rig.tick(0.0);
    rig.log.start();
    rig.tick(1.0);
    // Another sensor arrives before the chosen one: its index moves.
    rig.schema = cpu(2, &["other", "total"]);
    rig.tick(2.0);
    rig.log.stop();
    assert_eq!(rig.mem.files(), [base()]);
    assert_eq!(rig.values(&base()), ["1", "2"]);
}

#[test]
fn selected_sensor_arriving_opens_a_new_part() {
    let mut rig = Rig::new();
    rig.set(|s| s.log.sensors = Some(vec![id("total"), id("extra")]));
    rig.tick(0.0);
    rig.log.start();
    rig.tick(1.0);
    rig.schema = cpu(2, &["total", "extra"]);
    rig.tick(2.0);
    rig.log.stop();
    assert_eq!(rig.mem.files(), [part2(), base()]);
    let lines = rig.lines(&part2());
    assert!(lines[0].contains(&id("extra")));
    assert_eq!(lines[1].split(',').count(), 3);
}

#[test]
fn rows_with_only_timestamps_when_every_column_disappears() {
    let mut rig = Rig::new();
    rig.set(|s| s.log.sensors = Some(vec![id("total")]));
    rig.tick(0.0);
    rig.log.start();
    rig.tick(1.0);
    rig.schema = cpu(2, &["other"]);
    rig.tick(2.0);
    rig.tick(3.0);
    assert_eq!(rig.log.stop().state, LogState::Idle);
    let lines = rig.lines(&part2());
    assert_eq!(lines[0], "Timestamp");
    assert_eq!(lines.len(), 3);
    assert!(lines[1..].iter().all(|line| !line.contains(',')));
}

#[test]
fn tick_without_its_schema_sends_nothing() {
    let mut rig = Rig::new();
    rig.tick(0.0);
    rig.log.start();
    let mut out = rig.next(1.0);
    out.snapshot.revision = 2;
    rig.log.on_tick(&out, &rig.schema, &rig.store.snapshot());
    rig.tick(2.0);
    rig.log.stop();
    assert_eq!(rig.values(&base()), ["2"]);
}

// --- errors and events -------------------------------------------------------------

#[test]
fn error_toasts_only_with_the_window_closed() {
    let open = fail_while_recording(true);
    assert!(open.env.toasts().is_empty());

    let closed = fail_while_recording(false);
    assert_eq!(
        closed.env.toasts(),
        [(
            t(Lang::En, "log.toast.errorTitle", &[]),
            t(Lang::En, "log.error.diskFull", &[]),
        )]
    );
    assert_eq!(closed.env.states().last(), Some(&LogState::Error));
}

#[test]
fn status_is_emitted_on_change_and_paced() {
    let mut rig = Rig::new();
    rig.set_interval(250);
    rig.tick(0.0);
    let start = rig.log.start();
    assert_eq!(rig.env.emits(), std::slice::from_ref(&start));
    for _ in 0..13 {
        rig.tick(1.0);
    }
    let stop = rig.log.stop();
    let emits = rig.env.emits();
    assert_eq!(
        emits.iter().map(|s| s.state).collect::<Vec<_>>(),
        [
            LogState::Recording,
            LogState::Recording,
            LogState::Recording,
            LogState::Recording,
            LogState::Idle
        ]
    );
    // Counters at most once a second of the tick clock.
    let recorded: Vec<u64> = emits[1..4].iter().map(|s| s.recorded_ms).collect();
    assert_eq!(recorded, [250, 1250, 2250]);
    assert_eq!(emits.last(), Some(&stop));
    assert!(emits.windows(2).all(|w| w[0].revision < w[1].revision));
}

#[test]
fn hotkeys_change_emits_once() {
    let rig = Rig::new();
    let mut hotkeys = HotkeyStatuses::default();
    hotkeys.toggle.requested = Some("Ctrl+Alt+Shift+R".into());
    rig.log.set_hotkeys(hotkeys.clone());
    rig.log.set_hotkeys(hotkeys.clone());
    assert_eq!(rig.env.emits().len(), 1);
    assert_eq!(rig.status().hotkeys, hotkeys);
}

#[test]
fn state_listeners_hear_every_change() {
    let mut rig = Rig::new();
    let heard = Arc::new(Mutex::new(Vec::new()));
    let sink = heard.clone();
    rig.log
        .on_state_change(Box::new(move |state| lock(&sink).push(state)));
    rig.tick(0.0);
    rig.log.start();
    rig.log.pause();
    rig.log.resume();
    rig.log.stop();
    assert_eq!(
        *lock(&heard),
        [
            LogState::Recording,
            LogState::Paused,
            LogState::Recording,
            LogState::Idle
        ]
    );
}

#[test]
fn old_writer_events_cannot_mutate_a_new_session() {
    let mut rig = Rig::new();
    rig.tick(0.0);
    rig.log.start();
    rig.tick(1.0);
    rig.log.stop();
    let current = rig.log.start();
    assert_eq!(current.session, 2);

    rig.log.on_writer_event(WriterEvent::Progress {
        session: 1,
        path: base(),
        part: 7,
        part_bytes: 999,
        rows: 999,
        bytes: 999,
    });
    rig.log.on_writer_event(WriterEvent::Failed {
        session: 1,
        failure: WriteFailure::DiskFull,
    });
    assert_eq!(rig.status(), current);

    // A failure of the current session sticks; its later progress only
    // updates the counters.
    rig.log.on_writer_event(WriterEvent::Failed {
        session: 2,
        failure: WriteFailure::Denied,
    });
    rig.log.on_writer_event(WriterEvent::Progress {
        session: 2,
        path: path_of(&current).unwrap(),
        part: 1,
        part_bytes: 10,
        rows: 5,
        bytes: 10,
    });
    let status = rig.status();
    assert_eq!(status.state, LogState::Error);
    assert_eq!(key_of(&status), Some("log.error.denied"));
    assert_eq!(status.rows, 5);
}

#[test]
fn late_start_ack_is_stopped() {
    let mut rig = Rig::with_timeout(Duration::from_millis(100));
    rig.tick(0.0);
    rig.gate.block_create(true);
    let status = rig.log.start();
    assert_eq!(status.state, LogState::Error);
    assert_eq!(key_of(&status), Some("log.error.closeTimeout"));
    assert_eq!(status.session, 1);

    // The writer creates the file late, answers nobody, then meets the Stop
    // the coordinator queued: header flush, then the stop flush.
    rig.gate.block_create(false);
    rig.gate.wait_flushes(2);
    wait_until("the late stop", || rig.mem.flush_count(&base()) == 2);
    assert_eq!(rig.status().state, LogState::Error, "no late reactivation");
    rig.tick(1.0);
    assert_eq!(rig.lines(&base()).len(), 1, "no rows after the timeout");

    let status = rig.log.start();
    assert_eq!(status.state, LogState::Recording);
    assert_eq!(status.session, 2);
}

#[test]
fn stop_flush_failure_never_returns_idle() {
    let mut rig = Rig::new();
    rig.tick(0.0);
    rig.log.start();
    rig.tick(1.0);
    // Flush 1 was the header's.
    rig.mem.fail_flush(2, 112);
    rig.env.clear();
    let status = rig.log.stop();
    assert_eq!(status.state, LogState::Error);
    assert_eq!(key_of(&status), Some("log.error.diskFull"));
    assert_eq!(rig.env.states(), [LogState::Error]);
    assert!(
        rig.env.toasts().is_empty(),
        "the caller shows a command's error"
    );
    assert_eq!(rig.log.start().state, LogState::Recording);
}

// --- serialization, timeouts and exit ------------------------------------------------

#[test]
fn commands_are_serialized_during_a_slow_stop() {
    let mut rig = Rig::new();
    rig.tick(0.0);
    rig.log.start();
    rig.tick(1.0);
    rig.gate.flush_delay(Duration::from_millis(200));
    let flushes = rig.gate.flushes();
    let stop = rig.command(LogService::stop);
    rig.gate.wait_flushes(flushes + 1);
    // The stop is waiting for the writer: a start now waits for it.
    let start = rig.command(LogService::start);
    let stopped = stop.join().unwrap();
    let started = start.join().unwrap();
    assert_eq!((stopped.state, stopped.session), (LogState::Idle, 1));
    assert_eq!((started.state, started.session), (LogState::Recording, 2));
}

#[test]
fn stop_timeout_moves_to_error() {
    let mut rig = Rig::with_timeout(Duration::from_millis(100));
    rig.tick(0.0);
    rig.log.start();
    rig.gate.block_flush(true);
    let begun = Instant::now();
    let status = rig.log.stop();
    assert!(begun.elapsed() < Duration::from_secs(3));
    assert_eq!(status.state, LogState::Error);
    assert_eq!(key_of(&status), Some("log.error.closeTimeout"));
    rig.gate.block_flush(false);
    assert_eq!(rig.log.start().state, LogState::Recording);
}

#[test]
fn exit_stops_the_session_within_the_bound() {
    let mut rig = Rig::new();
    rig.tick(0.0);
    rig.log.start();
    rig.tick(1.0);
    rig.tick(2.0);
    assert!(rig.log.shutdown(CLOSE_TIMEOUT));
    assert_eq!(rig.status().state, LogState::Idle);
    assert_eq!(rig.values(&base()), ["1", "2"]);
    assert_eq!(rig.mem.flushed(&base()), rig.mem.content(&base()));
    // No restart after the exit.
    assert_eq!(rig.log.start().state, LogState::Idle);
    assert_eq!(rig.mem.files(), [base()]);

    // A writer stuck in a flush: the exit gives up at the deadline.
    let mut stuck = Rig::new();
    stuck.tick(0.0);
    stuck.log.start();
    stuck.tick(1.0);
    stuck.gate.block_flush(true);
    let begun = Instant::now();
    assert!(!stuck.log.shutdown(Duration::from_millis(200)));
    assert!(begun.elapsed() < Duration::from_secs(2));
}

// --- barriers ------------------------------------------------------------------------

#[test]
fn tick_racing_stop_cannot_enqueue_after_barrier() {
    // The stop has queued its barrier and released the session lock, and
    // stays there; the writer is stuck in the stop's flush, so a row queued
    // now would stay in the queue.
    let mut rig = Rig::new();
    rig.tick(0.0);
    rig.log.start();
    rig.gate.block_flush(true);
    let flushes = rig.gate.flushes();
    let meet = hook_at(&rig.log, Point::BarrierReleased);
    let stop = rig.command(LogService::stop);
    meet.arrived.recv().unwrap();
    rig.gate.wait_flushes(flushes + 1);
    rig.tick_raw(1.0);
    assert_eq!(rig.log.queue.len(), 0, "nothing after the barrier");
    assert_eq!(
        rig.status().dropped,
        0,
        "refused by the admission, not by lock contention"
    );
    meet.go.send(()).unwrap();
    rig.gate.block_flush(false);
    assert_eq!(stop.join().unwrap().state, LogState::Idle);
    assert!(rig.values(&base()).is_empty());

    // The tick holds admission with its row queued: the stop comes after it,
    // so the row is in the file.
    let mut rig = Rig::new();
    rig.tick(0.0);
    rig.log.start();
    let meet = hook_at(&rig.log, Point::TickQueued);
    let tick = rig.tick_on_thread(1.0);
    meet.arrived.recv().unwrap();
    let stop = rig.command(LogService::stop);
    meet.go.send(()).unwrap();
    tick.join().unwrap();
    let stopped = stop.join().unwrap();
    assert_eq!(stopped.state, LogState::Idle);
    assert_eq!(rig.values(&base()), ["1"]);
    assert_eq!((stopped.rows, stopped.dropped), (1, 0));
}

#[test]
fn tick_racing_pause_is_drained_before_ack() {
    // The tick wins: its row is written and flushed before the pause answers.
    let mut rig = Rig::new();
    rig.tick(0.0);
    rig.log.start();
    rig.tick(1.0);
    let meet = hook_at(&rig.log, Point::TickQueued);
    let tick = rig.tick_on_thread(2.0);
    meet.arrived.recv().unwrap();
    let pause = rig.command(LogService::pause);
    meet.go.send(()).unwrap();
    tick.join().unwrap();
    assert_eq!(pause.join().unwrap().state, LogState::Paused);
    assert_eq!(rig.flushed_values(&base()), ["1", "2"]);

    // The pause wins: once it has queued its barrier and released the lock,
    // a tick sends nothing, now or after the resume. The writer has written
    // row 3 and waits in the pause's flush, so a row queued now would stay
    // in the queue.
    rig.log.resume();
    rig.tick(3.0);
    rig.gate.block_flush(true);
    let flushes = rig.gate.flushes();
    let meet = hook_at(&rig.log, Point::BarrierReleased);
    let pause = rig.command(LogService::pause);
    meet.arrived.recv().unwrap();
    rig.gate.wait_flushes(flushes + 1);
    rig.tick_raw(4.0);
    assert_eq!(rig.log.queue.len(), 0, "nothing after the barrier");
    assert_eq!(
        rig.status().dropped,
        0,
        "refused by the admission, not by lock contention"
    );
    meet.go.send(()).unwrap();
    rig.gate.block_flush(false);
    assert_eq!(pause.join().unwrap().state, LogState::Paused);
    assert_eq!(rig.flushed_values(&base()), ["1", "2", "3"]);
    rig.log.resume();
    rig.tick(5.0);
    rig.log.stop();
    assert_eq!(rig.values(&base()), ["1", "2", "3", "5"]);
}
