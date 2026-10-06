//! The phase engine (§4.2–§4.4, DA7, DA9, DA10, DA18): runs the phases of a plan one after
//! the other on pinned worker threads, checks every iteration against the reference and
//! reports progress, errors and notices through `out`.
//!
//! A phase (or one slice of a `core_cycle` phase) runs as one *set* of workers:
//! 1. each worker pins its thread and builds its kernels there; a memory error rebuilds
//!    the whole set smaller (DA10);
//! 2. the engine computes the reference on up to three cores (DA7) while the workers wait;
//! 3. the workers run the load mode, and the engine thread watches the clock, the stop
//!    flag and the errors, and sends `Progress` once a second.
//!
//! A sentinel thread watches the workers' beats for the whole run.

mod modes;
mod schedule;
mod sentinel;

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use oma_ipc::load::{
    ComputeError, CoreProgress, CoreState, ErrorKind, FinishReason, Finished, Isa, KernelId,
    LoadMessage, LoadMode, LogicalCpu, Notice, Phase, PhaseDone, Placement, Plan, Progress,
    Topology,
};

use crate::args::Inject;
use crate::kernel::{
    Check, Kernel, KernelError, KernelFactory, PhaseShared, ThreadBudget, WorkerCtx,
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
}

/// How often the engine thread looks at the clock, the stop flag and the errors.
const POLL: Duration = Duration::from_millis(50);
const PROGRESS_EVERY: Duration = Duration::from_secs(1);
/// The iteration whose check the fault injection flips (DA18), counted from 1.
const INJECT_AT: u64 = 3;

/// The reference digest of each kernel of a set; `None` for a self-checking kernel.
type Refs = Vec<Option<u64>>;

/// Runs `plan` and returns how it ended; the caller sends the `Finished`. A hung worker
/// ends the process instead (after the sentinel sent its own `Finished`).
pub fn run(
    plan: &Plan,
    topology: &Topology,
    out: &(dyn Fn(LoadMessage) + Sync),
    stop: &AtomicBool,
    inject: Option<Inject>,
) -> Finished {
    let exit = || std::process::exit(crate::link::EXIT_OK);
    let hooks = Hooks {
        factory: &crate::kernel::factory,
        on_hung: &exit,
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
        hung: AtomicBool::new(false),
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
        let finished = engine.run_phases();
        done.store(true, Ordering::Release);
        sentinel.thread().unpark();
        finished
    })
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
    hung: AtomicBool,
    /// A kernel panicked.
    crashed: AtomicBool,
    watched: Mutex<Watched>,
}

/// One worker of a set.
struct Slot {
    cpu: LogicalCpu,
    beat: AtomicU64,
    iterations: AtomicU64,
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
    cores: Vec<CoreProgress>,
    memory_bytes: u64,
    next_progress: Instant,
    rate_at: Instant,
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
#[derive(Default)]
struct Memo {
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
    /// Rebuild the set with this many bytes per thread.
    Reduce(u64),
    /// Rebuild the set with one thread per physical core.
    OnePerCore,
}

/// Holds the workers between their kernels and the reference: `None` while undecided,
/// then `Some(None)` to abort or `Some(Some(refs))` to go.
#[derive(Default)]
struct Gate {
    decision: Mutex<Option<Option<Refs>>>,
    cv: Condvar,
}

impl Gate {
    fn set(&self, go: Option<Refs>) {
        let mut d = lock(&self.decision);
        if d.is_none() {
            *d = Some(go);
            self.cv.notify_all();
        }
    }

    fn wait(&self) -> Option<Refs> {
        let mut d = lock(&self.decision);
        while d.is_none() {
            d = self.cv.wait(d).unwrap_or_else(PoisonError::into_inner);
        }
        d.clone().flatten()
    }
}

/// Aborts the gate on every way out of a set, so no worker waits for good.
struct AbortOnDrop<'g>(&'g Gate);

impl Drop for AbortOnDrop<'_> {
    fn drop(&mut self) {
        self.0.set(None);
    }
}

