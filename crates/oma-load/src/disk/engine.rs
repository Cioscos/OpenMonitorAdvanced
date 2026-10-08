//! The disk phase engine (plan DC3): opens the test file of a disk plan, then for each phase
//! starts `threads` workers, each with its own [`IoQueue`] (an IOCP on Windows) and `queue`
//! I/Os in flight. A phase ends at `duration_s`, at `write_cap_bytes` (a write that would
//! pass it never starts), at a stop or at the first persistent error; the workers then stop
//! submitting and wait for the I/Os in flight. Once a second the engine sends `Progress`
//! from the workers' atomic counters; `PhaseDone.disk` carries the totals and latencies.
//!
//! What a worker submits comes from a [`WorkerLoad`]: `disk_fill` and `disk_bench` here,
//! the stress loads (C5, C6) next to them in [`worker_load`].
//!
//! Errors (DC3): a full disk sends `Notice disk_full` and ends the run `Failed` with exit
//! code 0; any other I/O error is retried once on the same slot: a good retry sends
//! `Error io_error` with `transient: Some(true)` and the phase goes on, a bad one sends it
//! with `Some(false)` and ends the run `Failed` with [`EXIT_IO`].

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use oma_core::disk_block::{write_block, BLOCK_BYTES};
use oma_ipc::load::{
    ComputeError, DiskJob, DiskPhaseStats, DiskProgress, DiskTarget, ErrorKind, FinishReason,
    Finished, IoStats, Isa, KernelId, LoadMessage, Notice, Phase, PhaseDone, Plan, Progress,
};

use super::file::{classify_win32, fits, DiskError};
use super::hist::Histogram;
use super::offsets::{IoPicker, IoReq};
use crate::args::Inject;
use crate::link::{EXIT_IO, EXIT_OK};
use crate::rng::{phase_seed, Xoshiro256ss};

/// One completed I/O: the slot it used, the bytes moved or the Win32 error, and the time
/// from its submission to its completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoDone {
    pub slot: usize,
    pub result: Result<u32, u32>,
    pub latency_us: u64,
}

/// The overlapped I/O of one worker: `slots()` buffers, each used by at most one I/O.
///
/// The engine calls `buffer(slot)` only for a slot with no I/O in flight and drops the
/// borrow before `submit`; `submit` failing means the I/O did not start (no completion
/// will come for it). Dropping the queue cancels and waits for whatever is still in flight.
pub trait IoQueue: Send {
    fn submit(&mut self, req: IoReq, buf: usize) -> Result<(), DiskError>;
    /// Appends the I/Os completed within `timeout_ms` to `out` (none on a timeout).
    fn wait(&mut self, timeout_ms: u32, out: &mut Vec<IoDone>) -> Result<(), DiskError>;
    fn slots(&self) -> usize;
    fn buffer(&mut self, slot: usize) -> &mut [u8];
}

/// A test data file, as the engine sees it.
pub trait DataFileApi {
    fn bytes(&self) -> u64;
    /// The physical sector, in bytes (a power of 2, at least 512).
    fn sector(&self) -> u32;
    fn flush(&self) -> Result<(), DiskError>;
    /// For the queue hook, which needs the concrete file (its handle).
    fn as_any(&self) -> &dyn Any;
}

/// What the engine runs on: [`run_disk`] gives the real files and IOCP queues, the tests
/// fakes.
pub struct DiskHooks<'h> {
    /// Creates a data file: the suffix (`None`, `-<n>` for V3, `-sync` for V4), its size
    /// and write-through.
    #[allow(clippy::type_complexity)]
    pub open:
        &'h dyn Fn(&DiskTarget, Option<&str>, u64, bool) -> Result<Box<dyn DataFileApi>, DiskError>,
    /// The queue of one worker of a phase on `file`.
    #[allow(clippy::type_complexity)]
    pub queue: &'h dyn Fn(&dyn DataFileApi, &DiskJob) -> Result<Box<dyn IoQueue>, DiskError>,
    /// The free bytes of the target volume, `None` when unknown.
    pub free_bytes: &'h dyn Fn() -> Option<u64>,
    pub clock: &'h (dyn Fn() -> Instant + Sync),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskRunEnd {
    pub finished: Finished,
    /// [`EXIT_IO`] after a persistent I/O error: the process ends with it.
    pub exit_code: i32,
}

