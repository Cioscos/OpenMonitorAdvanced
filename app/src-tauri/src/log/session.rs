//! The session coordinator of the CSV log: the state machine (idle →
//! recording ⇄ paused → idle, plus error), the commands from UI, tray and
//! hotkey (serialized), one row per sampler tick, the recorded time (L3) and
//! the `oma:log` status.
//!
//! Locks, in the only order they are taken:
//! - `serial`: one command at a time; held while a command waits for the
//!   writer, never by the sampler or the writer thread;
//! - `tick`: the sampler's own layout cache, never taken by anyone else;
//! - `inner`: the session state and the admission of rows, held briefly and
//!   never while waiting for the writer, emitting, toasting or calling a
//!   state listener. The sampler only `try_lock`s it: contention skips the
//!   tick (its row counted as dropped) instead of waiting;
//! - the queue's lock, taken under `inner` to queue a row or a barrier; the
//!   writer thread never holds it while it reports back, so there is no path
//!   from the queue to `inner`.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, TryLockError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oma_core::csv::{self, DisplayUnits, Layout};
use oma_core::engine::TickOutput;
use oma_core::model::Schema;
use oma_core::settings::log::MAX_LOG_SENSORS;
use oma_core::settings::{Language, Settings};
use serde::Serialize;

use super::fs::LogFs;
use super::queue::{Control, LogQueue, Row, MAX_QUEUE_BYTES, MAX_ROWS};
use super::writer::{spawn_writer, WriteFailure, WriterEvent};
use super::HotkeyStatuses;
use crate::i18n::{sensor_label, t};
use crate::settings::SettingsStore;
use crate::tray::language_for;

pub const EVENT_LOG: &str = "oma:log";
/// Longest wait for the writer's answer to a command (L6).
pub const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Counter updates are emitted at most this often, on the tick clock.
const EMIT_EVERY_MS: u64 = 1_000;
/// Dropped rows are logged at most this often, on the tick clock.
const DROP_LOG_EVERY_MS: u64 = 60_000;
/// Smallest gap between two ticks that counts as a suspend (M5b R4).
const SUSPEND_MIN_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogState {
    #[default]
    Idle,
    Recording,
    Paused,
    Error,
}

/// Why the log is in `error`: an i18n key and the `{detail}` of
/// `log.error.other`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogError {
    pub key: String,
    pub detail: Option<String>,
}

impl LogError {
    fn key(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            detail: None,
        }
    }
}

impl From<&WriteFailure> for LogError {
    fn from(failure: &WriteFailure) -> Self {
        Self {
            key: failure.key().to_owned(),
            detail: failure.detail(),
        }
    }
}

/// The `oma:log` payload. `revision` grows with every observable change and
/// never restarts, so the UI can drop stale copies.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStatus {
    pub revision: u64,
    pub state: LogState,
    pub session: u64,
    pub path: Option<String>,
    pub part: u32,
    pub part_bytes: u64,
    pub recorded_ms: u64,
    pub rows: u64,
    pub bytes: u64,
    pub dropped: u64,
    pub error: Option<LogError>,
    pub hotkeys: HotkeyStatuses,
}

/// What the coordinator needs from the app (faked in the tests).
pub trait LogEnv: Send + Sync {
    /// The Documents folder; the default log folder is inside it.
    fn default_dir(&self) -> io::Result<PathBuf>;
    /// Unix time now, for the file name.
    fn now_ms(&self) -> u64;
    fn offset_minutes(&self, unix_ms: u64) -> i32;
    fn window_open(&self) -> bool;
    fn toast(&self, title: String, body: String);
    fn emit(&self, status: &LogStatus);
}

type Listener = Arc<dyn Fn(LogState) + Send + Sync>;

