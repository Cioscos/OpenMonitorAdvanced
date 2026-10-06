//! Engine tests with fake kernels: at most 2 workers and under 3 s each.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use oma_ipc::load::{
    CacheSizes, ComputeError, CoreState, DataSize, ErrorKind, FinishReason, Isa, LoadMode,
    LogicalCpu, Phase, PhaseDone, Placement, Progress,
};

use super::*;
use crate::kernel::{Check, Kernel, KernelError, RefFailure, WorkerCtx};
use crate::rng::{phase_seed, Xoshiro256ss};

const REF: u64 = 0x5EED_5EED;
const MIB: u64 = 1 << 20;

fn cpu(index: u32, core: u32) -> LogicalCpu {
    LogicalCpu {
        index,
        group: 0,
        number: index as u8,
        core,
        core_index: core,
        efficiency_class: 0,
        llc: 0,
        parked: false,
        apic_id: None,
    }
}

/// `logical` processors, `per_core` on each core.
fn topology(logical: u32, per_core: u32) -> Topology {
    Topology {
        logical: (0..logical).map(|i| cpu(i, i / per_core)).collect(),
        caches: CacheSizes {
            l1d_bytes: 32 << 10,
            l2_bytes: MIB,
            l2_shared_by: per_core,
            l3_bytes: 32 * MIB,
            l3_total_bytes: 32 * MIB,
        },
        hypervisor: false,
        vendor: String::new(),
        brand: String::new(),
    }
}

fn phase(kernel: KernelId, placement: Placement, duration_s: u32) -> Phase {
    Phase {
        kernel,
        alt_kernel: None,
        isa: Isa::Sse2,
        size: DataSize::L2,
        mode: LoadMode::Steady,
        placement,
        duration_s,
        per_core_s: None,
        both_smt: false,
        cores: None,
        patterns: vec![],
        stop_on_error: false,
    }
}

fn plan(phases: Vec<Phase>) -> Plan {
    Plan {
        seed: 7,
        ram_bytes: 4 * MIB,
        phases,
    }
}

/// A deterministic kernel: the reference digest after about 1 ms, or a wrong one.
struct CountKernel {
    bad: bool,
    count: Arc<AtomicU64>,
}

impl Kernel for CountKernel {
    fn iterate(&mut self, beat: &AtomicU64) -> Check {
        thread::sleep(Duration::from_millis(1));
        beat.fetch_add(1, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        Check::Digest(if self.bad { REF ^ 0x10 } else { REF })
    }
}

/// Makes [`CountKernel`]s; those created with an index in `bad` give wrong digests.
struct CountFactory {
    created: AtomicU32,
    bad: Range<u32>,
    count: Arc<AtomicU64>,
}

impl CountFactory {
    fn new(bad: Range<u32>) -> Self {
        Self {
            created: AtomicU32::new(0),
            bad,
            count: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl KernelFactory for CountFactory {
    fn reference(&self, _: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        Some(Ok(vec![REF]))
    }

    fn worker(&self, _: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        let n = self.created.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(CountKernel {
            bad: self.bad.contains(&n),
            count: Arc::clone(&self.count),
        }))
    }
}

/// A kernel that blocks without beating until released.
struct StallKernel(Arc<AtomicBool>);

impl Kernel for StallKernel {
    fn iterate(&mut self, _: &AtomicU64) -> Check {
        while !self.0.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(10));
        }
        Check::Digest(REF)
    }
}

struct StallFactory(Arc<AtomicBool>);

impl KernelFactory for StallFactory {
    fn reference(&self, _: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        Some(Ok(vec![REF]))
    }

    fn worker(&self, _: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        Ok(Box::new(StallKernel(Arc::clone(&self.0))))
    }
}

/// Fails the allocation down to 1 MiB per thread, then has not enough memory. The
/// reference fails the same way above 2 MiB.
struct MemoryFactory;

impl KernelFactory for MemoryFactory {
    fn reference(&self, ctx: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        match ctx.budget.ram_per_thread {
            b if b > 2 * MIB => Some(Err(KernelError::Memory(b / 2).into())),
            _ => Some(Ok(vec![REF])),
        }
    }