/// The block sizes a job really uses: the random one if any I/O is random, the
/// sequential one if any is sequential. Gives `(smallest, largest)`.
pub fn used_blocks(job: &DiskJob) -> (u32, u32) {
    match job.random_percent {
        0 => (job.seq_block_bytes, job.seq_block_bytes),
        100.. => (job.block_bytes, job.block_bytes),
        _ => (
            job.block_bytes.min(job.seq_block_bytes),
            job.block_bytes.max(job.seq_block_bytes),
        ),
    }
}

/// Runs the disk `plan` on real files and IOCP queues; the caller sends the `Finished` and
/// exits with `exit_code` when it is not `EXIT_OK`. The test files are deleted when they
/// close (`FILE_FLAG_DELETE_ON_CLOSE`), before this returns.
#[cfg(windows)]
pub fn run_disk(
    plan: &Plan,
    out: &(dyn Fn(LoadMessage) + Sync),
    stop: &AtomicBool,
    inject: Option<Inject>,
) -> DiskRunEnd {
    use super::file::{free_space, TestFiles};
    use super::iocp::IocpQueue;
    use std::cell::RefCell;
    use std::path::Path;

    // Outlives every data file (they are dropped inside `run_disk_with`): the sidecar stays
    // until the run ends.
    let files: RefCell<Option<TestFiles>> = RefCell::new(None);
    let open = |t: &DiskTarget, suffix: Option<&str>, bytes: u64, write_through: bool| {
        let mut files = files.borrow_mut();
        if files.is_none() {
            *files = Some(TestFiles::create(
                Path::new(&t.dir),
                plan.seed,
                t.reserve_bytes,
            )?);
        }
        let tf = files.as_ref().expect("created above");
        let file = tf.open_data(suffix, bytes, write_through)?;
        Ok(Box::new(file) as Box<dyn DataFileApi>)
    };
    let queue = |file: &dyn DataFileApi, job: &DiskJob| {
        IocpQueue::new(file, job).map(|q| Box::new(q) as Box<dyn IoQueue>)
    };
    let dir = plan
        .disk
        .as_ref()
        .map(|d| d.dir.clone())
        .unwrap_or_default();
    let free = || free_space(Path::new(&dir)).ok();
    let hooks = DiskHooks {
        open: &open,
        queue: &queue,
        free_bytes: &free,
        clock: &Instant::now,
    };
    run_disk_with(plan, out, stop, inject, &hooks)
}

/// [`run_disk`] on the given files, queues and clock.
pub fn run_disk_with(
    plan: &Plan,
    out: &(dyn Fn(LoadMessage) + Sync),
    stop: &AtomicBool,
    inject: Option<Inject>,
    hooks: &DiskHooks<'_>,
) -> DiskRunEnd {
    let now = (hooks.clock)();
    let mut run = Run {
        plan,
        out,
        stop,
        _inject: inject,
        hooks,
        start: now,
        phase: 0,
        checks: 0,
        errors: 0,
        read_total: 0,
        written_total: 0,
    };
    // `validate` refuses disk kernels in a plan without a disk.
    let Some(target) = plan.disk.as_ref() else {
        tracing::error!("a disk run without a disk target");
        return run.end(FinishReason::Failed);
    };
    if (hooks.free_bytes)().is_some_and(|free| !fits(free, target.file_bytes, target.reserve_bytes))
    {
        return run.fatal(Fatal::Full);
    }
    // Creating the file can wait for a disk to spin up.
    let opened = run.heartbeat(0, || (hooks.open)(target, None, target.file_bytes, false));
    let file = match opened {
        Ok(file) => file,
        Err(e) => return run.fatal(Fatal::from_open(e)),
    };
    run.notice("file_bytes", Some(file.bytes()));
    for (index, spec) in plan.phases.iter().enumerate() {
        if stop.load(Ordering::Relaxed) {
            return run.end(FinishReason::Stopped);
        }
        run.phase = index as u32;
        match run.run_phase(&*file, spec) {
            Ok(End::Stopped) => return run.end(FinishReason::Stopped),
            Ok(End::Done | End::Skipped) => {}
            Err(fatal) => return run.fatal(fatal),
        }
    }
    run.end(FinishReason::Completed)
}