pub struct LogService {
    store: Arc<SettingsStore>,
    env: Arc<dyn LogEnv>,
    close_timeout: Duration,
    queue: Arc<LogQueue>,
    writer: Mutex<Option<JoinHandle<()>>>,
    serial: Serial,
    inner: Mutex<Inner>,
    /// The open admission (`inner.admission`), 0 for none: the sampler's
    /// check without a lock, so an idle tick costs almost nothing.
    /// `inner.admitting` is the authority.
    admitting: AtomicU64,
    shut: AtomicBool,
    tick: Mutex<TickState>,
    listeners: Mutex<Vec<Listener>>,
    #[cfg(test)]
    hook: Mutex<Option<Hook>>,
}

impl LogService {
    /// Starts the writer thread; `close_timeout` bounds every wait for it
    /// ([`CLOSE_TIMEOUT`] in the app).
    pub fn new(
        store: Arc<SettingsStore>,
        fs: Arc<dyn LogFs>,
        env: Arc<dyn LogEnv>,
        close_timeout: Duration,
    ) -> Arc<Self> {
        let queue = LogQueue::new(MAX_ROWS, MAX_QUEUE_BYTES);
        Arc::new_cyclic(|weak: &std::sync::Weak<Self>| {
            let events = weak.clone();
            let writer = spawn_writer(
                queue.clone(),
                fs,
                Box::new(move |event| {
                    if let Some(log) = events.upgrade() {
                        log.on_writer_event(event);
                    }
                }),
            );
            Self {
                store,
                env,
                close_timeout,
                queue,
                writer: Mutex::new(Some(writer)),
                serial: Serial::default(),
                inner: Mutex::new(Inner::default()),
                admitting: AtomicU64::new(0),
                shut: AtomicBool::new(false),
                tick: Mutex::new(TickState::default()),
                listeners: Mutex::new(Vec::new()),
                #[cfg(test)]
                hook: Mutex::new(None),
            }
        })
    }

    /// `listener` hears every state change, on the thread that made it (a
    /// command's, the sampler's or the writer's); it must not block.
    pub fn on_state_change(&self, listener: Box<dyn Fn(LogState) + Send + Sync>) {
        lock(&self.listeners).push(Arc::from(listener));
    }

    /// The state of the global hotkeys, from the hotkey manager.
    pub fn set_hotkeys(&self, hotkeys: HotkeyStatuses) {
        let mut fx = Effects::default();
        {
            let mut inner = lock(&self.inner);
            if inner.hotkeys == hotkeys {
                return;
            }
            inner.hotkeys = hotkeys;
            inner.touch();
            inner.publish(&mut fx);
        }
        self.run(fx);
    }

    pub fn status(&self) -> LogStatus {
        lock(&self.inner).status()
    }

    /// The folder of the session in progress (recording or paused),
    /// otherwise the configured one.
    pub fn folder(&self) -> io::Result<PathBuf> {
        {
            let inner = lock(&self.inner);
            if inner.active() {
                if let Some(dir) = &inner.session_dir {
                    return Ok(dir.clone());
                }
            }
        }
        self.configured_dir(&self.store.snapshot())
    }

