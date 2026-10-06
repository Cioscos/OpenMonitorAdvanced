//! The phase engine (§4.2–§4.4, DA7, DA9, DA10, DA18): runs the phases of a plan one after
//! the other on pinned worker threads, checks every iteration against the reference and
//! reports progress, errors and notices through `out`.
//!
//! A phase (or one slice of a `core_cycle` phase) runs as one *set* of workers:
//! 1. the references, on up to three cores that must agree (DA7), on a helper thread;
//! 2. the workers, each building its kernels on its own pinned thread; a memory error in
//!    either step starts the set again smaller (DA10);
//! 3. the load mode.
//!
//! The engine thread polls all along: it sends `Progress` once a second and ends the set
//! on a stop, a first error, the deadline, a hung or a crashed worker. A sentinel thread
//! watches the workers' beats for the whole run.

mod modes;
mod schedule;
mod sentinel;

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use oma_core::load::KEEP_FREE_BYTES;
use oma_ipc::load::{
    ComputeError, CoreProgress, CoreState, DataSize, ErrorKind, FinishReason, Finished, Isa,
    KernelId, LoadMessage, LoadMode, LogicalCpu, Notice, Phase, PhaseDone, Placement, Plan,
    Progress, Topology,
};

use crate::args::Inject;
use crate::kernel::{
    Check, Kernel, KernelError, KernelFactory, PhaseShared, RefFailure, ThreadBudget, WorkerCtx,
};
use crate::rng::phase_seed;
use crate::verify::{self, RefError};

/// Where the engine finds the kernels: [`crate::kernel::factory`] outside the tests.
pub type FactoryFn<'a> = dyn Fn(KernelId) -> Option<&'a dyn KernelFactory> + Sync + 'a;

pub struct Hooks<'h, 'f> {
    pub factory: &'h FactoryFn<'f>,
    /// Called by the sentinel after it sent `Error { hung }` and `Finished { failed }`:
    /// the process exits, since a hung thread cannot be stopped.
    pub on_hung: &'h (dyn Fn() + Sync),
    /// The physical memory available now, in bytes; `None` when it cannot be read.
    pub available: &'h (dyn Fn() -> Option<u64> + Sync),
}

/// How often the engine thread looks at the clock, the stop flag and the errors.
const POLL: Duration = Duration::from_millis(50);
const PROGRESS_EVERY: Duration = Duration::from_secs(1);
/// The shortest gap between two `Progress` sent because of new errors (4 Hz).
const PROGRESS_AFTER_ERROR: Duration = Duration::from_millis(250);
/// `Error` messages per worker and phase; the errors after them are only counted.
const ERRORS_PER_WORKER: u32 = 16;
/// The iteration whose check the fault injection flips (DA18), counted from 1.
const INJECT_AT: u64 = 3;

/// The reference digests of each kernel of a set; `None` for a self-checking kernel.
type Refs = Vec<Option<Vec<u64>>>;

/// Runs `plan` and returns how it ended; the caller sends the `Finished`. A hung worker
/// ends the process instead (after the sentinel sent its own `Finished`).
pub fn run(
    plan: &Plan,
    topology: &Topology,
    out: &(dyn Fn(LoadMessage) + Sync),
    stop: &AtomicBool,
    inject: Option<Inject>,
) -> Finished {
    let exit = || {
        crate::log::flush();
        std::process::exit(crate::link::EXIT_OK)
    };
    let hooks = Hooks {
        factory: &crate::kernel::factory,
        on_hung: &exit,
        available: &crate::sys::available_memory,
    };
    run_with(plan, topology, out, stop, inject, &hooks)
}

/// [`run`] with the kernels and the end of a hung run given by the caller.
pub fn run_with(
    plan: &Plan,
    topology: &Topology,
    out: &(dyn Fn(LoadMessage) + Sync),
    stop: &AtomicBool,
    inject: Option<Inject>,
    hooks: &Hooks<'_, '_>,
) -> Finished {
    let order = oma_core::load::core_order(topology);
    let engine = Engine {
        plan,
        topology,
        out,
        stop,
        inject,
        hooks,
        ref_cpus: schedule::reference_cpus(topology, &order),
        order,
        start: Instant::now(),
        checks: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        iterations: AtomicU64::new(0),
        failed_cores: Mutex::new(Vec::new()),
        hung: AtomicBool::new(false),
        closed: Mutex::new(false),
        crashed: AtomicBool::new(false),
        watched: Mutex::new(Watched::default()),
    };
    let done = AtomicBool::new(false);
    thread::scope(|s| {
        let sentinel = s.spawn(|| {
            sentinel::watch_loop(
                &done,
                || engine.beats(),
                crate::sys::asleep_ms,
                |i| engine.hung_worker(i),
            )
        });
        // Also on a panic, so the scope can join the sentinel.
        let _done = Done(&done, sentinel.thread().clone());
        engine.run_phases()
    })
}