/// The `Progress` interval of a running phase.
const PROGRESS_EVERY: Duration = Duration::from_secs(1);
/// The `Progress` interval while nothing is measured (file creation, pause): the app takes
/// 5 s of silence for a hung process.
const HEARTBEAT: Duration = Duration::from_millis(900);
/// The engine thread's sleep between its looks at the workers.
const SLICE: Duration = Duration::from_millis(2);
/// The completion wait of a worker (DC3).
const WAIT_MS: u32 = 100;
/// The longest real time a worker waits for its I/Os in flight after the phase ended;
/// past it the queue's drop cancels them.
const DRAIN_LIMIT: Duration = Duration::from_secs(10);
/// `Error` messages per phase; the errors after them are only counted.
const ERRORS_PER_PHASE: u32 = 16;
/// `ERROR_HANDLE_EOF`: an I/O that moved fewer bytes than asked.
const ERROR_HANDLE_EOF: u32 = 38;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Done,
    Stopped,
    Skipped,
}

/// What ends a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fatal {
    /// The volume is full (or would pass the reserve).
    Full,
    /// The folder refuses our files.
    Denied,
    /// A persistent I/O error at this 4 KiB block, with its Win32 code.
    Io { iteration: u64, code: u32 },
}

impl Fatal {
    fn from_open(e: DiskError) -> Fatal {
        match e {
            DiskError::Full => Fatal::Full,
            DiskError::AccessDenied => Fatal::Denied,
            DiskError::Io(code) => Fatal::Io { iteration: 0, code },
        }
    }
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

/// The Win32 code of an error, for the retry and the report.
fn code_of(e: DiskError) -> u32 {
    match e {
        DiskError::Full => 112,
        DiskError::AccessDenied => 5,
        DiskError::Io(code) => code,
    }
}

fn io_error(
    phase: u32,
    kernel: KernelId,
    seed: u64,
    iteration: u64,
    code: u32,
    transient: bool,
) -> LoadMessage {
    LoadMessage::Error(ComputeError {
        phase,
        kernel,
        isa: Isa::Sse2,
        kind: ErrorKind::IoError,
        logical: None,
        core: None,
        iteration,
        expected: 0,
        actual: u64::from(code),
        seed,
        load_percent: None,
        transient: Some(transient),
    })
}

struct Run<'r> {
    plan: &'r Plan,
    out: &'r (dyn Fn(LoadMessage) + Sync),
    stop: &'r AtomicBool,
    /// The fault injection of the verified loads (C6).
    _inject: Option<Inject>,
    hooks: &'r DiskHooks<'r>,
    start: Instant,
    phase: u32,
    checks: u64,
    errors: u64,
    /// Bytes read and written since the run started, for `Progress.disk`.
    read_total: u64,
    written_total: u64,
}

/// The counters of the last `Progress`, for the rates of the next.
struct Meter {
    at: Instant,
    read: u64,
    written: u64,
    ios: u64,
}