    /// From `idle` or `error`: a new session in a new file.
    pub fn start(&self) -> LogStatus {
        let _serial = self.serial.enter();
        let schema = {
            let inner = lock(&self.inner);
            if self.is_shut() || inner.active() {
                return inner.status();
            }
            inner.schema.clone()
        };
        let settings = self.store.snapshot();
        let plan = match schema {
            Some(schema) => self.plan(&schema, &settings),
            None => Err(LogError::key("log.error.schemaUnavailable")),
        };
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => return self.refuse(error),
        };
        let (reply, answer) = sync_channel(1);
        {
            let mut inner = lock(&self.inner);
            inner.session += 1;
            inner.path = None;
            inner.part = 0;
            inner.part_bytes = 0;
            inner.rows = 0;
            inner.bytes = 0;
            inner.dropped = 0;
            inner.recorded_ms = 0;
            inner.session_dir = Some(plan.dir.clone());
            inner.touch();
            let start = Control::Start {
                session: inner.session,
                layout: Arc::new(plan.layout),
                dir: plan.dir,
                stem: plan.stem,
                limit_bytes: plan.limit_bytes,
                reply,
            };
            if self.queue.push_control(start).is_err() {
                return inner.status();
            }
            inner.busy = true;
        }
        let outcome = wait(&answer, self.close_timeout);
        self.finish(outcome, true, |log, inner, fx| {
            inner.error = None;
            log.open_admission(inner, settings.log.every_ticks);
            inner.set_state(LogState::Recording, fx);
        })
    }

    /// From `recording`: rows stop, the writer flushes what it has.
    pub fn pause(&self) -> LogStatus {
        let _serial = self.serial.enter();
        let Some(answer) = self.barrier(&[LogState::Recording], |session, reply| Control::Pause {
            session,
            reply,
        }) else {
            return self.status();
        };
        let outcome = wait(&answer, self.close_timeout);
        self.finish(outcome, true, |_, inner, fx| {
            inner.clock.reset();
            inner.set_state(LogState::Paused, fx);
        })
    }

    /// From `paused`: the next tick is recorded at once.
    pub fn resume(&self) -> LogStatus {
        let _serial = self.serial.enter();
        let answer = {
            let mut inner = lock(&self.inner);
            if self.is_shut() || inner.state != LogState::Paused {
                return inner.status();
            }
            let (reply, answer) = sync_channel(1);
            let resume = Control::Resume {
                session: inner.session,
                reply,
            };
            if self.queue.push_control(resume).is_err() {
                return inner.status();
            }
            inner.busy = true;
            answer
        };
        let every_ticks = self.store.snapshot().log.every_ticks;
        let outcome = wait(&answer, self.close_timeout);
        self.finish(outcome, true, |log, inner, fx| {
            log.open_admission(inner, every_ticks);
            inner.set_state(LogState::Recording, fx);
        })
    }

    /// From `recording` or `paused`: `idle` only once the writer confirms
    /// that the accepted rows are written and the file is closed.
    pub fn stop(&self) -> LogStatus {
        let _serial = self.serial.enter();
        let Some(answer) = self.barrier(
            &[LogState::Recording, LogState::Paused],
            |session, reply| Control::Stop { session, reply },
        ) else {
            return self.status();
        };
        let outcome = wait(&answer, self.close_timeout);
        self.stopped(outcome)
    }

    /// Sampler thread: one row per recorded tick. Never waits on the writer
    /// or on a command; `schema` is the current schema (`alerts.schema()`).
    pub fn on_tick(&self, out: &TickOutput, schema: &Schema, settings: &Settings) {
        // Without the tick's own schema the values cannot be placed.
        if schema.revision != out.snapshot.revision {
            return;
        }
        let mut tick = lock(&self.tick);
        let fresh = (tick.revision != Some(schema.revision)).then(|| {
            tick.revision = Some(schema.revision);
            Arc::new(schema.clone())
        });
        let admitting = self.admitting.load(Ordering::Acquire);
        if fresh.is_none() && admitting == 0 && tick.missed.is_none() {
            return;
        }
        // Built outside the session lock; the cache rebuilds only on a new key.
        let prepared = (admitting != 0).then(|| {
            (
                tick.layout(schema, settings),
                self.env.offset_minutes(out.snapshot.timestamp_ms),
            )
        });
        let mut guard = match self.inner.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => {
                if admitting != 0 {
                    tick.missed = Some(match tick.missed {
                        Some((admission, count)) if admission == admitting => {
                            (admission, count + 1)
                        }
                        _ => (admitting, 1),
                    });
                }
                if fresh.is_some() {
                    // Handed over on the next tick.
                    tick.revision = None;
                }
                return;
            }
        };
        let inner = &mut *guard;
        let mut fx = Effects::default();
        let now = out.monotonic_ms;
        if let Some(schema) = fresh {
            inner.schema = Some(schema);
        }
        if let Some((admission, count)) = tick.missed.take() {
            // Only while the admission they met is still open: once a barrier
            // has closed it their rows would have been refused anyway.
            if inner.admitting && admission == inner.admission {
                inner.miss_ticks(count, now);
            }
        }
        if let (true, Some(((layout, overflow), offset))) = (inner.admitting, prepared) {
            let added = inner
                .clock
                .advance(now, max_gap_ms(settings.general.interval_ms));
            if added > 0 {
                inner.recorded_ms += added;
                inner.touch();
            }
            if overflow {
                // Never the truncated layout (L12).
                let error = LogError::key("log.error.tooManyColumns");
                tracing::warn!(
                    session = inner.session,
                    "the log layout grew past the column limit"
                );
                fx.toast = Some(error.clone());
                self.fail(inner, error, true, &mut fx);
                inner.publish(&mut fx);
            } else {
                let every = settings.log.every_ticks.max(1);
                if every != inner.every_ticks {
                    inner.every_ticks = every;
                    inner.phase = 0;
                }
                if inner.phase == 0 {
                    self.push_row(inner, &layout, out, offset);
                }
                inner.phase = (inner.phase + 1) % every;
                if inner.dirty
                    && inner
                        .last_emit_ms
                        .is_none_or(|last| now.saturating_sub(last) >= EMIT_EVERY_MS)
                {
                    inner.last_emit_ms = Some(now);
                    inner.publish(&mut fx);
                }
            }
        }
        drop(guard);
        drop(tick);
        self.run(fx);
    }

    /// App exit: closes the admission at once and stops the session, all
    /// within `timeout` (serialization, stop and the writer's exit). `false`
    /// when something was still pending at the deadline. No start after it.
    pub fn shutdown(&self, timeout: Duration) -> bool {
        let deadline = Instant::now().checked_add(timeout);
        let left = || deadline.map_or(timeout, |d| d.saturating_duration_since(Instant::now()));
        self.shut.store(true, Ordering::SeqCst);
        self.close_admission(&mut lock(&self.inner));
        let mut complete = true;
        match self.serial.enter_until(deadline) {
            Some(_serial) => {
                let answer = self.barrier(
                    &[LogState::Recording, LogState::Paused],
                    |session, reply| Control::Stop { session, reply },
                );
                if let Some(answer) = answer {
                    let outcome = wait(&answer, left());
                    complete &= !matches!(outcome, Outcome::TimedOut);
                    self.stopped(outcome);
                }
            }
            None => {
                tracing::warn!("a log command was still waiting for the writer at exit");
                complete = false;
            }
        }
        self.queue.close();
        let writer = lock(&self.writer).take();
        if let Some(writer) = writer {
            loop {
                if writer.is_finished() {
                    let _ = writer.join();
                    break;
                }
                if left().is_zero() {
                    complete = false;
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        if !complete {
            tracing::error!(
                "the CSV log did not close within {timeout:?}: the last rows may be missing"
            );
        }
        complete
    }

    // --- commands --------------------------------------------------------

    /// In one of `states`: closes the admission and queues `make`'s control
    /// as a barrier under the same lock, so no row of the session can follow
    /// it; the reply receiver, or `None` when the command does not apply.
    fn barrier(
        &self,
        states: &[LogState],
        make: impl FnOnce(u64, super::queue::Reply) -> Control,
    ) -> Option<Receiver<Result<(), WriteFailure>>> {
        let mut inner = lock(&self.inner);
        // The exit's own stop comes through here after `shut` is set.
        if !states.contains(&inner.state) {
            return None;
        }
        self.close_admission(&mut inner);
        let (reply, answer) = sync_channel(1);
        self.queue.push_control(make(inner.session, reply)).ok()?;
        inner.busy = true;
        drop(inner);
        // Admission is closed and stays closed while the command waits.
        self.hook(Point::BarrierReleased);
        Some(answer)
    }

    /// Settles a stop: `idle` only on a confirmed, clean close.
    fn stopped(&self, outcome: Outcome) -> LogStatus {
        self.finish(outcome, false, |_, inner, fx| {
            inner.clock.reset();
            inner.set_state(LogState::Idle, fx);
        })
    }

    /// Settles a command that waited for the writer: `success` on a clean
    /// answer; otherwise `error` with the writer's reason (or
    /// `log.error.closeTimeout`), releasing the writer's session when
    /// `release` (the stop has already released it).
    fn finish(
        &self,
        outcome: Outcome,
        release: bool,
        success: impl FnOnce(&Self, &mut Inner, &mut Effects),
    ) -> LogStatus {
        let mut fx = Effects::default();
        let status = {
            let mut guard = lock(&self.inner);
            let inner = &mut *guard;
            inner.busy = false;
            let held = inner.held.take();
            let failure = match outcome {
                Outcome::Done => held.map(|failure| LogError::from(&failure)),
                Outcome::Failed(failure) => Some(LogError::from(&failure)),
                Outcome::TimedOut => {
                    tracing::error!(
                        session = inner.session,
                        "the log writer did not answer within {:?}",
                        self.close_timeout
                    );
                    Some(LogError::key("log.error.closeTimeout"))
                }
            };
            match failure {
                None => success(self, inner, &mut fx),
                Some(error) => {
                    tracing::warn!(session = inner.session, key = %error.key, "log command failed");
                    self.fail(inner, error, release, &mut fx);
                }
            }
            inner.publish(&mut fx);
            fx.status.clone().unwrap_or_else(|| inner.status())
        };
        self.run(fx);
        status
    }

    /// A start refused before any file: `error` with `error`, without the
    /// file and counters of the previous session.
    fn refuse(&self, error: LogError) -> LogStatus {
        let mut fx = Effects::default();
        let status = {
            let mut inner = lock(&self.inner);
            inner.path = None;
            inner.part = 0;
            inner.part_bytes = 0;
            inner.rows = 0;
            inner.bytes = 0;
            inner.recorded_ms = 0;
            inner.dropped = 0;
            inner.error = Some(error);
            inner.set_state(LogState::Error, &mut fx);
            inner.publish(&mut fx);
            inner.status()
        };
        self.run(fx);
        status
    }

    /// Layout, folder and file name of a new session.
    fn plan(&self, schema: &Schema, settings: &Settings) -> Result<Plan, LogError> {
        let (layout, overflow) = build_layout(schema, settings);
        if overflow {
            return Err(LogError::key("log.error.tooManyColumns"));
        }
        if layout.columns.is_empty() {
            return Err(LogError::key("log.error.noColumns"));
        }
        let dir = self
            .configured_dir(settings)
            .map_err(|err| LogError::from(&WriteFailure::from_io(&err)))?;
        let now = self.env.now_ms();
        let name = csv::file_name(csv::local_time(now, self.env.offset_minutes(now)), None, 1);
        let stem = name.strip_suffix(".csv").unwrap_or(&name).to_owned();
        Ok(Plan {
            layout,
            dir,
            stem,
            limit_bytes: u64::from(settings.log.max_file_mb) * 1024 * 1024,
        })
    }

    pub(crate) fn configured_dir(&self, settings: &Settings) -> io::Result<PathBuf> {
        match &settings.log.folder {
            Some(folder) => Ok(PathBuf::from(folder)),
            None => Ok(self
                .env
                .default_dir()?
                .join("OpenMonitor Advanced")
                .join("logs")),
        }
    }

    fn is_shut(&self) -> bool {
        self.shut.load(Ordering::SeqCst)
    }

    // --- admission and failures (called with `inner` locked) ---------------

    fn open_admission(&self, inner: &mut Inner, every_ticks: u32) {
        inner.clock.reset();
        inner.phase = 0;
        inner.every_ticks = every_ticks.max(1);
        if self.is_shut() {
            return;
        }
        inner.admitting = true;
        inner.admission += 1;
        self.admitting.store(inner.admission, Ordering::Release);
    }

    fn close_admission(&self, inner: &mut Inner) {
        inner.admitting = false;
        self.admitting.store(0, Ordering::Release);
    }

    /// The session goes to `error`; with `release`, a `Stop` (nobody waits
    /// for its answer) frees what the writer holds for it, ahead of any
    /// later `Start`.
    fn fail(&self, inner: &mut Inner, error: LogError, release: bool, fx: &mut Effects) {
        self.close_admission(inner);
        if release {
            let (reply, _) = sync_channel(1);
            let _ = self.queue.push_control(Control::Stop {
                session: inner.session,
                reply,
            });
        }
        inner.clock.reset();
        inner.error = Some(error);
        inner.set_state(LogState::Error, fx);
    }

    fn push_row(&self, inner: &mut Inner, layout: &Arc<Layout>, out: &TickOutput, offset: i32) {
        let row = Row {
            session: inner.session,
            layout: layout.clone(),
            timestamp_ms: out.snapshot.timestamp_ms,
            offset_minutes: offset,
            values: layout.extract(&out.snapshot.values, &out.quality),
        };
        match self.queue.try_push_row(row) {
            Ok(()) => self.hook(Point::TickQueued),
            Err(_) => inner.note_drops(1, out.monotonic_ms),
        }
    }

    // --- writer events -------------------------------------------------------

    /// Writer thread. Events of another session are ignored; a failure never
    /// leaves `error`, and while a command waits it is kept for that command.
    fn on_writer_event(&self, event: WriterEvent) {
        let mut fx = Effects::default();
        {
            let mut guard = lock(&self.inner);
            let inner = &mut *guard;
            match event {
                WriterEvent::Progress {
                    session,
                    path,
                    part,
                    part_bytes,
                    rows,
                    bytes,
                } if session == inner.session => {
                    let path = Some(path);
                    if inner.path != path
                        || (inner.part, inner.part_bytes, inner.rows, inner.bytes)
                            != (part, part_bytes, rows, bytes)
                    {
                        inner.path = path;
                        inner.part = part;
                        inner.part_bytes = part_bytes;
                        inner.rows = rows;
                        inner.bytes = bytes;
                        inner.touch();
                    }
                }
                WriterEvent::Failed { session, failure } if session == inner.session => {
                    if inner.busy {
                        inner.held.get_or_insert(failure);
                    } else if inner.active() {
                        tracing::warn!(session, ?failure, "the log session failed");
                        let error = LogError::from(&failure);
                        fx.toast = Some(error.clone());
                        self.fail(inner, error, true, &mut fx);
                        inner.publish(&mut fx);
                    }
                }
                _ => {}
            }
        }
        self.run(fx);
    }

    /// Emits, tells the listeners and toasts, with no lock of ours held.
    fn run(&self, fx: Effects) {
        if let Some(status) = &fx.status {
            self.env.emit(status);
        }
        if let Some(state) = fx.state {
            let listeners = lock(&self.listeners).clone();
            for listener in listeners {
                listener(state);
            }
        }
        if let Some(error) = fx.toast {
            if !self.env.window_open() {
                let lang = language_for(self.store.snapshot().general.language);
                let detail = error.detail.as_deref().unwrap_or("");
                self.env.toast(
                    t(lang, "log.toast.errorTitle", &[]),
                    t(lang, &error.key, &[("detail", detail)]),
                );
            }
        }
    }

    #[cfg(test)]
    fn hook(&self, point: Point) {
        let hook = lock(&self.hook).clone();
        if let Some(hook) = hook {
            hook(point);
        }
    }

    #[cfg(not(test))]
    fn hook(&self, _point: Point) {}

    /// Runs `hook` at the test points.
    #[cfg(test)]
    fn set_hook(&self, hook: Hook) {
        *lock(&self.hook) = Some(hook);
    }
}

impl Drop for LogService {
    fn drop(&mut self) {
        // The writer thread holds the queue: let it leave.
        self.queue.close();
    }
}

/// Points where the tests stop a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Point {
    /// A tick queued its row (session lock held).
    TickQueued,
    /// A pause or stop queued its barrier and released the session lock;
    /// it has not started waiting for the writer yet.
    BarrierReleased,
}

#[cfg(test)]
type Hook = Arc<dyn Fn(Point) + Send + Sync>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a command needs to start a session.
struct Plan {
    layout: Layout,
    dir: PathBuf,
    stem: String,
    limit_bytes: u64,
}

enum Outcome {
    Done,
    Failed(WriteFailure),
    TimedOut,
}

fn wait(answer: &Receiver<Result<(), WriteFailure>>, timeout: Duration) -> Outcome {
    match answer.recv_timeout(timeout) {
        Ok(Ok(())) => Outcome::Done,
        Ok(Err(failure)) => Outcome::Failed(failure),
        // A writer that is gone answers nothing, like one that is stuck.
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => Outcome::TimedOut,
    }
}

/// L3: a gap longer than this between two ticks is a suspend or a stall.
fn max_gap_ms(interval_ms: u32) -> u64 {
    (3 * u64::from(interval_ms)).max(SUSPEND_MIN_MS)
}

fn display_units(settings: &Settings) -> DisplayUnits {
    DisplayUnits {
        temperature: settings.general.temperature_unit,
        throughput: settings.general.throughput_unit,
    }
}

/// The columns of `schema` for the log settings, labelled in the app's
/// language; `true` on overflow.
fn build_layout(schema: &Schema, settings: &Settings) -> (Layout, bool) {
    let lang = language_for(settings.general.language);
    Layout::build(
        schema,
        settings.log.sensors.as_deref(),
        display_units(settings),
        &|sensor| sensor_label(lang, &sensor.label),
        MAX_LOG_SENSORS,
    )
}

/// Recorded time on the tick clock (L3): the time between two ticks counts
/// unless it is longer than the suspend bound; a reset re-anchors at the
/// next tick without counting the gap before it.
#[derive(Debug, Default)]
struct RecordedClock {
    anchor: Option<u64>,
}

impl RecordedClock {
    /// The milliseconds `now_ms` adds.
    fn advance(&mut self, now_ms: u64, max_gap_ms: u64) -> u64 {
        let added = self.anchor.map_or(0, |anchor| {
            let gap = now_ms.saturating_sub(anchor);
            if gap <= max_gap_ms {
                gap
            } else {
                0
            }
        });
        self.anchor = Some(now_ms);
        added
    }

    fn reset(&mut self) {
        self.anchor = None;
    }
}

/// One command at a time; unlike a `Mutex<()>`, it can be entered with a
/// deadline (the exit).
#[derive(Default)]
struct Serial {
    busy: Mutex<bool>,
    freed: Condvar,
}

struct SerialGuard<'a>(&'a Serial);

