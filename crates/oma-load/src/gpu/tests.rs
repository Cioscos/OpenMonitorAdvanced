//! The GPU phase engine with a fake submission queue and fake loads (at most 3 s each), and
//! one short end-to-end run on the real GPU.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use oma_ipc::load::{
    ComputeError, DataSize, ErrorKind, FinishReason, GpuTarget, Isa, KernelId, LoadMessage,
    LoadMode, Notice, Phase, PhaseDone, Placement, Plan, Progress,
};
use windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext;

use super::device::GpuError;
use super::engine::{
    run_gpu, run_gpu_with, GpuCheck, GpuMismatch, GpuRunEnd, GpuWorkload, Hooks, PhaseCtx,
    WorkloadResult,
};
use super::submit::Submit;
use crate::args::Inject;
use crate::link::{EXIT_DEVICE_LOST, EXIT_OK};
use crate::rng::phase_seed;

/// A clock that runs `speed` times faster than the real one, so a 72 s `pause_resume`
/// cycle fits in a short test.
#[derive(Clone, Copy)]
struct Clock {
    base: Instant,
    speed: u32,
}

impl Clock {
    fn now(&self) -> Instant {
        self.base + (Instant::now() - self.base) * self.speed
    }

    fn since(&self) -> Duration {
        self.now() - self.base
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Ev {
    Ctx(KernelId, Option<Inject>),
    Prepare(KernelId),
    /// The end of a submission, on the test clock.
    Submit(KernelId, Duration),
    Finish(Duration),
}

type Log = Arc<Mutex<Vec<Ev>>>;

fn push(log: &Log, ev: Ev) {
    log.lock().unwrap().push(ev);
}

/// A GPU that takes `busy` (real time) for every submission.
struct FakeSub {
    log: Log,
    clock: Clock,
    busy: Duration,
    fail_at: Option<(u64, GpuError)>,
    submitted: u64,
}

impl Submit for FakeSub {
    fn submit(&mut self, _: &mut dyn FnMut(&ID3D11DeviceContext)) -> Result<(), GpuError> {
        self.submitted += 1;
        if let Some((n, e)) = self.fail_at {
            if self.submitted >= n {
                return Err(e);
            }
        }
        thread::sleep(self.busy);
        Ok(())
    }
    fn finish(&mut self) -> Result<(), GpuError> {
        push(&self.log, Ev::Finish(self.clock.since()));
        Ok(())
    }
    fn gpu_ms(&mut self, _: &mut dyn FnMut(&ID3D11DeviceContext)) -> Result<f64, GpuError> {
        Ok(1.0)
    }
}

struct FakeLoad {
    kernel: KernelId,
    log: Log,
    clock: Clock,
    prepare_err: Option<GpuError>,
    /// Real time `prepare` takes.
    prepare_time: Duration,
    /// Given by the `n`-th check (from 1).
    mismatch: Option<(u32, GpuMismatch)>,
    checks_done: u32,
    since_check: u64,
    /// Given by every check.
    notices: Vec<(String, u64)>,
}

impl GpuWorkload for FakeLoad {
    fn prepare(&mut self, _: &mut dyn Submit, _: f64, stop: &AtomicBool) -> Result<(), GpuError> {
        push(&self.log, Ev::Prepare(self.kernel));
        thread::sleep(self.prepare_time);
        if self.prepare_err == Some(GpuError::Stopped) {
            // A long preparation that looks at the stop every 10 ms.
            while !stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(10));
            }
        }
        self.prepare_err.map_or(Ok(()), Err)
    }
    fn submit(&mut self, sub: &mut dyn Submit) -> Result<(), GpuError> {
        sub.submit(&mut |_| {})?;
        self.since_check += 1;
        push(&self.log, Ev::Submit(self.kernel, self.clock.since()));
        Ok(())
    }
    fn check(&mut self, _: &mut dyn Submit) -> Result<GpuCheck, GpuError> {
        self.checks_done += 1;
        let mismatches = match self.mismatch {
            Some((n, m)) if n == self.checks_done => vec![m],
            _ => vec![],
        };
        Ok(GpuCheck {
            checks: std::mem::take(&mut self.since_check),
            mismatches,
            notices: self.notices.clone(),
        })
    }
}