impl Run<'_> {
    fn send(&self, msg: LoadMessage) {
        (self.out)(msg);
    }

    fn now(&self) -> Instant {
        (self.hooks.clock)()
    }

    fn notice(&self, code: &str, value: Option<u64>) {
        self.send(LoadMessage::Notice(Notice {
            phase: self.phase,
            code: code.to_owned(),
            value,
        }));
    }

    fn end(&self, reason: FinishReason) -> DiskRunEnd {
        DiskRunEnd {
            finished: Finished {
                reason,
                checks: self.checks,
                errors: self.errors,
            },
            exit_code: EXIT_OK,
        }
    }

    fn fatal(&mut self, f: Fatal) -> DiskRunEnd {
        let mut end = self.end(FinishReason::Failed);
        match f {
            Fatal::Full => {
                tracing::warn!("the disk is full");
                self.notice("disk_full", None);
            }
            Fatal::Denied => {
                tracing::warn!("the test folder refuses our files");
                self.notice("access_denied", None);
            }
            Fatal::Io { iteration, code } => {
                tracing::error!(code, iteration, "persistent disk I/O error");
                let kernel = self
                    .plan
                    .phases
                    .get(self.phase as usize)
                    .map_or(KernelId::DiskFill, |p| p.kernel);
                let seed = phase_seed(self.plan.seed, self.phase);
                self.errors += 1;
                end.finished.errors = self.errors;
                self.send(io_error(self.phase, kernel, seed, iteration, code, false));
                end.exit_code = EXIT_IO;
            }
        }
        end
    }

    /// A `Progress` of the current phase; `meter` gives the disk rates since the last one.
    fn progress(&self, phase_elapsed: Duration, meter: Option<(&mut Meter, u64, u64, u64)>) {
        let now = self.now();
        let disk = meter.map(|(m, read, written, ios)| {
            let dt = (now - m.at).as_secs_f64();
            let per_s = |n: u64| if dt > 0.0 { n as f64 / dt } else { 0.0 };
            let d = DiskProgress {
                read_bps: per_s(read - m.read),
                write_bps: per_s(written - m.written),
                read_bytes: self.read_total + read,
                written_bytes: self.written_total + written,
                iops: per_s(ios - m.ios),
            };
            *m = Meter {
                at: now,
                read,
                written,
                ios,
            };
            d
        });
        self.send(LoadMessage::Progress(Progress {
            phase: self.phase,
            phase_elapsed_ms: ms(phase_elapsed),
            elapsed_ms: ms(now - self.start),
            checks: self.checks,
            errors: self.errors,
            current_core: None,
            cores: Vec::new(),
            memory_bytes: 0,
            rate: disk.as_ref().map(|d| d.read_bps + d.write_bps),
            load_percent: None,
            disk,
        }));
    }

    /// Runs `f` while a thread sends a `Progress` without rates every [`HEARTBEAT`].
    fn heartbeat<R>(&self, phase: u32, f: impl FnOnce() -> R) -> R {
        let (done, beat) = mpsc::channel::<()>();
        let (out, clock, start) = (self.out, self.hooks.clock, self.start);
        let base = Progress {
            phase,
            phase_elapsed_ms: 0,
            elapsed_ms: 0,
            checks: self.checks,
            errors: self.errors,
            current_core: None,
            cores: Vec::new(),
            memory_bytes: 0,
            rate: None,
            load_percent: None,
            disk: None,
        };
        thread::scope(|s| {
            s.spawn(move || {
                // Ends when `done` is dropped, right after `f`.
                while beat.recv_timeout(HEARTBEAT) == Err(RecvTimeoutError::Timeout) {
                    out(LoadMessage::Progress(Progress {
                        elapsed_ms: ms(clock() - start),
                        ..base.clone()
                    }));
                }
            });
            let result = f();
            drop(done);
            result
        })
    }

    fn skip(&self, why: &str) -> End {
        self.send(LoadMessage::PhaseDone(PhaseDone {
            phase: self.phase,
            checks: 0,
            errors: 0,
            duration_ms: 0,
            skipped: Some(why.to_owned()),
            work_ms: None,
            workers: Vec::new(),
            rates: Vec::new(),
            disk: None,
        }));
        End::Skipped
    }

    /// The pause before a phase, outside its duration, with a `Progress` at least every
    /// [`HEARTBEAT`]; `false` when a stop came.
    fn pause(&self, ms_before: u32) -> bool {
        let from = self.now();
        let until = from + Duration::from_millis(ms_before.into());
        let mut beat = from;
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return false;
            }
            let now = self.now();
            if now >= until {
                return true;
            }
            if now - beat >= HEARTBEAT {
                beat = now;
                self.progress(Duration::ZERO, None);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn run_phase(&mut self, file: &dyn DataFileApi, spec: &Phase) -> Result<End, Fatal> {
        // `validate` gives every disk phase a job.
        let Some(job) = spec.disk.as_ref() else {
            return Ok(self.skip("unsupported"));
        };
        let (smallest, largest) = used_blocks(job);
        let sector = file.sector();
        let bytes = file.bytes();
        if sector == 0
            || smallest < sector
            || !smallest.is_multiple_of(sector)
            || !bytes.is_multiple_of(u64::from(sector))
        {
            self.notice("disk_sector", Some(u64::from(sector)));
            return Ok(self.skip("disk_sector"));
        }
        if u64::from(largest) > bytes {
            return Ok(self.skip("unsupported"));
        }
        let seed = phase_seed(self.plan.seed, self.phase);
        let ctx = LoadCtx {
            kernel: spec.kernel,
            job,
            file_bytes: bytes,
            seed,
            session: self.plan.seed,
            compressible: self.plan.disk.as_ref().is_some_and(|d| d.compressible),
        };
        let mut loads = Vec::with_capacity(usize::from(job.threads));
        for t in 0..job.threads {
            match worker_load(&ctx, t) {
                Some(load) => loads.push(load),
                None => return Ok(self.skip("unsupported")),
            }
        }
        if spec.pause_before_ms > 0 && !self.pause(spec.pause_before_ms) {
            return Ok(End::Stopped);
        }
        let mut queues = Vec::with_capacity(loads.len());
        for _ in 0..loads.len() {
            match (self.hooks.queue)(file, job) {
                Ok(q) => queues.push(q),
                Err(DiskError::Full) => return Err(Fatal::Full),
                Err(e) => {
                    return Err(Fatal::Io {
                        iteration: 0,
                        code: code_of(e),
                    })
                }
            }
        }
        let shared = Shared {
            phase: self.phase,
            kernel: spec.kernel,
            seed,
            clock: self.hooks.clock,
            out: self.out,
            halt: AtomicBool::new(false),
            write_left: AtomicU64::new(job.write_cap_bytes.unwrap_or(u64::MAX)),
            fatal: Mutex::new(None),
            read_bytes: AtomicU64::new(0),
            write_bytes: AtomicU64::new(0),
            ios: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            errors_sent: AtomicU32::new(0),
        };
        // The phase clock starts at the first submissions, after the pause.
        let phase_start = self.now();
        let duration = Duration::from_secs(spec.duration_s.into());
        let ends = thread::scope(|s| {
            let handles: Vec<_> = queues
                .into_iter()
                .zip(loads)
                .map(|(q, load)| {
                    let shared = &shared;
                    s.spawn(move || worker(shared, q, load))
                })
                .collect();
            let mut meter = Meter {
                at: phase_start,
                read: 0,
                written: 0,
                ios: 0,
            };
            while !handles.iter().all(|h| h.is_finished()) {
                thread::sleep(SLICE);
                let now = self.now();
                if self.stop.load(Ordering::Relaxed) || now - phase_start >= duration {
                    shared.halt.store(true, Ordering::Relaxed);
                }
                if now - meter.at >= PROGRESS_EVERY {
                    let (r, w, n) = shared.counters();
                    self.progress(now - phase_start, Some((&mut meter, r, w, n)));
                }
            }
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
                .collect::<Vec<_>>()
        });
        let phase_errors = shared.errors.load(Ordering::Relaxed);
        self.errors += phase_errors;
        let (read, written, _) = shared.counters();
        self.read_total += read;
        self.written_total += written;
        if let Some(fatal) = shared.fatal.into_inner().unwrap_or_else(|p| p.into_inner()) {
            return Err(fatal);
        }
        let stats = merge(&ends);
        self.send(LoadMessage::PhaseDone(PhaseDone {
            phase: self.phase,
            checks: 0,
            errors: phase_errors,
            duration_ms: ms(self.now() - phase_start),
            skipped: None,
            work_ms: None,
            workers: Vec::new(),
            rates: Vec::new(),
            disk: Some(stats),
        }));
        Ok(if self.stop.load(Ordering::Relaxed) {
            End::Stopped
        } else {
            End::Done
        })
    }
}