impl Serial {
    fn enter(&self) -> SerialGuard<'_> {
        let mut busy = lock(&self.busy);
        while *busy {
            busy = self
                .freed
                .wait(busy)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *busy = true;
        SerialGuard(self)
    }

    /// `None` when still taken at `deadline` (`None`: no deadline).
    fn enter_until(&self, deadline: Option<Instant>) -> Option<SerialGuard<'_>> {
        let Some(deadline) = deadline else {
            return Some(self.enter());
        };
        let mut busy = lock(&self.busy);
        while *busy {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            busy = self
                .freed
                .wait_timeout(busy, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        *busy = true;
        Some(SerialGuard(self))
    }
}

impl Drop for SerialGuard<'_> {
    fn drop(&mut self) {
        *lock(&self.0.busy) = false;
        self.0.freed.notify_all();
    }
}

/// What the effects of a change are, run after the lock is released.
#[derive(Default)]
struct Effects {
    status: Option<LogStatus>,
    state: Option<LogState>,
    /// Toasted when the window is closed.
    toast: Option<LogError>,
}

/// The session state; see the module comment for its lock.
#[derive(Default)]
struct Inner {
    revision: u64,
    state: LogState,
    session: u64,
    path: Option<PathBuf>,
    part: u32,
    part_bytes: u64,
    recorded_ms: u64,
    rows: u64,
    bytes: u64,
    dropped: u64,
    error: Option<LogError>,
    hotkeys: HotkeyStatuses,
    /// The latest schema whose snapshot came with it, kept in `idle` too.
    schema: Option<Arc<Schema>>,
    /// Folder of the current (or last) session.
    session_dir: Option<PathBuf>,
    /// Rows of `session` are accepted.
    admitting: bool,
    /// Grows with every opened admission (never 0 once opened), so ticks
    /// that met a busy lock are counted only against the admission they met.
    admission: u64,
    /// A command waits for the writer's answer about `session`.
    busy: bool,
    /// A failure reported while a command waited, for that command.
    held: Option<WriteFailure>,
    clock: RecordedClock,
    /// Ticks since the last recorded one, modulo `every_ticks`.
    phase: u32,
    every_ticks: u32,
    /// Changed since the last emit.
    dirty: bool,
    /// Tick time of the last paced emit.
    last_emit_ms: Option<u64>,
    last_drop_log_ms: Option<u64>,
}