struct Setup {
    busy: Duration,
    speed: u32,
    fail_at: Option<(u64, GpuError)>,
    open_err: Option<GpuError>,
    supported: Vec<KernelId>,
    prepare_err: Option<(KernelId, GpuError)>,
    prepare_time: Duration,
    mismatch: Option<(u32, GpuMismatch)>,
    inject: Option<Inject>,
    notices: Vec<(String, u64)>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            busy: Duration::from_millis(5),
            speed: 1,
            fail_at: None,
            open_err: None,
            supported: vec![KernelId::S1, KernelId::S2, KernelId::S5],
            prepare_err: None,
            prepare_time: Duration::ZERO,
            mismatch: None,
            inject: None,
            notices: vec![],
        }
    }
}

fn phase(kernel: KernelId, mode: LoadMode, duration_s: u32) -> Phase {
    Phase {
        kernel,
        alt_kernel: None,
        isa: Isa::Sse2,
        size: DataSize::Auto,
        mode,
        placement: Placement::AllLogical,
        duration_s,
        per_core_s: None,
        both_smt: false,
        cores: None,
        patterns: vec![],
        stop_on_error: false,
        iterations: None,
        pause_before_ms: 0,
        windows: None,
    }
}

fn plan(phases: Vec<Phase>) -> Plan {
    Plan {
        seed: 7,
        ram_bytes: 0,
        phases,
        gpu: Some(GpuTarget {
            luid: 0x1234,
            integrated: false,
        }),
    }
}

struct Ran {
    end: GpuRunEnd,
    msgs: Vec<LoadMessage>,
    /// When each of `msgs` arrived, in real time.
    arrived: Vec<Instant>,
    events: Vec<Ev>,
}

impl Ran {
    fn progress(&self, phase: u32) -> Vec<&Progress> {
        self.msgs
            .iter()
            .filter_map(|m| match m {
                LoadMessage::Progress(p) if p.phase == phase => Some(p),
                _ => None,
            })
            .collect()
    }

    fn done(&self) -> Vec<&PhaseDone> {
        self.msgs
            .iter()
            .filter_map(|m| match m {
                LoadMessage::PhaseDone(d) => Some(d),
                _ => None,
            })
            .collect()
    }