/// The totals of a phase: both directions share its time, from the first submission to the
/// last completion of any worker.
fn merge(ends: &[WorkerEnd]) -> DiskPhaseStats {
    let first = ends.iter().filter_map(|e| e.first).min();
    let last = ends.iter().filter_map(|e| e.last).max();
    let elapsed_us = match (first, last) {
        (Some(a), Some(b)) => (b.saturating_duration_since(a)).as_micros() as u64,
        _ => 0,
    };
    let side = |pick: fn(&WorkerEnd) -> (&Histogram, u64)| {
        let mut h = Histogram::new();
        let mut bytes = 0u64;
        for e in ends {
            let (eh, eb) = pick(e);
            h.merge(eh);
            bytes = bytes.saturating_add(eb);
        }
        IoStats {
            bytes,
            ios: h.count(),
            elapsed_us: if h.count() > 0 { elapsed_us } else { 0 },
            mean_lat_us: h.mean_us(),
            p99_lat_us: h.p99_us(),
        }
    };
    DiskPhaseStats {
        read: side(|e| (&e.read, e.read_bytes)),
        write: side(|e| (&e.write, e.write_bytes)),
    }
}

/// What the workers of a phase share with each other and the engine thread.
struct Shared<'a> {
    phase: u32,
    kernel: KernelId,
    seed: u64,
    clock: &'a (dyn Fn() -> Instant + Sync),
    out: &'a (dyn Fn(LoadMessage) + Sync),
    /// Stop submitting: the duration, the write cap, a stop or a fatal error.
    halt: AtomicBool,
    /// Bytes of writes that may still start (the write cap).
    write_left: AtomicU64,
    /// The first fatal error; the engine thread reports it.
    fatal: Mutex<Option<Fatal>>,
    read_bytes: AtomicU64,
    write_bytes: AtomicU64,
    ios: AtomicU64,
    errors: AtomicU64,
    errors_sent: AtomicU32,
}