impl Inner {
    fn active(&self) -> bool {
        matches!(self.state, LogState::Recording | LogState::Paused)
    }

    fn touch(&mut self) {
        self.revision += 1;
        self.dirty = true;
    }

    fn set_state(&mut self, state: LogState, fx: &mut Effects) {
        if self.state != state {
            self.state = state;
            fx.state = Some(state);
        }
        self.touch();
    }

    /// Queues the emit of the current status.
    fn publish(&mut self, fx: &mut Effects) {
        self.dirty = false;
        fx.status = Some(self.status());
    }

    /// `count` ticks that met a busy lock: each takes its turn in the
    /// every-N phase, and only those whose turn records a row are dropped.
    fn miss_ticks(&mut self, count: u64, now_ms: u64) {
        let every = u64::from(self.every_ticks.max(1));
        let phase = u64::from(self.phase);
        // Recording turns are the multiples of `every` in [phase, phase + count).
        let dropped = (phase + count).div_ceil(every) - phase.div_ceil(every);
        self.phase = ((phase + count) % every) as u32;
        if dropped > 0 {
            self.note_drops(dropped, now_ms);
        }
    }

    /// Counts rows the queue refused (or a busy lock dropped); logged at
    /// most once a minute.
    fn note_drops(&mut self, count: u64, now_ms: u64) {
        self.dropped += count;
        self.touch();
        if self
            .last_drop_log_ms
            .is_none_or(|last| now_ms.saturating_sub(last) >= DROP_LOG_EVERY_MS)
        {
            self.last_drop_log_ms = Some(now_ms);
            tracing::warn!(
                session = self.session,
                dropped = self.dropped,
                "log rows dropped: the writer is behind"
            );
        }
    }