/// Ends the sentinel when dropped.
struct Done<'a>(&'a AtomicBool, thread::Thread);

impl Drop for Done<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
        self.1.unpark();
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

struct Engine<'e, 'f> {
    plan: &'e Plan,
    topology: &'e Topology,
    out: &'e (dyn Fn(LoadMessage) + Sync),
    stop: &'e AtomicBool,
    inject: Option<Inject>,
    hooks: &'e Hooks<'e, 'f>,
    order: Vec<u32>,
    ref_cpus: Vec<LogicalCpu>,
    start: Instant,
    checks: AtomicU64,
    /// Mismatches and reference disagreements (`reference_invalid` is not a core's error).
    errors: AtomicU64,
    iterations: AtomicU64,
    /// Cores with a mismatch in any phase: `Failed` in `Progress`, skipped by the later
    /// `core_cycle` phases (DA11).
    failed_cores: Mutex<Vec<u32>>,
    hung: AtomicBool,
    /// Set by the sentinel with its `Finished`: nothing is sent after it.
    closed: Mutex<bool>,
    /// A kernel panicked.
    crashed: AtomicBool,
    watched: Mutex<Watched>,
}

/// One worker of a set.
struct Slot {
    cpu: LogicalCpu,
    beat: AtomicU64,
    iterations: AtomicU64,
    errors_sent: AtomicU32,
}

/// The running set, as the sentinel sees it; `generation` changes with every set.
#[derive(Default)]
struct Watched {
    generation: u64,
    live: Option<Published>,
}

struct Published {
    phase: u32,
    kernel: KernelId,
    isa: Isa,
    seed: u64,
    slots: Vec<Arc<Slot>>,
}

/// The engine thread's view of the run, for `Progress`.
struct Cursor {
    phase: u32,
    phase_start: Instant,
    current_core: Option<u32>,
    /// `Untested`, `Testing` or `Passed`; `Failed` comes from `failed_cores`.
    cores: Vec<CoreProgress>,
    memory_bytes: u64,
    sent_at: Instant,
    rate_iterations: u64,
}

impl Cursor {
    fn state(&self, core: u32) -> CoreState {
        self.cores
            .iter()
            .find(|c| c.core == core)
            .map_or(CoreState::Untested, |c| c.state)
    }

    fn set_state(&mut self, core: u32, state: CoreState) {
        if let Some(c) = self.cores.iter_mut().find(|c| c.core == core) {
            c.state = state;
        }
    }
}

/// A phase being run.
struct PhaseRun<'p> {
    index: u32,
    spec: &'p Phase,
    seed: u64,
    /// The phase kernel, then `alt_kernel` in `variable` mode.
    kernels: Vec<(KernelId, &'p dyn KernelFactory)>,
    /// The fault injection happens once per phase.
    injected: AtomicBool,
}

/// What a phase learnt in its earlier sets.
struct Memo {
    /// How many of the phase kernels run: 1 once the alt kernel turned out unsupported.
    kernels: usize,
    /// Bytes per thread after a reduction (DA10).
    ram_cap: Option<u64>,
    /// The references, valid for this budget.
    refs: Option<(ThreadBudget, Refs)>,
}

/// The flags of one set, shared with its workers.
struct Live {
    shared: Arc<PhaseShared>,
    failed: AtomicBool,
    first_error: AtomicBool,
    inject_worker: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    /// The time ran out.
    Done,
    /// A `core_cycle` slice found an error on its core.
    CoreFailed,
    Stopped,
    FirstError,
    /// Hung or crashed: the run ends as `failed`.
    Aborted,
    Skipped(&'static str),
}

enum Attempt {
    End(End),
    /// Start the set again with this many bytes per thread.
    Reduce(u64),
    /// Start the set again with one thread per physical core.
    OnePerCore,
    /// Start the set again without the alt kernel.
    DropAlt,
}

/// Why a worker has no kernels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildFail {
    /// Kernel `k` of the phase refused.
    Kernel(usize, KernelError),
    Panic,
}

/// Why the engine's reference closure stopped.
enum RefStop {
    Failure(RefFailure),
    Stopped,
}

/// Holds the built workers until the engine says go (`true`) or abort (`false`).
#[derive(Default)]
struct Gate {
    decision: Mutex<Option<bool>>,
    cv: Condvar,
}

impl Gate {
    fn set(&self, go: bool) {
        let mut d = lock(&self.decision);
        if d.is_none() {
            *d = Some(go);
            self.cv.notify_all();
        }
    }

    fn wait(&self) -> bool {
        let mut d = lock(&self.decision);
        while d.is_none() {
            d = self.cv.wait(d).unwrap_or_else(PoisonError::into_inner);
        }
        d.unwrap_or(false)
    }
}