impl Shared<'_> {
    fn counters(&self) -> (u64, u64, u64) {
        (
            self.read_bytes.load(Ordering::Relaxed),
            self.write_bytes.load(Ordering::Relaxed),
            self.ios.load(Ordering::Relaxed),
        )
    }

    /// Takes `len` bytes from the write cap; `false` (and nothing taken) when they would
    /// pass it.
    fn reserve_write(&self, len: u32) -> bool {
        self.write_left
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                left.checked_sub(u64::from(len))
            })
            .is_ok()
    }

    fn has_fatal(&self) -> bool {
        self.fatal.lock().map_or(true, |f| f.is_some())
    }

    /// Records the first fatal error and halts the phase.
    fn set_fatal(&self, f: Fatal) {
        if let Ok(mut slot) = self.fatal.lock() {
            slot.get_or_insert(f);
        }
        self.halt.store(true, Ordering::Relaxed);
    }

    /// A retry that came back right: the error is reported and the phase goes on.
    fn transient(&self, req: &IoReq, code: u32) {
        self.errors.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(code, offset = req.offset, "transient disk I/O error");
        if self.errors_sent.fetch_add(1, Ordering::Relaxed) < ERRORS_PER_PHASE {
            let iteration = req.offset / BLOCK_BYTES as u64;
            (self.out)(io_error(
                self.phase,
                self.kernel,
                self.seed,
                iteration,
                code,
                true,
            ));
        }
    }
}

/// What a worker knows of its phase.
pub struct LoadCtx<'a> {
    pub kernel: KernelId,
    pub job: &'a DiskJob,
    pub file_bytes: u64,
    /// The phase seed (random offsets and data).
    pub seed: u64,
    /// The block session (`plan.seed`, DC3).
    pub session: u64,
    pub compressible: bool,
}

/// What one worker submits in a phase. `next` gives `None` when the worker's work is done;
/// `prepare` fills the buffer of a write just before it starts.
pub trait WorkerLoad: Send {
    /// Called once, before any I/O, with every slot idle.
    fn init(&mut self, _q: &mut dyn IoQueue) {}
    fn next(&mut self) -> Option<IoReq>;
    fn prepare(&mut self, _req: &IoReq, _buf: &mut [u8]) {}
}

/// The load of worker `t`, or `None` for a kernel without one (the phase is skipped).
pub fn worker_load(ctx: &LoadCtx<'_>, t: u16) -> Option<Box<dyn WorkerLoad>> {
    match ctx.kernel {
        KernelId::DiskFill => Some(Box::new(Fill::new(ctx, t))),
        KernelId::DiskBench => Some(Box::new(Bench {
            picker: IoPicker::new(ctx.job, ctx.file_bytes, t, ctx.seed),
            seed: ctx.seed ^ u64::from(t),
            compressible: ctx.compressible,
        })),
        _ => None,
    }
}