    fn errors(&self) -> Vec<&ComputeError> {
        self.msgs
            .iter()
            .filter_map(|m| match m {
                LoadMessage::Error(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    fn submits(&self) -> Vec<(KernelId, Duration)> {
        self.events
            .iter()
            .filter_map(|e| match e {
                Ev::Submit(k, t) => Some((*k, *t)),
                _ => None,
            })
            .collect()
    }

    /// Gaps between consecutive submission ends, in ms of the test clock.
    fn gaps_ms(&self) -> Vec<(f64, f64)> {
        let s = self.submits();
        s.windows(2)
            .map(|w| {
                (
                    w[0].1.as_secs_f64() * 1e3,
                    (w[1].1 - w[0].1).as_secs_f64() * 1e3,
                )
            })
            .collect()
    }
}

fn run_with_stop(plan: &Plan, setup: &Setup, stop: &AtomicBool) -> Ran {
    let log: Log = Arc::default();
    let clock = Clock {
        base: Instant::now(),
        speed: setup.speed,
    };
    let msgs = Mutex::new(Vec::new());
    let out = |m: LoadMessage| msgs.lock().unwrap().push((Instant::now(), m));
    let open = |luid: u64| -> Result<((), Box<dyn Submit>), GpuError> {
        assert_eq!(luid, 0x1234);
        if let Some(e) = setup.open_err {
            return Err(e);
        }
        Ok((
            (),
            Box::new(FakeSub {
                log: Arc::clone(&log),
                clock,
                busy: setup.busy,
                fail_at: setup.fail_at,
                submitted: 0,
            }),
        ))
    };
    let workload = |kernel: KernelId, _: &(), ctx: &PhaseCtx| -> WorkloadResult {
        push(&log, Ev::Ctx(kernel, ctx.inject.clone()));
        if !setup.supported.contains(&kernel) {
            return Ok(None);
        }
        Ok(Some(Box::new(FakeLoad {
            kernel,
            log: Arc::clone(&log),
            clock,
            prepare_err: setup
                .prepare_err
                .filter(|&(k, _)| k == kernel)
                .map(|(_, e)| e),
            prepare_time: setup.prepare_time,
            mismatch: setup.mismatch.filter(|_| kernel == KernelId::S1),
            checks_done: 0,
            since_check: 0,
            notices: setup.notices.clone(),
        })))
    };
    let now = || clock.now();
    let hooks = Hooks {
        open: &open,
        budget: &|_: &()| Default::default(),
        workload: &workload,
        clock: &now,
    };
    let end = run_gpu_with(plan, &out, stop, setup.inject.clone(), &hooks);
    let events = log.lock().unwrap().clone();
    let (arrived, msgs) = msgs.into_inner().unwrap().into_iter().unzip();
    Ran {
        end,
        msgs,
        arrived,
        events,
    }
}

fn run(plan: &Plan, setup: &Setup) -> Ran {
    run_with_stop(plan, setup, &AtomicBool::new(false))
}

#[test]
fn phases_run_in_order_with_progress_every_second() {
    let ran = run(
        &plan(vec![
            phase(KernelId::S1, LoadMode::Steady, 2),
            phase(KernelId::S2, LoadMode::Steady, 1),
        ]),
        &Setup::default(),
    );
    assert_eq!(ran.end.finished.reason, FinishReason::Completed);
    assert_eq!(ran.end.exit_code, EXIT_OK);
    let done = ran.done();
    assert_eq!(done.iter().map(|d| d.phase).collect::<Vec<_>>(), [0, 1]);
    assert!(done.iter().all(|d| d.skipped.is_none() && d.checks > 0));
    assert!(done[0].duration_ms >= 2000 && done[0].duration_ms < 2400);
    // A start, one a second, an end: the second one at about 1 s.
    let p0 = ran.progress(0);
    assert!(p0.len() >= 3, "{p0:?}");
    assert!(p0.iter().any(|p| (900..1300).contains(&p.phase_elapsed_ms)));
    // Everything of phase 0 comes before phase 1.
    let first_of_1 = ran
        .msgs
        .iter()
        .position(|m| matches!(m, LoadMessage::Progress(p) if p.phase == 1))
        .unwrap();
    let done_0 = ran
        .msgs
        .iter()
        .position(|m| matches!(m, LoadMessage::PhaseDone(d) if d.phase == 0))
        .unwrap();
    assert!(done_0 < first_of_1);
    let s = ran.submits();
    let last_s1 = s.iter().rposition(|&(k, _)| k == KernelId::S1).unwrap();
    assert!(s[last_s1 + 1..].iter().all(|&(k, _)| k == KernelId::S2));
    assert_eq!(
        ran.end.finished.checks,
        done.iter().map(|d| d.checks).sum::<u64>()
    );
    assert_eq!(ran.end.finished.checks, s.len() as u64);
}

#[test]
fn rate_counts_completed_submissions() {
    let ran = run(
        &plan(vec![phase(KernelId::S1, LoadMode::Steady, 2)]),
        &Setup {
            busy: Duration::from_millis(10),
            ..Setup::default()
        },
    );
    let p = ran.progress(0);
    let mut seen = 0;
    for w in p.windows(2) {
        let (a, b) = (w[0], w[1]);
        let dt = (b.elapsed_ms - a.elapsed_ms) as f64 / 1e3;
        let Some(rate) = b.rate else { continue };
        if dt < 0.5 {
            continue;
        }
        seen += 1;
        // Every submission is checked: the checks counted between two Progress are the
        // submissions completed in between.
        let done = (b.checks - a.checks) as f64;
        assert!(
            (rate * dt - done).abs() <= 2.0,
            "rate {rate} dt {dt} done {done}"
        );
        assert!((50.0..=110.0).contains(&rate), "rate {rate}");
    }
    assert!(seen >= 1, "{p:?}");
}

#[test]
fn ramp_reports_the_load_level() {
    let ran = run(
        &plan(vec![phase(KernelId::S1, LoadMode::Ramp, 2)]),
        &Setup {
            busy: Duration::from_millis(10),
            ..Setup::default()
        },
    );
    assert_eq!(ran.end.finished.reason, FinishReason::Completed);
    let levels: Vec<u8> = ran
        .progress(0)
        .iter()
        .filter_map(|p| p.load_percent)
        .collect();
    assert!(levels.len() >= 2, "{levels:?}");
    assert!(levels.windows(2).all(|w| w[0] <= w[1]), "{levels:?}");
    assert!(levels.iter().all(|l| (20..=100).contains(l) && l % 5 == 0));
    assert!(levels.iter().any(|l| (55..=65).contains(l)), "{levels:?}");
    // 20% at the start: 10 ms of work, then 40 ms idle; 100% at the end: back to back.
    let gaps = ran.gaps_ms();
    assert!(gaps[0].1 >= 40.0, "{gaps:?}");
    assert!(gaps.last().unwrap().1 < 25.0, "{gaps:?}");
    // The pause comes after the submissions in flight are done.
    let first_two: Vec<usize> = ran
        .events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Ev::Submit(..)))
        .map(|(i, _)| i)
        .take(2)
        .collect();
    assert!(ran.events[first_two[0]..first_two[1]]
        .iter()
        .any(|e| matches!(e, Ev::Finish(_))));
}

#[test]
fn alternate_switches_between_100_and_15() {
    let ran = run(
        &plan(vec![phase(KernelId::S1, LoadMode::Alternate, 2)]),
        &Setup {
            busy: Duration::from_millis(5),
            ..Setup::default()
        },
    );
    let levels: Vec<u8> = ran
        .progress(0)
        .iter()
        .filter_map(|p| p.load_percent)
        .collect();
    assert!(!levels.is_empty());
    assert!(levels.iter().all(|l| matches!(l, 15 | 100)), "{levels:?}");
    // At 100% the 5 ms submissions follow each other; at 15% each one waits ~28 ms more.
    let gaps: Vec<f64> = ran.gaps_ms().iter().map(|g| g.1).collect();
    assert!(gaps.iter().any(|&g| g < 12.0), "{gaps:?}");
    assert!(gaps.iter().any(|&g| g > 25.0), "{gaps:?}");
}

#[test]
fn pause_resume_stops_submitting_during_the_pause() {
    // 80 s of the test clock in 2 s: 60 s on, 12 s off, 8 s on.
    let ran = run(
        &plan(vec![phase(KernelId::S1, LoadMode::PauseResume, 80)]),
        &Setup {
            busy: Duration::from_millis(2),
            speed: 40,
            ..Setup::default()
        },
    );
    assert_eq!(ran.end.finished.reason, FinishReason::Completed);
    let secs: Vec<f64> = ran.submits().iter().map(|s| s.1.as_secs_f64()).collect();
    assert!(secs.iter().any(|&t| t < 60.0));
    assert!(secs.iter().any(|&t| t > 72.0));
    assert!(
        !secs.iter().any(|&t| t > 60.3 && t < 72.0),
        "a submission during the pause"
    );
    // The pause starts with the submissions in flight done.
    assert!(ran
        .events
        .iter()
        .any(|e| matches!(e, Ev::Finish(t) if (60.0..61.0).contains(&t.as_secs_f64()))));
    assert!(ran.progress(0).iter().all(|p| p.load_percent.is_none()));
}

#[test]
fn alt_kernel_alternates_submissions() {
    let mut p = phase(KernelId::S5, LoadMode::Steady, 1);
    p.alt_kernel = Some(KernelId::S1);
    let ran = run(&plan(vec![p]), &Setup::default());
    assert!(ran.events.contains(&Ev::Prepare(KernelId::S5)));
    assert!(ran.events.contains(&Ev::Prepare(KernelId::S1)));
    let kernels: Vec<KernelId> = ran.submits().iter().map(|s| s.0).collect();
    assert!(kernels.len() > 10);
    assert_eq!(kernels[0], KernelId::S5);
    assert!(kernels.windows(2).all(|w| w[0] != w[1]), "{kernels:?}");
}

fn mismatch() -> GpuMismatch {
    GpuMismatch {
        iteration: 7,
        expected: 0xAA,
        actual: 0xAB,
    }
}

#[test]
fn per_error_notices_are_capped_per_phase() {
    // 20 bad frames (S6) and 20 bad rounds (S4) in every check: at most 16 notices of each
    // per-error code per phase, while the other notices all go through.
    let per_check = 20;
    let mut notices = vec![];
    for code in ["artifact_tiles", "vram_bits", "vram_words", "vram_reduced"] {
        notices.extend(std::iter::repeat_n((code.to_owned(), 1), per_check));
    }
    let ran = run(
        &plan(vec![
            phase(KernelId::S1, LoadMode::Steady, 1),
            phase(KernelId::S2, LoadMode::Steady, 1),
        ]),
        &Setup {
            notices,
            ..Setup::default()
        },
    );
    for phase in 0..2 {
        let count = |code: &str| {
            ran.msgs
                .iter()
                .filter(
                    |m| matches!(m, LoadMessage::Notice(n) if n.phase == phase && n.code == code),
                )
                .count()
        };
        for code in ["artifact_tiles", "vram_bits", "vram_words"] {
            assert_eq!(count(code), 16, "{code} in phase {phase}");
        }
        assert_eq!(count("vram_reduced") % per_check, 0);
        assert!(count("vram_reduced") >= per_check);
    }
}

#[test]
fn mismatch_becomes_an_error_with_iteration() {
    let ran = run(
        &plan(vec![phase(KernelId::S1, LoadMode::Steady, 1)]),
        &Setup {
            mismatch: Some((1, mismatch())),
            ..Setup::default()
        },
    );
    assert_eq!(
        ran.errors(),
        [&ComputeError {
            phase: 0,
            kernel: KernelId::S1,
            isa: Isa::Sse2,
            kind: ErrorKind::Mismatch,
            logical: None,
            core: None,
            iteration: 7,
            expected: 0xAA,
            actual: 0xAB,
            seed: phase_seed(7, 0),
            load_percent: None,
        }]
    );
    assert_eq!(ran.done()[0].errors, 1);
    assert_eq!(ran.end.finished.errors, 1);
    assert_eq!(ran.end.finished.reason, FinishReason::Completed);
}

#[test]
fn stop_on_error_finishes_with_first_error() {
    let mut p = phase(KernelId::S1, LoadMode::Steady, 10);
    p.stop_on_error = true;
    let started = Instant::now();
    let ran = run(
        &plan(vec![p, phase(KernelId::S2, LoadMode::Steady, 1)]),
        &Setup {
            mismatch: Some((1, mismatch())),
            ..Setup::default()
        },
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(ran.end.finished.reason, FinishReason::FirstError);
    assert_eq!(ran.done().len(), 1);
    assert_eq!(ran.errors().len(), 1);
}

#[test]
fn stop_flag_finishes_within_200_ms() {
    // 50 ms submissions at 20%: 200 ms pauses, which the stop must cut short.
    let stop = AtomicBool::new(false);
    let stopped_at = Mutex::new(None);
    let ran = thread::scope(|s| {
        s.spawn(|| {
            thread::sleep(Duration::from_millis(300));
            *stopped_at.lock().unwrap() = Some(Instant::now());
            stop.store(true, Ordering::Relaxed);
        });
        run_with_stop(
            &plan(vec![phase(KernelId::S1, LoadMode::Ramp, 10)]),
            &Setup {
                busy: Duration::from_millis(50),
                ..Setup::default()
            },
            &stop,
        )
    });
    let took = stopped_at.lock().unwrap().unwrap().elapsed();
    assert!(took < Duration::from_millis(200), "{took:?}");
    assert_eq!(ran.end.finished.reason, FinishReason::Stopped);
    assert_eq!(ran.done().len(), 1);
}

#[test]
fn stop_during_prepare_finishes_within_200_ms() {
    let stop = AtomicBool::new(false);
    let stopped_at = Mutex::new(None);
    let ran = thread::scope(|s| {
        s.spawn(|| {
            thread::sleep(Duration::from_millis(300));
            *stopped_at.lock().unwrap() = Some(Instant::now());
            stop.store(true, Ordering::Relaxed);
        });
        run_with_stop(
            &plan(vec![phase(KernelId::S1, LoadMode::Steady, 10)]),
            &Setup {
                prepare_err: Some((KernelId::S1, GpuError::Stopped)),
                ..Setup::default()
            },
            &stop,
        )
    });
    let took = stopped_at.lock().unwrap().unwrap().elapsed();
    assert!(took < Duration::from_millis(200), "{took:?}");
    assert_eq!(ran.end.finished.reason, FinishReason::Stopped);
    assert_eq!(ran.done().len(), 1);
    assert_eq!(ran.done()[0].skipped, None);
    assert!(ran.submits().is_empty());
}

#[test]
fn device_lost_finishes_and_exits_with_4() {
    let ran = run(
        &plan(vec![phase(KernelId::S1, LoadMode::Steady, 2)]),
        &Setup {
            fail_at: Some((5, GpuError::Lost(0x887A_0006))),
            ..Setup::default()
        },
    );
    assert_eq!(ran.end.exit_code, EXIT_DEVICE_LOST);
    assert_eq!(ran.end.finished.reason, FinishReason::Failed);
    let errors = ran.errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, ErrorKind::DeviceLost);
    assert_eq!(errors[0].actual, 0x887A_0006);
    assert!(ran.done().is_empty());
}

#[test]
fn hung_submission_is_hung() {
    let ran = run(
        &plan(vec![phase(KernelId::S1, LoadMode::Steady, 2)]),
        &Setup {
            fail_at: Some((3, GpuError::Hung)),
            ..Setup::default()
        },
    );
    assert_eq!(ran.end.exit_code, EXIT_OK);
    assert_eq!(ran.end.finished.reason, FinishReason::Failed);
    let errors = ran.errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, ErrorKind::Hung);
    assert_eq!(errors[0].iteration, 2);
}

#[test]
fn unknown_luid_sends_gpu_missing() {
    let ran = run(
        &plan(vec![phase(KernelId::S1, LoadMode::Steady, 1)]),
        &Setup {
            open_err: Some(GpuError::NotFound),
            ..Setup::default()
        },
    );
    assert_eq!(
        ran.msgs,
        [LoadMessage::Notice(Notice {
            phase: 0,
            code: "gpu_missing".into(),
            value: None,
        })]
    );
    assert_eq!(ran.end.finished.reason, FinishReason::Failed);
    assert_eq!(ran.end.exit_code, EXIT_OK);
}

#[test]
fn unusable_gpu_sends_gpu_error() {
    let ran = run(
        &plan(vec![phase(KernelId::S1, LoadMode::Steady, 1)]),
        &Setup {
            open_err: Some(GpuError::Create(0x8000_4005_u32 as i32)),
            ..Setup::default()
        },
    );
    assert_eq!(
        ran.msgs,
        [LoadMessage::Notice(Notice {
            phase: 0,
            code: "gpu_error".into(),
            value: None,
        })]
    );
    assert_eq!(ran.end.finished.reason, FinishReason::Failed);
    assert_eq!(ran.end.exit_code, EXIT_OK);
}

#[test]
fn slow_prepare_still_sends_progress_every_second() {
    let ran = run(
        &plan(vec![phase(KernelId::S1, LoadMode::Steady, 1)]),
        &Setup {
            prepare_time: Duration::from_millis(2500),
            ..Setup::default()
        },
    );
    let times: Vec<Instant> = ran
        .msgs
        .iter()
        .zip(&ran.arrived)
        .filter(|(m, _)| matches!(m, LoadMessage::Progress(_)))
        .map(|(_, t)| *t)
        .collect();
    assert!(times.len() >= 4, "{} progress messages", times.len());
    for w in times.windows(2) {
        let gap = w[1] - w[0];
        assert!(gap <= Duration::from_secs(1), "{gap:?}");
    }
    // The calibration is not part of the rate.
    assert!(ran
        .progress(0)
        .iter()
        .skip(1)
        .rev()
        .skip(1)
        .all(|p| p.rate.is_none()));
}

#[test]
fn missing_workload_skips_the_phase() {
    let ran = run(
        &plan(vec![
            phase(KernelId::S4, LoadMode::Steady, 1),
            phase(KernelId::S1, LoadMode::Steady, 1),
        ]),
        &Setup::default(),
    );
    let done = ran.done();
    assert_eq!(done[0].skipped.as_deref(), Some("unsupported"));
    assert!(done[0].duration_ms < 100);
    assert_eq!(done[1].skipped, None);
    assert_eq!(ran.end.finished.reason, FinishReason::Completed);
}

#[test]
fn reference_invalid_skips_the_phase() {
    let ran = run(
        &plan(vec![
            phase(KernelId::S2, LoadMode::Steady, 1),
            phase(KernelId::S1, LoadMode::Steady, 1),
        ]),
        &Setup {
            prepare_err: Some((KernelId::S2, GpuError::ReferenceInvalid)),
            ..Setup::default()
        },
    );
    let errors = ran.errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, ErrorKind::ReferenceInvalid);
    assert_eq!(ran.done()[0].skipped.as_deref(), Some("reference_invalid"));
    assert_eq!(ran.done()[1].skipped, None);
    // Not an error of the GPU.
    assert_eq!(ran.end.finished.errors, 0);
}