/// On every way out of a set (a panic too): abort the gate and raise `quit`, so no worker
/// waits or runs for good.
struct AbortOnDrop<'g>(&'g Gate, &'g PhaseShared);

impl Drop for AbortOnDrop<'_> {
    fn drop(&mut self) {
        self.0.set(false);
        self.1.quit.store(true, Ordering::Relaxed);
    }
}

impl Engine<'_, '_> {
    /// Sends `msg` unless the sentinel already sent the final `Finished`.
    fn send(&self, msg: LoadMessage) {
        let closed = lock(&self.closed);
        if !*closed {
            (self.out)(msg);
        }
    }

    fn finish(&self, reason: FinishReason) -> Finished {
        Finished {
            reason,
            checks: self.checks.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }

    fn is_failed(&self, core: u32) -> bool {
        lock(&self.failed_cores).contains(&core)
    }

    fn run_phases(&self) -> Finished {
        let now = Instant::now();
        let mut cur = Cursor {
            phase: 0,
            phase_start: now,
            current_core: None,
            cores: self
                .order
                .iter()
                .map(|&core| CoreProgress {
                    core,
                    state: CoreState::Untested,
                })
                .collect(),
            memory_bytes: 0,
            sent_at: now,
            rate_iterations: 0,
        };
        for (index, spec) in self.plan.phases.iter().enumerate() {
            if self.stop.load(Ordering::Relaxed) {
                return self.finish(FinishReason::Stopped);
            }
            let index = index as u32;
            cur.phase = index;
            cur.phase_start = Instant::now();
            cur.current_core = None;
            self.progress(&mut cur);
            let (checks, errors) = (
                self.checks.load(Ordering::Relaxed),
                self.errors.load(Ordering::Relaxed),
            );
            let end = self.phase(&mut cur, index, spec);
            if end == End::Aborted {
                return self.finish(FinishReason::Failed);
            }
            self.send(LoadMessage::PhaseDone(PhaseDone {
                phase: index,
                checks: self.checks.load(Ordering::Relaxed) - checks,
                errors: self.errors.load(Ordering::Relaxed) - errors,
                duration_ms: ms(cur.phase_start.elapsed()),
                skipped: match end {
                    End::Skipped(reason) => Some(reason.to_owned()),
                    _ => None,
                },
                work_ms: None,
            }));
            match end {
                End::Stopped => return self.finish(FinishReason::Stopped),
                End::FirstError => return self.finish(FinishReason::FirstError),
                _ => {}
            }
        }
        self.finish(FinishReason::Completed)
    }

    fn phase(&self, cur: &mut Cursor, index: u32, spec: &Phase) -> End {
        let Some(main) = (self.hooks.factory)(spec.kernel) else {
            return End::Skipped("unsupported");
        };
        let mut kernels = vec![(spec.kernel, main)];
        if spec.mode == LoadMode::Variable {
            if let Some(alt) = spec.alt_kernel {
                match (self.hooks.factory)(alt) {
                    Some(f) => kernels.push((alt, f)),
                    None => tracing::info!(kernel = ?alt, "no alt kernel: the main one runs alone"),
                }
            }
        }
        let pr = PhaseRun {
            index,
            spec,
            seed: phase_seed(self.plan.seed, index),
            kernels,
            injected: AtomicBool::new(false),
        };
        let deadline = cur.phase_start + Duration::from_secs(spec.duration_s.into());
        let mut memo = Memo {
            kernels: pr.kernels.len(),
            ram_cap: None,
            refs: None,
        };
        if spec.placement == Placement::CoreCycle {
            return self.core_cycle(cur, &pr, &mut memo, deadline);
        }
        let per_core = match spec.placement {
            Placement::OnePerCore => 1,
            _ => usize::MAX,
        };
        // `cores` limits every placement to the listed cores (the order of `self.order`).
        let order: Vec<u32> = match &spec.cores {
            Some(list) => self
                .order
                .iter()
                .copied()
                .filter(|c| list.contains(c))
                .collect(),
            None => self.order.clone(),
        };
        let mut cpus = schedule::phase_cpus(self.topology, &order, per_core);
        if spec.mode == LoadMode::Light {
            cpus.truncate(1);
        }
        if cpus.is_empty() {
            return End::Skipped("no_cpu");
        }
        match self.run_set(cur, &pr, &mut memo, cpus, deadline) {
            End::CoreFailed => End::Done,
            end => end,
        }
    }

    /// One core at a time (§4.4): `per_core_s` on each core of the phase, wrapping around
    /// until `duration_s` elapses; failed cores are skipped, also those of earlier phases.
    fn core_cycle(
        &self,
        cur: &mut Cursor,
        pr: &PhaseRun,
        memo: &mut Memo,
        deadline: Instant,
    ) -> End {
        let spec = pr.spec;
        let cores: Vec<u32> = match &spec.cores {
            Some(list) => list
                .iter()
                .copied()
                .filter(|c| self.order.contains(c))
                .collect(),
            None => self.order.clone(),
        };
        let per_core = Duration::from_secs(spec.per_core_s.unwrap_or(spec.duration_s).into());
        let both = spec.both_smt && spec.mode != LoadMode::Light;
        let mut from = 0;
        let mut ran = false;
        while Instant::now() < deadline {
            let Some(i) = schedule::next_core(&cores, from, |c| self.is_failed(c)) else {
                break;
            };
            from = i + 1;
            let core = cores[i];
            let cpus = schedule::core_cpus(self.topology, core, both);
            ran = true;
            let before = cur.state(core);
            cur.set_state(core, CoreState::Testing);
            cur.current_core = Some(core);
            let slice_deadline = (Instant::now() + per_core).min(deadline);
            let end = self.run_set(cur, pr, memo, cpus, slice_deadline);
            cur.current_core = None;
            // `Failed` comes from `failed_cores`, set only by a worker of this core.
            cur.set_state(
                core,
                if end == End::Done {
                    CoreState::Passed
                } else {
                    before
                },
            );
            self.progress(cur);
            match end {
                End::Done | End::CoreFailed => {}
                other => return other,
            }
        }
        // No slice ran: no core to cycle through, never a phase passed without a check.
        if ran {
            End::Done
        } else {
            End::Skipped("no_cpu")
        }
    }

    /// Runs one set on `cpus` until `deadline`, starting it again smaller when memory is
    /// short.
    fn run_set(
        &self,
        cur: &mut Cursor,
        pr: &PhaseRun,
        memo: &mut Memo,
        mut cpus: Vec<LogicalCpu>,
        deadline: Instant,
    ) -> End {
        if pr.spec.kernel == KernelId::K9 {
            // K9 works in pairs: an odd last worker has no partner and is dropped.
            cpus.truncate(cpus.len() & !1);
            if cpus.is_empty() {
                self.notice(pr, "k9_needs_two_cores", None);
                return End::Skipped("k9_needs_two_cores");
            }
        }
        loop {
            let mut budget = ThreadBudget::for_workers(self.topology, &cpus, self.plan.ram_bytes);
            if let Some(cap) = memo.ram_cap {
                budget.ram_per_thread = budget.ram_per_thread.min(cap);
            }
            // The memory may have gone since the plan was built: each set leaves 2 GiB free.
            if uses_ram(pr.spec) {
                if let Some(available) = (self.hooks.available)() {
                    let cap = available.saturating_sub(KEEP_FREE_BYTES) / cpus.len().max(1) as u64;
                    if cap < budget.ram_per_thread {
                        self.notice(pr, "ram_reduced", Some(cap));
                        budget.ram_per_thread = cap;
                        memo.ram_cap = Some(cap);
                    }
                }
            }
            match self.attempt(cur, pr, memo, &cpus, budget, deadline) {
                Attempt::End(end) => return end,
                Attempt::Reduce(bytes) => memo.ram_cap = Some(bytes),
                Attempt::OnePerCore => {
                    cpus = schedule::one_per_core(&cpus);
                    // Fewer threads share the quota: start again from the full share.
                    memo.ram_cap = None;
                }
                Attempt::DropAlt => {
                    tracing::info!(phase = pr.index, "the alt kernel is unsupported: dropped");
                    memo.kernels = 1;
                    memo.refs = None;
                }
            }
        }
    }

    fn notice(&self, pr: &PhaseRun, code: &str, value: Option<u64>) {
        self.send(LoadMessage::Notice(Notice {
            phase: pr.index,
            code: code.to_owned(),
            value,
        }));
    }

    /// What to do after kernel `k` refused (DA10): skip, drop the alt kernel, or start the
    /// set again smaller.
    fn decide(
        &self,
        pr: &PhaseRun,
        fails: &[(usize, KernelError)],
        cpus: &[LogicalCpu],
        budget: ThreadBudget,
    ) -> Attempt {
        let unsupported = |main: bool| {
            fails
                .iter()
                .any(|&(k, e)| e == KernelError::Unsupported && (k == 0) == main)
        };
        if unsupported(true) {
            return Attempt::End(End::Skipped("unsupported"));
        }
        if unsupported(false) {
            return Attempt::DropAlt;
        }
        let smaller = fails
            .iter()
            .filter_map(|&(_, e)| match e {
                KernelError::Memory(b) => Some(b),
                _ => None,
            })
            .min();
        if let Some(b) = smaller.filter(|&b| b > 0 && b < budget.ram_per_thread) {
            self.notice(pr, "ram_reduced", Some(b));
            return Attempt::Reduce(b);
        }
        let reducible = schedule::one_per_core(cpus).len() < cpus.len();
        if matches!(pr.spec.kernel, KernelId::K3 | KernelId::K4 | KernelId::K10) && reducible {
            return Attempt::OnePerCore;
        }
        self.notice(pr, "ram_insufficient", None);
        Attempt::End(End::Skipped("ram_insufficient"))
    }

    fn attempt(
        &self,
        cur: &mut Cursor,
        pr: &PhaseRun,
        memo: &mut Memo,
        cpus: &[LogicalCpu],
        budget: ThreadBudget,
        deadline: Instant,
    ) -> Attempt {
        let kernels = &pr.kernels[..memo.kernels];
        let shared = Arc::new(PhaseShared::default());
        let ctx = WorkerCtx {
            isa: pr.spec.isa,
            size: pr.spec.size,
            budget,
            seed: pr.seed,
            worker: 0,
            workers: cpus.len() as u32,
            patterns: pr.spec.patterns.clone(),
            shared: Arc::clone(&shared),
        };
        let refs = match self.references(cur, pr, kernels, memo, cpus, &ctx) {
            Ok(refs) => refs,
            Err(attempt) => return attempt,
        };
        let slots: Vec<Arc<Slot>> = cpus
            .iter()
            .map(|cpu| {
                Arc::new(Slot {
                    cpu: cpu.clone(),
                    beat: AtomicU64::new(0),
                    iterations: AtomicU64::new(0),
                    errors_sent: AtomicU32::new(0),
                })
            })
            .collect();
        let live = Live {
            shared: Arc::clone(&shared),
            failed: AtomicBool::new(false),
            first_error: AtomicBool::new(false),
            inject_worker: self.inject_worker(kernels, cpus),
        };
        let gate = Gate::default();
        let (tx, rx) = mpsc::channel();
        thread::scope(|s| {
            let _abort = AbortOnDrop(&gate, &shared);
            for (i, slot) in slots.iter().enumerate() {
                let (tx, live, gate, refs) = (tx.clone(), &live, &gate, &refs);
                let mut ctx = ctx.clone();
                ctx.worker = i as u32;
                s.spawn(move || self.worker(pr, kernels, live, i, slot, refs, ctx, tx, gate));
            }
            drop(tx);
            // Every worker sends once, then drops its sender.
            let mut built = Vec::with_capacity(cpus.len());
            let waited = self.wait(cur, |timeout| match rx.recv_timeout(timeout) {
                Ok(r) => {
                    built.push(r);
                    built.len() == cpus.len()
                }
                Err(mpsc::RecvTimeoutError::Timeout) => false,
                Err(mpsc::RecvTimeoutError::Disconnected) => true,
            });
            if let Err(end) = waited {
                return Attempt::End(end);
            }
            if built.len() < cpus.len() || built.contains(&Err(BuildFail::Panic)) {
                return Attempt::End(End::Skipped("kernel_panic"));
            }
            let fails: Vec<(usize, KernelError)> = built
                .iter()
                .filter_map(|r| match r {
                    Err(BuildFail::Kernel(k, e)) => Some((*k, *e)),
                    _ => None,
                })
                .collect();
            if !fails.is_empty() {
                return self.decide(pr, &fails, cpus, budget);
            }
            gate.set(true);
            self.publish(pr, &slots);
            if matches!(pr.spec.kernel, KernelId::K3 | KernelId::K4 | KernelId::K10) {
                cur.memory_bytes = budget.ram_per_thread * cpus.len() as u64;
            }
            let end = self.monitor(cur, pr, &live, deadline);
            self.unpublish();
            cur.memory_bytes = 0;
            Attempt::End(end)
        })
    }

    /// Polls until `poll` (which waits up to the time it is given) says it is done, sending
    /// `Progress` once a second; a stop, a hung or a crashed worker end it early.
    fn wait(&self, cur: &mut Cursor, mut poll: impl FnMut(Duration) -> bool) -> Result<(), End> {
        loop {
            if self.hung.load(Ordering::Relaxed) || self.crashed.load(Ordering::Relaxed) {
                return Err(End::Aborted);
            }
            if self.stop.load(Ordering::Relaxed) {
                return Err(End::Stopped);
            }
            if poll(POLL) {
                return Ok(());
            }
            if cur.sent_at.elapsed() >= PROGRESS_EVERY {
                self.progress(cur);
            }
        }
    }

    /// The worker of the injected fault in this set (DA18): the first one on the chosen
    /// core, or the first one when no core is given.
    fn inject_worker(
        &self,
        kernels: &[(KernelId, &dyn KernelFactory)],
        cpus: &[LogicalCpu],
    ) -> Option<usize> {
        let inject = self
            .inject
            .as_ref()
            .filter(|i| kernels.iter().any(|&(id, _)| id == i.kernel))?;
        match inject.core {
            Some(core) => cpus.iter().position(|c| c.core == core),
            None => Some(0),
        }
    }

    /// The references of the set's kernels (DA7), computed again only when the budget
    /// changed. They run on a helper thread while this one keeps polling; a stop ends them
    /// when the current reference returns.
    fn references(
        &self,
        cur: &mut Cursor,
        pr: &PhaseRun,
        kernels: &[(KernelId, &dyn KernelFactory)],
        memo: &mut Memo,
        cpus: &[LogicalCpu],
        ctx: &WorkerCtx,
    ) -> Result<Refs, Attempt> {
        if let Some((budget, refs)) = &memo.refs {
            if *budget == ctx.budget {
                return Ok(refs.clone());
            }
        }
        let rctx = WorkerCtx {
            worker: 0,
            shared: Arc::new(PhaseShared::default()),
            ..ctx.clone()
        };
        let mut refs = Refs::new();
        for (k, &(id, factory)) in kernels.iter().enumerate() {
            let self_checking = AtomicBool::new(false);
            let f = || -> Result<Vec<u64>, RefStop> {
                if self.stop.load(Ordering::Relaxed) {
                    return Err(RefStop::Stopped);
                }
                match factory.reference(&rctx) {
                    Some(r) => r.map_err(RefStop::Failure),
                    None => {
                        self_checking.store(true, Ordering::Relaxed);
                        Ok(Vec::new())
                    }
                }
            };
            let (waited, result) = thread::scope(|s| {
                let helper = s.spawn(|| verify::reference_on(&self.ref_cpus, &f));
                let waited = self.wait(cur, |timeout| {
                    helper.is_finished() || {
                        thread::sleep(timeout);
                        false
                    }
                });
                // On a stop the closure ends the helper at its next reference.
                let result = helper.join().unwrap_or(Err(RefError::Panicked));
                (waited, result)
            });
            if let Err(end) = waited {
                return Err(Attempt::End(end));
            }
            let (kind, expected, actual, reason) = match result {
                Ok(_) if self_checking.load(Ordering::Relaxed) => {
                    refs.push(None);
                    continue;
                }
                Ok(digests) => {
                    refs.push(Some(digests));
                    continue;
                }
                Err(RefError::Failed(RefStop::Stopped)) => return Err(Attempt::End(End::Stopped)),
                Err(RefError::Failed(RefStop::Failure(RefFailure::Kernel(e)))) => {
                    return Err(self.decide(pr, &[(k, e)], cpus, ctx.budget));
                }
                Err(RefError::Panicked) => {
                    tracing::error!(kernel = ?id, "the reference panicked");
                    return Err(Attempt::End(End::Skipped("kernel_panic")));
                }
                Err(RefError::Disagree(values)) => {
                    let (expected, actual) = first_difference(&values);
                    (
                        ErrorKind::ReferenceDisagreement,
                        expected,
                        actual,
                        "reference_disagreement",
                    )
                }
                Err(RefError::Failed(RefStop::Failure(RefFailure::Invalid(message)))) => {
                    tracing::error!(kernel = ?id, %message, "the reference is invalid");
                    (ErrorKind::ReferenceInvalid, 0, 0, "reference_invalid")
                }
            };
            if kind != ErrorKind::ReferenceInvalid {
                self.errors.fetch_add(1, Ordering::Relaxed);
            }
            self.send(self.error(pr, kind, id, None, 0, expected, actual));
            self.progress(cur);
            let first_error = kind == ErrorKind::ReferenceDisagreement && pr.spec.stop_on_error;
            return Err(Attempt::End(if first_error {
                End::FirstError
            } else {
                End::Skipped(reason)
            }));
        }
        memo.refs = Some((ctx.budget, refs.clone()));
        Ok(refs)
    }

    /// Watches a running set until it ends, sending `Progress` once a second and, at most
    /// four times a second, after new errors.
    fn monitor(&self, cur: &mut Cursor, pr: &PhaseRun, live: &Live, deadline: Instant) -> End {
        let mut errors = self.errors.load(Ordering::Relaxed);
        loop {
            if self.hung.load(Ordering::Relaxed) || self.crashed.load(Ordering::Relaxed) {
                return End::Aborted;
            }
            if self.stop.load(Ordering::Relaxed) {
                return End::Stopped;
            }
            if live.first_error.load(Ordering::Relaxed) {
                return End::FirstError;
            }
            if pr.spec.placement == Placement::CoreCycle && live.failed.load(Ordering::Relaxed) {
                return End::CoreFailed;
            }
            let now = Instant::now();
            if now >= deadline {
                return End::Done;
            }
            let e = self.errors.load(Ordering::Relaxed);
            let since = now - cur.sent_at;
            if (e != errors && since >= PROGRESS_AFTER_ERROR) || since >= PROGRESS_EVERY {
                errors = e;
                self.progress(cur);
            }
            thread::sleep(POLL.min(deadline - now));
        }
    }

    fn progress(&self, cur: &mut Cursor) {
        let now = Instant::now();
        let iterations = self.iterations.load(Ordering::Relaxed);
        let dt = (now - cur.sent_at).as_secs_f64();
        let rate = (dt > 0.0).then(|| (iterations - cur.rate_iterations) as f64 / dt);
        cur.sent_at = now;
        cur.rate_iterations = iterations;
        let failed = lock(&self.failed_cores).clone();
        let cores = cur
            .cores
            .iter()
            .map(|c| CoreProgress {
                core: c.core,
                state: if failed.contains(&c.core) {
                    CoreState::Failed
                } else {
                    c.state
                },
            })
            .collect();
        self.send(LoadMessage::Progress(Progress {
            phase: cur.phase,
            phase_elapsed_ms: ms(now - cur.phase_start),
            elapsed_ms: ms(now - self.start),
            checks: self.checks.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            current_core: cur.current_core,
            cores,
            memory_bytes: cur.memory_bytes,
            rate,
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn error(
        &self,
        pr: &PhaseRun,
        kind: ErrorKind,
        kernel: KernelId,
        cpu: Option<&LogicalCpu>,
        iteration: u64,
        expected: u64,
        actual: u64,
    ) -> LoadMessage {
        LoadMessage::Error(ComputeError {
            phase: pr.index,
            kernel,
            isa: pr.spec.isa,
            kind,
            logical: cpu.map(|c| c.index),
            core: cpu.map(|c| c.core),
            iteration,
            expected,
            actual,
            seed: pr.seed,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn worker(
        &self,
        pr: &PhaseRun,
        kernels: &[(KernelId, &dyn KernelFactory)],
        live: &Live,
        i: usize,
        slot: &Slot,
        refs: &Refs,
        ctx: WorkerCtx,
        tx: mpsc::Sender<Result<(), BuildFail>>,
        gate: &Gate,
    ) {
        if let Err(e) = crate::sys::pin(&slot.cpu) {
            tracing::warn!(logical = slot.cpu.index, error = %e, "worker not pinned");
        }
        crate::sys::prepare_worker();
        let built: Result<Vec<Box<dyn Kernel>>, BuildFail> = kernels
            .iter()
            .enumerate()
            .map(|(k, &(id, factory))| {
                match panic::catch_unwind(AssertUnwindSafe(|| factory.worker(&ctx))) {
                    Ok(r) => r.map_err(|e| BuildFail::Kernel(k, e)),
                    Err(_) => {
                        tracing::error!(kernel = ?id, "the kernel panicked while starting");
                        Err(BuildFail::Panic)
                    }
                }
            })
            .collect();
        let _ = tx.send(built.as_ref().map(|_| ()).map_err(|e| *e));
        drop(tx);
        let Ok(mut built) = built else { return };
        if !gate.wait() {
            return;
        }
        let step = |k: usize, kernel: &mut dyn Kernel| {
            self.step(pr, kernels, live, i, slot, refs, k, kernel)
        };
        let run = panic::catch_unwind(AssertUnwindSafe(|| {
            modes::work(
                pr.spec.mode,
                pr.seed,
                &mut built,
                &slot.beat,
                &ctx.shared.quit,
                step,
            )
        }));
        if run.is_err() {
            tracing::error!(kernel = ?pr.spec.kernel, "a kernel panicked");
            self.crashed.store(true, Ordering::Relaxed);
        }
    }

    /// One iteration of `kernel` (the `k`-th of the phase) and its check.
    #[allow(clippy::too_many_arguments)]
    fn step(
        &self,
        pr: &PhaseRun,
        kernels: &[(KernelId, &dyn KernelFactory)],
        live: &Live,
        i: usize,
        slot: &Slot,
        refs: &Refs,
        k: usize,
        kernel: &mut dyn Kernel,
    ) {
        let check = kernel.iterate(&slot.beat);
        slot.beat.fetch_add(1, Ordering::Relaxed);
        let iteration = slot.iterations.fetch_add(1, Ordering::Relaxed) + 1;
        self.iterations.fetch_add(1, Ordering::Relaxed);
        let id = kernels[k].0;
        let check = match &self.inject {
            Some(inject)
                if inject.kernel == id
                    && live.inject_worker == Some(i)
                    && iteration >= INJECT_AT
                    && !pr.injected.swap(true, Ordering::Relaxed) =>
            {
                match check {
                    Check::Digest(d) => Check::Digest(d ^ 1),
                    Check::DigestOf { variant, digest } => Check::DigestOf {
                        variant,
                        digest: digest ^ 1,
                    },
                    Check::Ok => Check::Mismatch {
                        expected: 0,
                        actual: 1,
                    },
                    mismatch => mismatch,
                }
            }
            _ => check,
        };
        self.checks.fetch_add(1, Ordering::Relaxed);
        let compare = |variant: u32, digest: u64| {
            // No reference: a self-checking kernel, which has nothing to match.
            let reference = refs[k].as_ref()?;
            match reference.get(variant as usize) {
                Some(&r) => (r != digest).then_some((r, digest)),
                None => {
                    static WARNED: AtomicBool = AtomicBool::new(false);
                    if !WARNED.swap(true, Ordering::Relaxed) {
                        tracing::error!(kernel = ?id, variant, "no reference for this variant");
                    }
                    None
                }
            }
        };
        let wrong = match check {
            Check::Digest(d) => compare(0, d),
            Check::DigestOf { variant, digest } => compare(variant, digest),
            Check::Ok => None,
            Check::Mismatch { expected, actual } => Some((expected, actual)),
        };
        let Some((expected, actual)) = wrong else {
            return;
        };
        self.errors.fetch_add(1, Ordering::Relaxed);
        {
            let mut failed = lock(&self.failed_cores);
            if !failed.contains(&slot.cpu.core) {
                failed.push(slot.cpu.core);
            }
        }
        if slot.errors_sent.fetch_add(1, Ordering::Relaxed) < ERRORS_PER_WORKER {
            let msg = self.error(
                pr,
                ErrorKind::Mismatch,
                id,
                Some(&slot.cpu),
                iteration,
                expected,
                actual,
            );
            self.send(msg);
        }
        live.failed.store(true, Ordering::Relaxed);
        if pr.spec.stop_on_error {
            live.first_error.store(true, Ordering::Relaxed);
        }
        // The set ends at the first error: the whole run, or this core's slice.
        if pr.spec.stop_on_error || pr.spec.placement == Placement::CoreCycle {
            live.shared.quit.store(true, Ordering::Relaxed);
        }
    }

    fn publish(&self, pr: &PhaseRun, slots: &[Arc<Slot>]) {
        let mut w = lock(&self.watched);
        w.generation += 1;
        w.live = Some(Published {
            phase: pr.index,
            kernel: pr.spec.kernel,
            isa: pr.spec.isa,
            seed: pr.seed,
            slots: slots.to_vec(),
        });
    }

    fn unpublish(&self) {
        let mut w = lock(&self.watched);
        w.generation += 1;
        w.live = None;
    }

    fn beats(&self) -> (u64, Vec<u64>) {
        let w = lock(&self.watched);
        let beats = w.live.as_ref().map_or_else(Vec::new, |p| {
            p.slots
                .iter()
                .map(|s| s.beat.load(Ordering::Relaxed))
                .collect()
        });
        (w.generation, beats)
    }

    /// The sentinel found worker `i` hung: `Error { hung }` and `Finished { failed }`, the
    /// last messages of the run, then the end of the process.
    fn hung_worker(&self, i: usize) {
        {
            let w = lock(&self.watched);
            let Some(p) = &w.live else { return };
            let mut closed = lock(&self.closed);
            self.hung.store(true, Ordering::Relaxed);
            let slot = p.slots.get(i);
            tracing::error!(phase = p.phase, logical = ?slot.map(|s| s.cpu.index), "a worker is hung");
            (self.out)(LoadMessage::Error(ComputeError {
                phase: p.phase,
                kernel: p.kernel,
                isa: p.isa,
                kind: ErrorKind::Hung,
                logical: slot.map(|s| s.cpu.index),
                core: slot.map(|s| s.cpu.core),
                iteration: slot.map_or(0, |s| s.iterations.load(Ordering::Relaxed)),
                expected: 0,
                actual: 0,
                seed: p.seed,
            }));
            (self.out)(LoadMessage::Finished(self.finish(FinishReason::Failed)));
            *closed = true;
        }
        (self.hooks.on_hung)();
    }
}

/// Whether a phase sizes its data from the RAM share (DA9, DA10).
fn uses_ram(spec: &Phase) -> bool {
    matches!(spec.kernel, KernelId::K3 | KernelId::K4 | KernelId::K10) || spec.size == DataSize::Ram
}

/// The first entry where a vector of `values` differs from the first one: (expected,
/// actual), 0 for a missing entry.
fn first_difference(values: &[Vec<u64>]) -> (u64, u64) {
    let first = &values[0];
    values
        .iter()
        .find(|v| *v != first)
        .and_then(|other| {
            (0..first.len().max(other.len()))
                .map(|j| {
                    (
                        first.get(j).copied().unwrap_or(0),
                        other.get(j).copied().unwrap_or(0),
                    )
                })
                .find(|(a, b)| a != b)
        })
        .unwrap_or((0, 0))
}

#[cfg(test)]
mod tests;