    fn status(&self) -> LogStatus {
        LogStatus {
            revision: self.revision,
            state: self.state,
            session: self.session,
            path: self.path.as_ref().map(|p| p.display().to_string()),
            part: self.part,
            part_bytes: self.part_bytes,
            recorded_ms: self.recorded_ms,
            rows: self.rows,
            bytes: self.bytes,
            dropped: self.dropped,
            error: self.error.clone(),
            hotkeys: self.hotkeys.clone(),
        }
    }
}

/// The sampler's layout cache: rebuilt only when the key changes (schema
/// revision, selection, language, units).
#[derive(Default)]
struct TickState {
    /// Revision of the schema handed to `Inner`.
    revision: Option<u64>,
    key: Option<LayoutKey>,
    layout: Option<(Arc<Layout>, bool)>,
    /// Ticks that met a busy session lock: `(admission, count)`, settled by
    /// the next tick that gets the lock.
    missed: Option<(u64, u64)>,
}

struct LayoutKey {
    revision: u64,
    sensors: Option<Vec<String>>,
    language: Language,
    units: DisplayUnits,
}

impl TickState {
    fn layout(&mut self, schema: &Schema, settings: &Settings) -> (Arc<Layout>, bool) {
        let units = display_units(settings);
        let current = self.key.as_ref().is_some_and(|key| {
            key.revision == schema.revision
                && key.language == settings.general.language
                && key.units == units
                && key.sensors.as_deref() == settings.log.sensors.as_deref()
        });
        match &self.layout {
            Some(layout) if current => layout.clone(),
            _ => {
                let (layout, overflow) = build_layout(schema, settings);
                let built = (Arc::new(layout), overflow);
                self.layout = Some(built.clone());
                self.key = Some(LayoutKey {
                    revision: schema.revision,
                    sensors: settings.log.sensors.clone(),
                    language: settings.general.language,
                    units,
                });
                built
            }
        }
    }
}

#[cfg(test)]
mod tests;