#[test]
fn injected_fault_hits_s1() {
    #[cfg(debug_assertions)]
    {
        let args = crate::args::parse_args(&[
            "--pipe".to_owned(),
            format!("{}x", oma_ipc::load::LOAD_PIPE_PREFIX),
            "--inject-fault".to_owned(),
            "s1".to_owned(),
        ])
        .unwrap();
        assert_eq!(
            args.inject,
            Some(Inject {
                kernel: KernelId::S1,
                core: None
            })
        );
    }
    let inject = Inject {
        kernel: KernelId::S1,
        core: None,
    };
    let ran = run(
        &plan(vec![
            phase(KernelId::S2, LoadMode::Steady, 1),
            phase(KernelId::S1, LoadMode::Steady, 1),
        ]),
        &Setup {
            inject: Some(inject.clone()),
            ..Setup::default()
        },
    );
    assert!(ran.events.contains(&Ev::Ctx(KernelId::S2, None)));
    assert!(ran.events.contains(&Ev::Ctx(KernelId::S1, Some(inject))));
}

#[test]
#[ignore = "requires real Windows hardware"]
fn gpu_plan_runs_end_to_end() {
    let adapter = oma_win::gpu::stress_adapters()
        .into_iter()
        .next()
        .expect("no hardware GPU");
    let mut plan = plan(vec![phase(KernelId::S1, LoadMode::Steady, 2)]);
    plan.gpu = Some(GpuTarget {
        luid: adapter.luid,
        integrated: adapter.integrated,
    });
    let msgs = Mutex::new(Vec::new());
    let end = run_gpu(
        &plan,
        &|m| msgs.lock().unwrap().push(m),
        &AtomicBool::new(false),
        None,
    );
    let msgs = msgs.into_inner().unwrap();
    assert_eq!(end.finished.reason, FinishReason::Completed, "{msgs:?}");
    assert!(end.finished.checks >= 1);
    assert_eq!(end.finished.errors, 0, "{msgs:?}");
    assert_eq!(end.exit_code, EXIT_OK);
}