/// `disk_fill`: every block of the worker's stripe written once, generation 1.
struct Fill {
    cursor: u64,
    end: u64,
    block: u64,
    session: u64,
    compressible: bool,
}

impl Fill {
    fn new(ctx: &LoadCtx<'_>, t: u16) -> Fill {
        let block = u64::from(ctx.job.seq_block_bytes.max(BLOCK_BYTES as u32));
        let threads = u128::from(ctx.job.threads.max(1));
        let stripe = |t: u128| -> u64 {
            let at = (u128::from(ctx.file_bytes) * t / threads) as u64;
            at / block * block
        };
        let t = u128::from(t);
        let end = if t + 1 == threads {
            ctx.file_bytes
        } else {
            stripe(t + 1)
        };
        Fill {
            cursor: stripe(t),
            end,
            block,
            session: ctx.session,
            compressible: ctx.compressible,
        }
    }
}

impl WorkerLoad for Fill {
    fn next(&mut self) -> Option<IoReq> {
        if self.cursor >= self.end {
            return None;
        }
        // The file is a multiple of the sector, and so of 4 KiB; the stripes are too.
        let len = self.block.min(self.end - self.cursor);
        let req = IoReq {
            offset: self.cursor,
            len: len as u32,
            write: true,
        };
        self.cursor += len;
        Some(req)
    }

    fn prepare(&mut self, req: &IoReq, buf: &mut [u8]) {
        let first = req.offset / BLOCK_BYTES as u64;
        for (i, block) in buf.chunks_exact_mut(BLOCK_BYTES).enumerate() {
            write_block(block, self.session, first + i as u64, 1, self.compressible);
        }
    }
}

/// `disk_bench`: the job's offsets, with each slot's buffer filled once (DC3).
struct Bench {
    picker: IoPicker,
    seed: u64,
    compressible: bool,
}

impl WorkerLoad for Bench {
    fn init(&mut self, q: &mut dyn IoQueue) {
        let mut rng = Xoshiro256ss::new(self.seed);
        for slot in 0..q.slots() {
            let buf = q.buffer(slot);
            if self.compressible {
                buf.fill(0);
            } else {
                for w in buf.chunks_exact_mut(8) {
                    w.copy_from_slice(&rng.next_u64().to_le_bytes());
                }
            }
        }
    }

    fn next(&mut self) -> Option<IoReq> {
        Some(self.picker.next())
    }
}

#[derive(Default)]
struct WorkerEnd {
    read: Histogram,
    write: Histogram,
    read_bytes: u64,
    write_bytes: u64,
    first: Option<Instant>,
    last: Option<Instant>,
}

/// An I/O in flight on a slot; `retry_of` is the code of the first try of a retry.
#[derive(Clone, Copy)]
struct Pending {
    req: IoReq,
    retry_of: Option<u32>,
}