    fn worker(&self, ctx: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        match ctx.budget.ram_per_thread {
            b if b > MIB => Err(KernelError::Memory(b / 2)),
            _ => Err(KernelError::Insufficient),
        }
    }
}

fn no_hang() {
    panic!("unexpected hung worker");
}

fn run_test(
    plan: &Plan,
    topology: &Topology,
    factory: &FactoryFn<'_>,
    inject: Option<Inject>,
    stop: &AtomicBool,
    on_hung: &(dyn Fn() + Sync),
) -> (Finished, Vec<LoadMessage>) {
    let log = Mutex::new(Vec::new());
    let out = |m: LoadMessage| log.lock().unwrap().push(m);
    let hooks = Hooks { factory, on_hung };
    let finished = run_with(plan, topology, &out, stop, inject, &hooks);
    (finished, log.into_inner().unwrap())
}

fn errors(msgs: &[LoadMessage]) -> Vec<&ComputeError> {
    msgs.iter()
        .filter_map(|m| match m {
            LoadMessage::Error(e) => Some(e),
            _ => None,
        })
        .collect()
}

fn done(msgs: &[LoadMessage]) -> Vec<&PhaseDone> {
    msgs.iter()
        .filter_map(|m| match m {
            LoadMessage::PhaseDone(d) => Some(d),
            _ => None,
        })
        .collect()
}

fn last_progress(msgs: &[LoadMessage]) -> &Progress {
    msgs.iter()
        .rev()
        .find_map(|m| match m {
            LoadMessage::Progress(p) => Some(p),
            _ => None,
        })
        .expect("a progress message")
}

#[test]
fn core_cycle_marks_failed_core_and_moves_on() {
    let f = CountFactory::new(0..1);
    let factory = |id: KernelId| (id == KernelId::K2).then_some(&f as &dyn KernelFactory);
    let mut p = phase(KernelId::K2, Placement::CoreCycle, 2);
    p.per_core_s = Some(1);
    let (fin, msgs) = run_test(
        &plan(vec![p]),
        &topology(2, 1),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    let errs = errors(&msgs);
    assert_eq!(errs.len(), 1, "core 0 is never tried again: {errs:?}");
    assert_eq!((errs[0].kind, errs[0].core), (ErrorKind::Mismatch, Some(0)));
    assert_eq!((errs[0].expected, errs[0].actual), (REF, REF ^ 0x10));
    let states: Vec<_> = last_progress(&msgs)
        .cores
        .iter()
        .map(|c| (c.core, c.state))
        .collect();
    assert_eq!(states, [(0, CoreState::Failed), (1, CoreState::Passed)]);
    assert!(
        f.created.load(Ordering::Relaxed) >= 3,
        "core 1 runs again after the wrap"
    );
    assert_eq!(done(&msgs).len(), 1);
    assert_eq!(done(&msgs)[0].errors, 1);
}

#[test]
fn stop_on_error_finishes_with_first_error() {
    let f = CountFactory::new(0..u32::MAX);
    let factory = |_: KernelId| Some(&f as &dyn KernelFactory);
    let mut first = phase(KernelId::K2, Placement::AllLogical, 2);
    first.stop_on_error = true;
    let second = phase(KernelId::K5, Placement::AllLogical, 2);
    let started = Instant::now();
    let (fin, msgs) = run_test(
        &plan(vec![first, second]),
        &topology(1, 1),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::FirstError);
    assert!(fin.errors >= 1);
    assert!(started.elapsed() < Duration::from_secs(1));
    let phases: Vec<u32> = done(&msgs).iter().map(|d| d.phase).collect();
    assert_eq!(phases, [0], "the second phase never starts");
}

#[test]
fn stop_flag_finishes_with_stopped_within_one_second() {
    let f = CountFactory::new(0..0);
    let factory = |_: KernelId| Some(&f as &dyn KernelFactory);
    let stop = AtomicBool::new(false);
    let (fin, stopped_at, ended) = thread::scope(|s| {
        let stopper = s.spawn(|| {
            thread::sleep(Duration::from_millis(300));
            stop.store(true, Ordering::Relaxed);
            Instant::now()
        });
        let (fin, _) = run_test(
            &plan(vec![phase(KernelId::K2, Placement::AllLogical, 60)]),
            &topology(2, 1),
            &factory,
            None,
            &stop,
            &no_hang,
        );
        let ended = Instant::now();
        (fin, stopper.join().unwrap(), ended)
    });
    assert_eq!(fin.reason, FinishReason::Stopped);
    assert!(fin.checks > 0);
    assert!(ended - stopped_at < Duration::from_secs(1));
}

#[test]
fn variable_mode_alternates_kernels() {
    // The first burst and pause of this seed end well within the 1 s phase.
    let mut rng = Xoshiro256ss::new(phase_seed(7, 0));
    let (busy, pause) = modes::durations(LoadMode::Variable, &mut rng);
    assert!(
        busy + pause < Duration::from_millis(800),
        "{busy:?} {pause:?}"
    );

    let (main, alt) = (CountFactory::new(0..0), CountFactory::new(0..0));
    let factory = |id: KernelId| match id {
        KernelId::K1 => Some(&main as &dyn KernelFactory),
        KernelId::K5 => Some(&alt as &dyn KernelFactory),
        _ => None,
    };
    let mut p = phase(KernelId::K1, Placement::OnePerCore, 1);
    p.mode = LoadMode::Variable;
    p.alt_kernel = Some(KernelId::K5);
    let (fin, msgs) = run_test(
        &plan(vec![p]),
        &topology(1, 1),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    assert!(errors(&msgs).is_empty());
    assert!(main.count.load(Ordering::Relaxed) > 0);
    assert!(alt.count.load(Ordering::Relaxed) > 0);
}

#[test]
fn injected_fault_hits_the_chosen_core() {
    let f = CountFactory::new(0..0);
    let factory = |_: KernelId| Some(&f as &dyn KernelFactory);
    let inject = Inject {
        kernel: KernelId::K2,
        core: Some(1),
    };
    let (fin, msgs) = run_test(
        &plan(vec![phase(KernelId::K2, Placement::AllLogical, 1)]),
        &topology(2, 1),
        &factory,
        Some(inject),
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    let errs = errors(&msgs);
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert_eq!((errs[0].core, errs[0].logical), (Some(1), Some(1)));
    assert_eq!(errs[0].iteration, 3);
    assert_eq!(errs[0].expected ^ errs[0].actual, 1, "bit 0 flipped");
}

#[test]
fn sentinel_reports_a_stalled_worker() {
    let release = Arc::new(AtomicBool::new(false));
    let f = StallFactory(Arc::clone(&release));
    let factory = |_: KernelId| Some(&f as &dyn KernelFactory);
    let hung = AtomicU32::new(0);
    let on_hung = || {
        hung.fetch_add(1, Ordering::Relaxed);
        release.store(true, Ordering::Relaxed);
    };
    let (fin, msgs) = run_test(
        &plan(vec![phase(KernelId::K2, Placement::AllLogical, 2)]),
        &topology(1, 1),
        &factory,
        None,
        &AtomicBool::new(false),
        &on_hung,
    );
    assert_eq!(fin.reason, FinishReason::Failed);
    assert_eq!(hung.load(Ordering::Relaxed), 1);
    let errs = errors(&msgs);
    assert_eq!(errs.len(), 1);
    assert_eq!((errs[0].kind, errs[0].core), (ErrorKind::Hung, Some(0)));
    let finished: Vec<_> = msgs
        .iter()
        .filter_map(|m| match m {
            LoadMessage::Finished(f) => Some(f.reason),
            _ => None,
        })
        .collect();
    assert_eq!(finished, [FinishReason::Failed]);
    assert!(
        matches!(msgs.last(), Some(LoadMessage::Finished(_))),
        "nothing after Finished"
    );
}

#[test]
fn missing_factory_skips_the_phase() {
    let factory = |_: KernelId| None;
    let (fin, msgs) = run_test(
        &plan(vec![phase(KernelId::K7, Placement::AllLogical, 60)]),
        &topology(1, 1),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    let d = done(&msgs);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].skipped.as_deref(), Some("unsupported"));
    assert!(matches!(msgs.first(), Some(LoadMessage::Progress(_))));
}

#[test]
fn ram_reduction_halves_then_one_per_core_then_skips() {
    let factory = |_: KernelId| Some(&MemoryFactory as &dyn KernelFactory);
    // One core with two threads: 2 MiB each, then one thread with 4 MiB.
    let (fin, msgs) = run_test(
        &plan(vec![phase(KernelId::K3, Placement::AllLogical, 60)]),
        &topology(2, 2),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    let notices: Vec<_> = msgs
        .iter()
        .filter_map(|m| match m {
            LoadMessage::Notice(n) => Some((n.code.as_str(), n.value)),
            _ => None,
        })
        .collect();
    assert_eq!(
        notices,
        [
            ("ram_reduced", Some(MIB)),
            ("ram_reduced", Some(2 * MIB)),
            ("ram_reduced", Some(MIB)),
            ("ram_insufficient", None),
        ]
    );
    assert_eq!(done(&msgs)[0].skipped.as_deref(), Some("ram_insufficient"));
}

#[test]
fn reference_disagreement_skips_the_phase() {
    struct Disagree(AtomicU32);
    impl KernelFactory for Disagree {
        fn reference(&self, _: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
            Some(Ok(vec![
                REF,
                u64::from(self.0.fetch_add(1, Ordering::Relaxed)),
            ]))
        }
        fn worker(&self, _: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
            Ok(Box::new(StallKernel(Arc::new(AtomicBool::new(true)))))
        }
    }
    let f = Disagree(AtomicU32::new(0));
    let factory = |_: KernelId| Some(&f as &dyn KernelFactory);
    let (fin, msgs) = run_test(
        &plan(vec![phase(KernelId::K2, Placement::AllLogical, 60)]),
        &topology(2, 1),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    let errs = errors(&msgs);
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, ErrorKind::ReferenceDisagreement);
    assert_eq!((errs[0].expected, errs[0].actual), (0, 1));
    assert_eq!(
        done(&msgs)[0].skipped.as_deref(),
        Some("reference_disagreement")
    );
    // A disagreement is no core's error.
    assert!(last_progress(&msgs)
        .cores
        .iter()
        .all(|c| c.state == CoreState::Untested));
}

/// The reference takes 700 ms on each processor.
struct SlowReference(AtomicU32);

impl KernelFactory for SlowReference {
    fn reference(&self, _: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        thread::sleep(Duration::from_millis(700));
        Some(Ok(vec![REF]))
    }

    fn worker(&self, _: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Err(KernelError::Unsupported)
    }
}

/// Runs `plan` and raises `stop` after `after`; returns the end and how long it took
/// from the stop.
fn run_and_stop(
    plan: &Plan,
    topology: &Topology,
    factory: &FactoryFn<'_>,
    after: Duration,
) -> (Finished, Vec<LoadMessage>, Duration) {
    let stop = AtomicBool::new(false);
    thread::scope(|s| {
        let stopper = s.spawn(|| {
            thread::sleep(after);
            stop.store(true, Ordering::Relaxed);
            Instant::now()
        });
        let (fin, msgs) = run_test(plan, topology, factory, None, &stop, &no_hang);
        let ended = Instant::now();
        (fin, msgs, ended - stopper.join().unwrap())
    })
}

#[test]
fn stop_during_a_slow_reference_finishes_within_one_second() {
    let f = SlowReference(AtomicU32::new(0));
    let factory = |_: KernelId| Some(&f as &dyn KernelFactory);
    let (fin, msgs, latency) = run_and_stop(
        &plan(vec![phase(KernelId::K2, Placement::OnePerCore, 60)]),
        &topology(3, 1),
        &factory,
        Duration::from_millis(1200),
    );
    assert_eq!(fin.reason, FinishReason::Stopped);
    assert!(latency < Duration::from_secs(1), "{latency:?}");
    let progress = msgs
        .iter()
        .filter(|m| matches!(m, LoadMessage::Progress(_)))
        .count();
    assert!(progress >= 2, "Progress keeps coming during the reference");
    assert_eq!(f.0.load(Ordering::Relaxed), 0, "no worker built");
}

#[test]
fn stop_during_core_cycle_finishes_within_one_second() {
    let f = CountFactory::new(0..0);
    let factory = |_: KernelId| Some(&f as &dyn KernelFactory);
    let mut p = phase(KernelId::K2, Placement::CoreCycle, 60);
    p.per_core_s = Some(30);
    let (fin, msgs, latency) = run_and_stop(
        &plan(vec![p]),
        &topology(2, 1),
        &factory,
        Duration::from_millis(300),
    );
    assert_eq!(fin.reason, FinishReason::Stopped);
    assert!(latency < Duration::from_secs(1), "{latency:?}");
    let states: Vec<_> = last_progress(&msgs).cores.iter().map(|c| c.state).collect();
    assert_eq!(states, [CoreState::Untested, CoreState::Untested]);
}

#[test]
fn errors_are_capped_per_worker_but_all_counted() {
    let f = CountFactory::new(0..u32::MAX);
    let factory = |_: KernelId| Some(&f as &dyn KernelFactory);
    let (fin, msgs) = run_test(
        &plan(vec![phase(KernelId::K2, Placement::AllLogical, 1)]),
        &topology(1, 1),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    assert_eq!(errors(&msgs).len(), 16);
    assert!(fin.errors > 16, "{}", fin.errors);
    assert_eq!(done(&msgs)[0].errors, fin.errors);
    let progress = msgs
        .iter()
        .filter(|m| matches!(m, LoadMessage::Progress(_)))
        .count();
    assert!(progress <= 7, "at most 4 Hz after errors: {progress}");
    // An all-core error marks the core too (DA11).
    assert_eq!(last_progress(&msgs).cores[0].state, CoreState::Failed);
}

#[test]
fn a_core_failed_in_an_all_core_phase_is_skipped_by_core_cycle() {
    let f = CountFactory::new(0..1);
    let factory = |_: KernelId| Some(&f as &dyn KernelFactory);
    let mut cycle = phase(KernelId::K2, Placement::CoreCycle, 1);
    cycle.per_core_s = Some(1);
    // One worker on the only core fails; the cycle then has no core left.
    let first = phase(KernelId::K2, Placement::OnePerCore, 1);
    let (fin, msgs) = run_test(
        &plan(vec![first, cycle]),
        &topology(1, 1),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    let d = done(&msgs);
    assert_eq!(d.len(), 2);
    assert_eq!(
        f.created.load(Ordering::Relaxed),
        1,
        "core 0 is not tried again"
    );
}

#[test]
fn reference_invalid_skips_the_phase_and_is_not_counted() {
    struct Invalid;
    impl KernelFactory for Invalid {
        fn reference(&self, _: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
            Some(Err(RefFailure::Invalid("sum off".into())))
        }
        fn worker(&self, _: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
            Err(KernelError::Unsupported)
        }
    }
    let factory = |_: KernelId| Some(&Invalid as &dyn KernelFactory);
    let (fin, msgs) = run_test(
        &plan(vec![phase(KernelId::K2, Placement::AllLogical, 60)]),
        &topology(1, 1),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!((fin.reason, fin.errors), (FinishReason::Completed, 0));
    assert_eq!(errors(&msgs)[0].kind, ErrorKind::ReferenceInvalid);
    assert_eq!(done(&msgs)[0].skipped.as_deref(), Some("reference_invalid"));
}

#[test]
fn light_mode_runs_one_thread_and_both_smt_runs_two() {
    let f = CountFactory::new(0..0);
    let factory = |_: KernelId| Some(&f as &dyn KernelFactory);
    let mut light = phase(KernelId::K2, Placement::AllLogical, 1);
    light.mode = LoadMode::Light;
    let (fin, _) = run_test(
        &plan(vec![light]),
        &topology(2, 1),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    assert_eq!(f.created.load(Ordering::Relaxed), 1);

    let g = CountFactory::new(0..0);
    let factory = |_: KernelId| Some(&g as &dyn KernelFactory);
    let mut both = phase(KernelId::K2, Placement::CoreCycle, 1);
    both.per_core_s = Some(1);
    both.both_smt = true;
    let (fin, msgs) = run_test(
        &plan(vec![both]),
        &topology(2, 2),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    assert_eq!(
        g.created.load(Ordering::Relaxed),
        2,
        "both threads of core 0"
    );
    assert_eq!(last_progress(&msgs).cores[0].state, CoreState::Passed);
}

#[test]
fn digest_of_is_compared_with_its_variant() {
    /// Alternates the two variants; the second is wrong when `bad`.
    struct Sizes {
        n: u32,
        bad: bool,
    }
    impl Kernel for Sizes {
        fn iterate(&mut self, beat: &AtomicU64) -> Check {
            thread::sleep(Duration::from_millis(1));
            beat.fetch_add(1, Ordering::Relaxed);
            self.n += 1;
            let variant = self.n % 2;
            let digest = REF + u64::from(variant) + u64::from(self.bad && variant == 1);
            Check::DigestOf { variant, digest }
        }
    }
    struct SizesFactory(bool);
    impl KernelFactory for SizesFactory {
        fn reference(&self, _: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
            Some(Ok(vec![REF, REF + 1]))
        }
        fn worker(&self, _: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
            Ok(Box::new(Sizes { n: 0, bad: self.0 }))
        }
    }
    for bad in [false, true] {
        let f = SizesFactory(bad);
        let factory = |_: KernelId| Some(&f as &dyn KernelFactory);
        let mut p = phase(KernelId::K4, Placement::AllLogical, 1);
        p.stop_on_error = true;
        let (fin, msgs) = run_test(
            &plan(vec![p]),
            &topology(1, 1),
            &factory,
            None,
            &AtomicBool::new(false),
            &no_hang,
        );
        if bad {
            assert_eq!(fin.reason, FinishReason::FirstError);
            let e = errors(&msgs)[0];
            assert_eq!((e.expected, e.actual), (REF + 1, REF + 2));
        } else {
            assert_eq!((fin.reason, fin.errors), (FinishReason::Completed, 0));
        }
    }
}

#[test]
fn unsupported_alt_kernel_leaves_the_main_one_and_a_panic_skips() {
    struct NoAlt;
    impl KernelFactory for NoAlt {
        fn reference(&self, _: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
            Some(Ok(vec![REF]))
        }
        fn worker(&self, _: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
            Err(KernelError::Unsupported)
        }
    }
    let main = CountFactory::new(0..0);
    let factory = |id: KernelId| match id {
        KernelId::K1 => Some(&main as &dyn KernelFactory),
        _ => Some(&NoAlt as &dyn KernelFactory),
    };
    let mut p = phase(KernelId::K1, Placement::OnePerCore, 1);
    p.mode = LoadMode::Variable;
    p.alt_kernel = Some(KernelId::K5);
    let (fin, msgs) = run_test(
        &plan(vec![p]),
        &topology(1, 1),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    assert_eq!(done(&msgs)[0].skipped, None);
    assert!(main.count.load(Ordering::Relaxed) > 0);

    struct Panics;
    impl KernelFactory for Panics {
        fn reference(&self, _: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
            Some(Ok(vec![REF]))
        }
        fn worker(&self, _: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
            panic!("kernel bug while building")
        }
    }
    let factory = |_: KernelId| Some(&Panics as &dyn KernelFactory);
    let (fin, msgs) = run_test(
        &plan(vec![phase(KernelId::K2, Placement::OnePerCore, 60)]),
        &topology(1, 1),
        &factory,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    assert_eq!(done(&msgs)[0].skipped.as_deref(), Some("kernel_panic"));
}

#[test]
fn k9_needs_two_workers() {
    let factory = CountFactory::new(0..0);
    let f = |_: KernelId| Some(&factory as &dyn KernelFactory);
    let (_, msgs) = run_test(
        &plan(vec![phase(KernelId::K9, Placement::AllLogical, 60)]),
        &topology(1, 1),
        &f,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert!(msgs
        .iter()
        .any(|m| matches!(m, LoadMessage::Notice(n) if n.code == "k9_needs_two_cores")));
    assert_eq!(
        done(&msgs)[0].skipped.as_deref(),
        Some("k9_needs_two_cores")
    );
}

#[test]
fn k9_with_three_workers_runs_two() {
    let factory = CountFactory::new(0..0);
    let f = |_: KernelId| Some(&factory as &dyn KernelFactory);
    let (fin, msgs) = run_test(
        &plan(vec![phase(KernelId::K9, Placement::AllLogical, 1)]),
        &topology(3, 1),
        &f,
        None,
        &AtomicBool::new(false),
        &no_hang,
    );
    assert_eq!(fin.reason, FinishReason::Completed);
    assert_eq!(factory.created.load(Ordering::Relaxed), 2);
    assert_eq!(done(&msgs)[0].skipped, None);
}

#[test]
fn cores_limit_all_logical_and_one_per_core() {
    // 4 cores of 2 threads; the phases name cores 1 and 2.
    for (placement, workers) in [(Placement::AllLogical, 4), (Placement::OnePerCore, 2)] {
        let f = CountFactory::new(0..0);
        let factory = |_: KernelId| Some(&f as &dyn KernelFactory);
        let mut p = phase(KernelId::K2, placement, 1);
        p.cores = Some(vec![1, 2]);
        let (fin, _) = run_test(
            &plan(vec![p]),
            &topology(8, 2),
            &factory,
            None,
            &AtomicBool::new(false),
            &no_hang,
        );
        assert_eq!(fin.reason, FinishReason::Completed);
        assert_eq!(f.created.load(Ordering::Relaxed), workers, "{placement:?}");
    }
}