impl Engine<'_, '_> {
    fn finish(&self, reason: FinishReason) -> Finished {
        Finished {
            reason,
            checks: self.checks.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
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
            next_progress: now,
            rate_at: now,
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
            (self.out)(LoadMessage::PhaseDone(PhaseDone {
                phase: index,
                checks: self.checks.load(Ordering::Relaxed) - checks,
                errors: self.errors.load(Ordering::Relaxed) - errors,
                duration_ms: ms(cur.phase_start.elapsed()),
                skipped: match end {
                    End::Skipped(reason) => Some(reason.to_owned()),
                    _ => None,
                },
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
        let mut memo = Memo::default();
        if spec.placement == Placement::CoreCycle {
            return self.core_cycle(cur, &pr, &mut memo, deadline);
        }
        let per_core = match spec.placement {
            Placement::OnePerCore => 1,
            _ => usize::MAX,
        };
        let mut cpus = schedule::phase_cpus(self.topology, &self.order, per_core);
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
    /// until `duration_s` elapses; a core with an error is marked and skipped from then on,
    /// in the later phases too.
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
        while Instant::now() < deadline {
            let Some(i) = schedule::next_core(&cores, from, |c| cur.state(c) == CoreState::Failed)
            else {
                break;
            };
            from = i + 1;
            let core = cores[i];
            let cpus = schedule::core_cpus(self.topology, core, both);
            let before = cur.state(core);
            cur.set_state(core, CoreState::Testing);
            cur.current_core = Some(core);
            let slice_deadline = (Instant::now() + per_core).min(deadline);
            let end = self.run_set(cur, pr, memo, cpus, slice_deadline);
            cur.current_core = None;
            let state = match end {
                End::Done => CoreState::Passed,
                End::CoreFailed | End::FirstError => CoreState::Failed,
                _ => before,
            };
            cur.set_state(core, state);
            self.progress(cur);
            match end {
                End::Done | End::CoreFailed => {}
                other => return other,
            }
        }
        End::Done
    }

    /// Runs one set on `cpus` until `deadline`, rebuilding it smaller when memory is short.
    fn run_set(
        &self,
        cur: &mut Cursor,
        pr: &PhaseRun,
        memo: &mut Memo,
        mut cpus: Vec<LogicalCpu>,
        deadline: Instant,
    ) -> End {
        loop {
            let mut budget = ThreadBudget::for_workers(self.topology, &cpus, self.plan.ram_bytes);
            if let Some(cap) = memo.ram_cap {
                budget.ram_per_thread = budget.ram_per_thread.min(cap);
            }
            match self.attempt(cur, pr, memo, &cpus, budget, deadline) {
                Attempt::End(end) => return end,
                Attempt::Reduce(bytes) => memo.ram_cap = Some(bytes),
                Attempt::OnePerCore => {
                    cpus = schedule::one_per_core(&cpus);
                    // Fewer threads share the quota: start again from the full share.
                    memo.ram_cap = None;
                }
            }
        }
    }

    fn notice(&self, pr: &PhaseRun, code: &str, value: Option<u64>) {
        (self.out)(LoadMessage::Notice(Notice {
            phase: pr.index,
            code: code.to_owned(),
            value,
        }));
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
        let slots: Vec<Arc<Slot>> = cpus
            .iter()
            .map(|cpu| {
                Arc::new(Slot {
                    cpu: cpu.clone(),
                    beat: AtomicU64::new(0),
                    iterations: AtomicU64::new(0),
                })
            })
            .collect();
        let live = Live {
            shared: Arc::clone(&shared),
            failed: AtomicBool::new(false),
            first_error: AtomicBool::new(false),
            inject_worker: self.inject_worker(pr, cpus),
        };
        let gate = Gate::default();
        let (tx, rx) = mpsc::channel();
        thread::scope(|s| {
            let _abort = AbortOnDrop(&gate);
            for (i, slot) in slots.iter().enumerate() {
                let (tx, live, gate) = (tx.clone(), &live, &gate);
                let mut ctx = ctx.clone();
                ctx.worker = i as u32;
                s.spawn(move || self.worker(pr, live, i, slot, ctx, tx, gate));
            }
            drop(tx);
            // Every worker sends once, then drops its sender.
            let created: Vec<Result<(), KernelError>> = rx.iter().collect();

            if created.len() < cpus.len() || created.contains(&Err(KernelError::Unsupported)) {
                return Attempt::End(End::Skipped("unsupported"));
            }
            let smaller = created
                .iter()
                .filter_map(|r| match r {
                    Err(KernelError::Memory(b)) => Some(*b),
                    _ => None,
                })
                .min();
            if let Some(b) = smaller.filter(|&b| b > 0 && b < budget.ram_per_thread) {
                self.notice(pr, "ram_reduced", Some(b));
                return Attempt::Reduce(b);
            }
            if created.iter().any(Result::is_err) {
                let reducible = schedule::one_per_core(cpus).len() < cpus.len();
                if matches!(pr.spec.kernel, KernelId::K3 | KernelId::K4) && reducible {
                    return Attempt::OnePerCore;
                }
                self.notice(pr, "ram_insufficient", None);
                return Attempt::End(End::Skipped("ram_insufficient"));
            }
            if self.stop.load(Ordering::Relaxed) {
                return Attempt::End(End::Stopped);
            }
            let refs = match self.references(cur, pr, memo, &ctx) {
                Ok(refs) => refs,
                Err(end) => return Attempt::End(end),
            };
            gate.set(Some(refs));
            self.publish(pr, &slots);
            if matches!(pr.spec.kernel, KernelId::K3 | KernelId::K4 | KernelId::K10) {
                cur.memory_bytes = budget.ram_per_thread * cpus.len() as u64;
            }
            let end = self.monitor(cur, pr, &live, deadline);
            self.unpublish();
            cur.memory_bytes = 0;
            shared.quit.store(true, Ordering::Relaxed);
            Attempt::End(end)
        })
    }

    /// The worker of the injected fault in this set (DA18): the first one on the chosen
    /// core, or the first one when no core is given.
    fn inject_worker(&self, pr: &PhaseRun, cpus: &[LogicalCpu]) -> Option<usize> {
        let inject = self
            .inject
            .as_ref()
            .filter(|i| pr.kernels.iter().any(|&(id, _)| id == i.kernel))?;
        match inject.core {
            Some(core) => cpus.iter().position(|c| c.core == core),
            None => Some(0),
        }
    }

    /// The references of the set's kernels (DA7), computed again only when the budget
    /// changed.
    fn references(
        &self,
        cur: &mut Cursor,
        pr: &PhaseRun,
        memo: &mut Memo,
        ctx: &WorkerCtx,
    ) -> Result<Refs, End> {
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
        for &(id, factory) in &pr.kernels {
            let self_checking = AtomicBool::new(false);
            let f = || match factory.reference(&rctx) {
                Some(r) => r,
                None => {
                    self_checking.store(true, Ordering::Relaxed);
                    Ok(0)
                }
            };
            let (kind, expected, actual, reason) = match verify::reference_on(&self.ref_cpus, &f) {
                Ok(_) if self_checking.load(Ordering::Relaxed) => {
                    refs.push(None);
                    continue;
                }
                Ok(digest) => {
                    refs.push(Some(digest));
                    continue;
                }
                Err(RefError::Disagree(values)) => {
                    let other = values.iter().copied().find(|&v| v != values[0]);
                    (
                        ErrorKind::ReferenceDisagreement,
                        values[0],
                        other.unwrap_or(values[0]),
                        "reference_disagreement",
                    )
                }
                Err(RefError::Invalid(message)) => {
                    tracing::error!(kernel = ?id, %message, "the reference is invalid");
                    (ErrorKind::ReferenceInvalid, 0, 0, "reference_invalid")
                }
            };
            self.report(pr, kind, id, None, 0, expected, actual);
            self.progress(cur);
            let first_error = kind == ErrorKind::ReferenceDisagreement && pr.spec.stop_on_error;
            return Err(if first_error {
                End::FirstError
            } else {
                End::Skipped(reason)
            });
        }
        memo.refs = Some((ctx.budget, refs.clone()));
        Ok(refs)
    }

    /// Watches a running set until it ends, sending `Progress` once a second and at once
    /// after an error.
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
            if e != errors || now >= cur.next_progress {
                errors = e;
                self.progress(cur);
            }
            thread::sleep(POLL.min(deadline - now));
        }
    }

    fn progress(&self, cur: &mut Cursor) {
        let now = Instant::now();
        let iterations = self.iterations.load(Ordering::Relaxed);
        let dt = (now - cur.rate_at).as_secs_f64();
        let rate = (dt > 0.0).then(|| (iterations - cur.rate_iterations) as f64 / dt);
        cur.rate_at = now;
        cur.rate_iterations = iterations;
        cur.next_progress = now + PROGRESS_EVERY;
        (self.out)(LoadMessage::Progress(Progress {
            phase: cur.phase,
            phase_elapsed_ms: ms(now - cur.phase_start),
            elapsed_ms: ms(now - self.start),
            checks: self.checks.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            current_core: cur.current_core,
            cores: cur.cores.clone(),
            memory_bytes: cur.memory_bytes,
            rate,
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn report(
        &self,
        pr: &PhaseRun,
        kind: ErrorKind,
        kernel: KernelId,
        cpu: Option<&LogicalCpu>,
        iteration: u64,
        expected: u64,
        actual: u64,
    ) {
        if kind != ErrorKind::ReferenceInvalid {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
        (self.out)(LoadMessage::Error(ComputeError {
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
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn worker(
        &self,
        pr: &PhaseRun,
        live: &Live,
        i: usize,
        slot: &Slot,
        ctx: WorkerCtx,
        tx: mpsc::Sender<Result<(), KernelError>>,
        gate: &Gate,
    ) {
        if let Err(e) = crate::sys::pin(&slot.cpu) {
            tracing::warn!(logical = slot.cpu.index, error = %e, "worker not pinned");
        }
        crate::sys::prepare_worker();
        let created: Result<Vec<Box<dyn Kernel>>, KernelError> = pr
            .kernels
            .iter()
            .map(|&(id, factory)| {
                panic::catch_unwind(AssertUnwindSafe(|| factory.worker(&ctx))).unwrap_or_else(
                    |_| {
                        tracing::error!(kernel = ?id, "the kernel panicked while starting");
                        Err(KernelError::Unsupported)
                    },
                )
            })
            .collect();
        let _ = tx.send(created.as_ref().map(|_| ()).map_err(|e| *e));
        drop(tx);
        let Ok(mut kernels) = created else { return };
        let Some(refs) = gate.wait() else { return };
        let step =
            |k: usize, kernel: &mut dyn Kernel| self.step(pr, live, i, slot, &refs, k, kernel);
        let run = panic::catch_unwind(AssertUnwindSafe(|| {
            modes::work(
                pr.spec.mode,
                pr.seed,
                &mut kernels,
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
        let id = pr.kernels[k].0;
        let check = match &self.inject {
            Some(inject)
                if inject.kernel == id
                    && live.inject_worker == Some(i)
                    && iteration >= INJECT_AT
                    && !pr.injected.swap(true, Ordering::Relaxed) =>
            {
                match check {
                    Check::Digest(d) => Check::Digest(d ^ 1),
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
        let wrong = match check {
            // A digest without a reference (a self-checking kernel) has nothing to match.
            Check::Digest(d) => refs[k].filter(|&r| r != d).map(|r| (r, d)),
            Check::Ok => None,
            Check::Mismatch { expected, actual } => Some((expected, actual)),
        };
        if let Some((expected, actual)) = wrong {
            self.report(
                pr,
                ErrorKind::Mismatch,
                id,
                Some(&slot.cpu),
                iteration,
                expected,
                actual,
            );
            live.failed.store(true, Ordering::Relaxed);
            if pr.spec.stop_on_error {
                live.first_error.store(true, Ordering::Relaxed);
            }
            // The set ends at the first error: the whole run, or this core's slice.
            if pr.spec.stop_on_error || pr.spec.placement == Placement::CoreCycle {
                live.shared.quit.store(true, Ordering::Relaxed);
            }
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

    /// The sentinel found worker `i` hung: `Error { hung }`, `Finished { failed }`, then
    /// the end of the process.
    fn hung_worker(&self, i: usize) {
        {
            let w = lock(&self.watched);
            let Some(p) = &w.live else { return };
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
        }
        (self.out)(LoadMessage::Finished(self.finish(FinishReason::Failed)));
        self.hung.store(true, Ordering::Relaxed);
        (self.hooks.on_hung)();
    }
}

#[cfg(test)]
mod tests;