/// One worker of a phase: keeps its queue full until the phase halts or its load ends,
/// then waits for the I/Os in flight. The queue is dropped (and so drained) on every path,
/// a panic included.
fn worker(sh: &Shared<'_>, mut q: Box<dyn IoQueue>, mut load: Box<dyn WorkerLoad>) -> WorkerEnd {
    crate::sys::prepare_worker();
    load.init(&mut *q);
    let n = q.slots();
    let mut w = Worker {
        sh,
        slots: vec![None; n],
        idle: (0..n).rev().collect(),
        in_flight: 0,
        end: WorkerEnd::default(),
    };
    let mut exhausted = false;
    let mut done = Vec::with_capacity(n);
    let mut halted_at: Option<Instant> = None;
    loop {
        let halted = sh.halt.load(Ordering::Relaxed);
        if !halted && !exhausted {
            while let Some(&slot) = w.idle.last() {
                if sh.halt.load(Ordering::Relaxed) {
                    break;
                }
                let Some(req) = load.next() else {
                    exhausted = true;
                    break;
                };
                if req.write && !sh.reserve_write(req.len) {
                    // The write cap is reached: the phase ends.
                    sh.halt.store(true, Ordering::Relaxed);
                    break;
                }
                if req.write {
                    // The borrow of the buffer ends before its I/O starts.
                    load.prepare(&req, &mut q.buffer(slot)[..req.len as usize]);
                }
                w.idle.pop();
                w.end.first.get_or_insert_with(sh.clock);
                w.start(
                    &mut *q,
                    slot,
                    Pending {
                        req,
                        retry_of: None,
                    },
                );
            }
        }
        if w.in_flight == 0 {
            if halted || exhausted || sh.halt.load(Ordering::Relaxed) {
                break;
            }
            continue;
        }
        if halted {
            let since = *halted_at.get_or_insert_with(Instant::now);
            if since.elapsed() > DRAIN_LIMIT {
                tracing::error!(in_flight = w.in_flight, "disk I/O does not complete");
                break;
            }
        }
        if let Err(e) = q.wait(WAIT_MS, &mut done) {
            // The queue itself broke: nothing more completes on it.
            sh.set_fatal(Fatal::Io {
                iteration: 0,
                code: code_of(e),
            });
            break;
        }
        for d in done.drain(..) {
            w.completed(&mut *q, d);
        }
    }
    // Dropping the queue cancels and drains what is left before the buffers go.
    drop(q);
    w.end
}

struct Worker<'s, 'a> {
    sh: &'s Shared<'a>,
    slots: Vec<Option<Pending>>,
    idle: Vec<usize>,
    in_flight: usize,
    end: WorkerEnd,
}

impl Worker<'_, '_> {
    fn start(&mut self, q: &mut dyn IoQueue, slot: usize, p: Pending) {
        match q.submit(p.req, slot) {
            Ok(()) => {
                self.slots[slot] = Some(p);
                self.in_flight += 1;
            }
            Err(e) => self.failed(q, slot, p, code_of(e)),
        }
    }

    fn completed(&mut self, q: &mut dyn IoQueue, d: IoDone) {
        let Some(p) = self.slots.get_mut(d.slot).and_then(Option::take) else {
            return;
        };
        self.in_flight -= 1;
        self.end.last = Some((self.sh.clock)());
        match d.result {
            Ok(n) if n == p.req.len => {
                let n = u64::from(n);
                if p.req.write {
                    self.end.write.record(d.latency_us);
                    self.end.write_bytes += n;
                    self.sh.write_bytes.fetch_add(n, Ordering::Relaxed);
                } else {
                    self.end.read.record(d.latency_us);
                    self.end.read_bytes += n;
                    self.sh.read_bytes.fetch_add(n, Ordering::Relaxed);
                }
                self.sh.ios.fetch_add(1, Ordering::Relaxed);
                if let Some(code) = p.retry_of {
                    self.sh.transient(&p.req, code);
                }
                self.idle.push(d.slot);
            }
            Ok(_) => self.failed(q, d.slot, p, ERROR_HANDLE_EOF),
            Err(code) => self.failed(q, d.slot, p, code),
        }
    }

    /// An I/O that failed (at submission or completion): full disk ends the run, another
    /// error is retried once on the same slot, a failed retry ends the run with `EXIT_IO`.
    fn failed(&mut self, q: &mut dyn IoQueue, slot: usize, p: Pending, code: u32) {
        if self.sh.has_fatal() {
            // Draining after a fatal error: the I/Os still in flight are not reported.
            self.idle.push(slot);
            return;
        }
        if classify_win32(code) == DiskError::Full {
            self.sh.set_fatal(Fatal::Full);
            self.idle.push(slot);
            return;
        }
        match p.retry_of {
            None => {
                tracing::warn!(code, offset = p.req.offset, "disk I/O error: retrying once");
                // A write retries with its buffer as it was: nothing touched it since.
                self.start(
                    q,
                    slot,
                    Pending {
                        req: p.req,
                        retry_of: Some(code),
                    },
                );
            }
            Some(_) => {
                self.sh.set_fatal(Fatal::Io {
                    iteration: p.req.offset / BLOCK_BYTES as u64,
                    code,
                });
                self.idle.push(slot);
            }
        }
    }
}
