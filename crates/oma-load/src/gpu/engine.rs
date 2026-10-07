//! The GPU phase engine (plan DG4, DG5, DG7, DG10): opens the plan's adapter, then for each
//! phase builds its loads, calibrates them and submits until the phase ends, pacing the
//! submissions by the load mode. Once a second it reads the loads' checks and sends
//! `Progress`; a lost device or a hung submission ends the run.
//!
//! A phase with `windows` (GPU benchmark, DH4) submits as in `steady`, but every second is
//! a window timed on the GPU: after the warm-up, each window's rate goes to
//! `PhaseDone.rates`, and the phase ends with its windows or at `duration_s`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use oma_ipc::load::{
    ComputeError, ErrorKind, FinishReason, Finished, KernelId, LoadMessage, LoadMode, Notice,
    Phase, PhaseDone, Plan, Progress, GPU_BENCH_WARMUP_S,
};

use super::compute::ComputeLoad;
use super::device::{GpuDevice, GpuError};
use super::graphics::{GraphicsKind, GraphicsWorkload};
use super::pace::{self, Alternate};
use super::sizing::{submit_target_ms, VramBudget};
use super::submit::{Submit, Submitter};
use super::vram::VramWorkload;
use crate::args::Inject;
use crate::link::{EXIT_DEVICE_LOST, EXIT_OK};
use crate::rng::phase_seed;

/// One verified GPU load of a phase. `sub` must be the submission queue of the load's own
/// device (it wraps `GpuDevice::context()`): the loads write constants through the device
/// and record dispatches on the context `sub` hands them, in one order.
pub trait GpuWorkload {
    /// Calibrates one submission to about `target_ms` of GPU time and writes the golden
    /// output; `ReferenceInvalid` when the GPU disagrees with the CPU reference. Looks at
    /// `stop` at least every 100 ms and gives `Stopped` when it is raised.
    fn prepare(
        &mut self,
        sub: &mut dyn Submit,
        target_ms: f64,
        stop: &AtomicBool,
    ) -> Result<(), GpuError>;
    /// Sends one calibrated submission, with its comparison against the golden output.
    fn submit(&mut self, sub: &mut dyn Submit) -> Result<(), GpuError>;
    /// The verdicts of the submissions since the last check.
    fn check(&mut self, sub: &mut dyn Submit) -> Result<GpuCheck, GpuError>;
    /// The work of one calibrated submission in base units (FLOP, operations, bytes,
    /// pixels or texels) for the windows of a benchmark phase (DH4); 0 when the load has
    /// no measure, which skips such a phase.
    fn work_per_submission(&self) -> f64 {
        0.0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpuCheck {
    /// Submissions compared.
    pub checks: u64,
    pub mismatches: Vec<GpuMismatch>,
    /// `Notice` codes and values for the app.
    pub notices: Vec<(String, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuMismatch {
    /// The submission, counted from 1 after the golden one.
    pub iteration: u64,
    pub expected: u64,
    pub actual: u64,
}

/// What a load knows of its phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseCtx {
    pub seed: u64,
    pub integrated: bool,
    /// The fault injection (DA18), given only to the load of the injected kernel.
    pub inject: Option<Inject>,
    pub budget: VramBudget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuRunEnd {
    pub finished: Finished,
    /// `EXIT_DEVICE_LOST` after a lost device: the process ends with it.
    pub exit_code: i32,
}

/// A load, or `None` for a kernel without one yet (the phase is skipped).
pub type WorkloadResult = Result<Option<Box<dyn GpuWorkload>>, GpuError>;

/// What the engine runs on: [`run_gpu`] gives the D3D11 ones, the tests fakes.
pub struct Hooks<'h, D> {
    /// The device of the adapter with this LUID and its submission queue.
    pub open: &'h dyn Fn(u64) -> Result<(D, Box<dyn Submit>), GpuError>,
    /// The VRAM budget now (DG6), read at each phase start.
    pub budget: &'h dyn Fn(&D) -> VramBudget,
    pub workload: &'h dyn Fn(KernelId, &D, &PhaseCtx) -> WorkloadResult,
    pub clock: &'h (dyn Fn() -> Instant + Sync),
}

/// The load of `kernel`.
pub fn workload(kernel: KernelId, dev: &GpuDevice, ctx: &PhaseCtx) -> WorkloadResult {
    match kernel {
        KernelId::S1 | KernelId::S2 => Ok(Some(Box::new(ComputeLoad::new(kernel, dev, ctx)?))),
        KernelId::S4 => Ok(Some(Box::new(VramWorkload::new(dev, ctx)?))),
        KernelId::S5 => Ok(Some(Box::new(GraphicsWorkload::new(
            GraphicsKind::Fur,
            dev,
            ctx,
        )?))),
        KernelId::S6 => Ok(Some(Box::new(GraphicsWorkload::new(
            GraphicsKind::Artifact,
            dev,
            ctx,
        )?))),
        _ => Ok(None),
    }
}

/// Runs the GPU `plan` and returns how it ended; the caller sends the `Finished` and exits
/// with `exit_code` when it is not `EXIT_OK`.
pub fn run_gpu(
    plan: &Plan,
    out: &(dyn Fn(LoadMessage) + Sync),
    stop: &AtomicBool,
    inject: Option<Inject>,
) -> GpuRunEnd {
    let open = |luid| -> Result<(GpuDevice, Box<dyn Submit>), GpuError> {
        let dev = GpuDevice::open(luid)?;
        let sub = Submitter::new(&dev)?;
        Ok((dev, Box::new(sub)))
    };
    let budget = |dev: &GpuDevice| {
        let (budget, usage) = dev.video_memory().unwrap_or_default();
        VramBudget {
            budget,
            usage,
            available_ram: crate::sys::available_memory().unwrap_or(0),
        }
    };
    let hooks = Hooks {
        open: &open,
        budget: &budget,
        workload: &workload,
        clock: &Instant::now,
    };
    run_gpu_with(plan, out, stop, inject, &hooks)
}

/// [`run_gpu`] on the given device, loads and clock.
pub fn run_gpu_with<D>(
    plan: &Plan,
    out: &(dyn Fn(LoadMessage) + Sync),
    stop: &AtomicBool,
    inject: Option<Inject>,
    hooks: &Hooks<'_, D>,
) -> GpuRunEnd {
    let now = (hooks.clock)();
    let mut run = Run {
        plan,
        out,
        stop,
        inject,
        hooks,
        start: now,
        phase: 0,
        phase_start: now,
        phase_base: 0,
        level: None,
        checks: 0,
        errors: 0,
        errors_sent: 0,
        notices_sent: [0; PER_ERROR_NOTICES.len()],
        submissions: 0,
        rate_from: now,
        rate_count: 0,
        bench: false,
        window_rate: None,
        valid_windows: 0,
        rates: Vec::new(),
    };
    // `validate` refuses GPU kernels in a plan without a GPU.
    let luid = plan.gpu.map_or(0, |g| g.luid);
    let (dev, mut sub) = match (hooks.open)(luid) {
        Ok(opened) => opened,
        Err(error) => {
            let kernel = plan.phases.first().map_or(KernelId::S1, |p| p.kernel);
            return run.fatal(Fatal { kernel, error });
        }
    };
    for (index, spec) in plan.phases.iter().enumerate() {
        if stop.load(Ordering::Relaxed) {
            return run.end(FinishReason::Stopped);
        }
        run.phase = index as u32;
        run.phase_start = (hooks.clock)();
        run.phase_base = run.submissions;
        run.level = None;
        run.errors_sent = 0;
        run.notices_sent = [0; PER_ERROR_NOTICES.len()];
        run.bench = spec.windows.is_some();
        run.window_rate = None;
        run.valid_windows = 0;
        run.rates.clear();
        run.restart_rate();
        run.progress();
        let (checks, errors) = (run.checks, run.errors);
        let end = match run.run_phase(&dev, &mut *sub, spec) {
            Ok(end) => end,
            Err(fatal) => return run.fatal(fatal),
        };
        let rates = std::mem::take(&mut run.rates);
        run.send(LoadMessage::PhaseDone(PhaseDone {
            phase: run.phase,
            checks: run.checks - checks,
            errors: run.errors - errors,
            duration_ms: ms((hooks.clock)() - run.phase_start),
            skipped: match end {
                End::Skipped(reason) => Some(reason.to_owned()),
                _ => None,
            },
            work_ms: None,
            workers: Vec::new(),
            rates: match end {
                End::Skipped(_) => Vec::new(),
                _ => rates,
            },
        }));
        match end {
            End::Stopped => return run.end(FinishReason::Stopped),
            End::FirstError => return run.end(FinishReason::FirstError),
            End::Done | End::Skipped(_) => {}
        }
    }
    run.end(FinishReason::Completed)
}

const CHECK_EVERY: Duration = Duration::from_secs(1);
/// The `Progress` interval while the loads are built and calibrated: the app takes 5 s of
/// silence for a hung process.
const HEARTBEAT: Duration = Duration::from_millis(900);
/// The longest sleep of an idle stretch, so a stop is seen quickly.
const IDLE_SLICE: Duration = Duration::from_millis(10);
/// `Error` messages per phase; the mismatches after them are only counted.
const ERRORS_PER_PHASE: u32 = 16;
/// `Notice` codes a load sends once per bad frame or round: capped like the errors, so a
/// GPU that fails all the time does not flood the app's diary.
const PER_ERROR_NOTICES: [&str; 3] = ["artifact_tiles", "vram_bits", "vram_words"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Done,
    Stopped,
    FirstError,
    Skipped(&'static str),
}

/// An error that ends the run: the device is gone, a submission hung, or no adapter.
struct Fatal {
    kernel: KernelId,
    error: GpuError,
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

type Loads = [(KernelId, Box<dyn GpuWorkload>)];

struct Run<'r, D> {
    plan: &'r Plan,
    out: &'r (dyn Fn(LoadMessage) + Sync),
    stop: &'r AtomicBool,
    inject: Option<Inject>,
    hooks: &'r Hooks<'r, D>,
    start: Instant,
    phase: u32,
    phase_start: Instant,
    /// `submissions` when the phase started.
    phase_base: u64,
    /// `Progress.load_percent`: the level of `ramp` and `alternate`.
    level: Option<u8>,
    checks: u64,
    errors: u64,
    errors_sent: u32,
    /// Per-error notices sent in this phase, by code (see [`PER_ERROR_NOTICES`]).
    notices_sent: [u32; PER_ERROR_NOTICES.len()],
    /// Submissions sent; all of them are completed right after a check.
    submissions: u64,
    rate_from: Instant,
    rate_count: u64,
    /// The phase has `windows`: `Progress.rate` is `window_rate`.
    bench: bool,
    /// The rate of the last timed window, warm-up included, in units a second.
    window_rate: Option<f64>,
    /// Windows with a rate in this phase.
    valid_windows: u32,
    /// The rates of the windows after the warm-up.
    rates: Vec<f64>,
}

impl<D> Run<'_, D> {
    fn send(&self, msg: LoadMessage) {
        (self.out)(msg);
    }

    fn now(&self) -> Instant {
        (self.hooks.clock)()
    }

    fn end(&self, reason: FinishReason) -> GpuRunEnd {
        GpuRunEnd {
            finished: Finished {
                reason,
                checks: self.checks,
                errors: self.errors,
            },
            exit_code: EXIT_OK,
        }
    }

    fn error(&self, kernel: KernelId, kind: ErrorKind, iteration: u64, expected: u64, actual: u64) {
        let spec = &self.plan.phases[self.phase as usize];
        self.send(LoadMessage::Error(ComputeError {
            phase: self.phase,
            kernel,
            isa: spec.isa,
            kind,
            logical: None,
            core: None,
            iteration,
            expected,
            actual,
            seed: phase_seed(self.plan.seed, self.phase),
            load_percent: self.level,
        }));
    }

    fn fatal(&self, f: Fatal) -> GpuRunEnd {
        let iteration = self.submissions - self.phase_base;
        let mut end = self.end(FinishReason::Failed);
        match f.error {
            GpuError::NotFound => self.send(LoadMessage::Notice(Notice {
                phase: self.phase,
                code: "gpu_missing".to_owned(),
                value: None,
            })),
            GpuError::Lost(reason) => {
                tracing::error!(reason = format_args!("{reason:#010x}"), "the GPU was lost");
                self.error(
                    f.kernel,
                    ErrorKind::DeviceLost,
                    iteration,
                    0,
                    u64::from(reason),
                );
                end.exit_code = EXIT_DEVICE_LOST;
            }
            GpuError::Hung => {
                tracing::error!(kernel = ?f.kernel, "a GPU submission is hung");
                self.error(f.kernel, ErrorKind::Hung, iteration, 0, 0);
            }
            other => {
                tracing::error!(error = ?other, "the GPU cannot be used");
                // Only `open` gets here: a load's other errors skip its phase.
                self.send(LoadMessage::Notice(Notice {
                    phase: self.phase,
                    code: "gpu_error".to_owned(),
                    value: None,
                }));
            }
        }
        end
    }

    /// Sorts an error of a load: the fatal ones end the run, the others skip the phase.
    fn failed(&self, kernel: KernelId, error: GpuError) -> Result<End, Fatal> {
        match error {
            GpuError::Lost(_) | GpuError::Hung | GpuError::NotFound => Err(Fatal { kernel, error }),
            GpuError::ReferenceInvalid => {
                tracing::error!(kernel = ?kernel, "the GPU disagrees with the CPU reference");
                self.error(kernel, ErrorKind::ReferenceInvalid, 0, 0, 0);
                Ok(End::Skipped("reference_invalid"))
            }
            GpuError::Stopped => Ok(End::Stopped),
            GpuError::OutOfMemory => Ok(End::Skipped("vram")),
            GpuError::Create(_) | GpuError::TimingDisjoint => {
                tracing::warn!(kernel = ?kernel, error = ?error, "GPU phase skipped");
                Ok(End::Skipped("gpu_error"))
            }
        }
    }

    fn restart_rate(&mut self) {
        self.rate_from = self.now();
        self.rate_count = self.submissions;
    }

    /// `Progress`, with `rate` = submissions per second since the last one (DG7), or the
    /// rate of the last window in a benchmark phase (DH4).
    fn progress(&mut self) {
        let now = self.now();
        let dt = (now - self.rate_from).as_secs_f64();
        let rate = if self.bench {
            self.window_rate
        } else {
            (dt > 0.0).then(|| (self.submissions - self.rate_count) as f64 / dt)
        };
        self.rate_from = now;
        self.rate_count = self.submissions;
        self.send(LoadMessage::Progress(Progress {
            phase: self.phase,
            phase_elapsed_ms: ms(now - self.phase_start),
            elapsed_ms: ms(now - self.start),
            checks: self.checks,
            errors: self.errors,
            current_core: None,
            cores: Vec::new(),
            memory_bytes: 0,
            rate,
            load_percent: self.level,
        }));
    }

    fn run_phase(&mut self, dev: &D, sub: &mut dyn Submit, spec: &Phase) -> Result<End, Fatal> {
        let seed = phase_seed(self.plan.seed, self.phase);
        let mut loads: Vec<(KernelId, Box<dyn GpuWorkload>)> = Vec::new();
        // Building and calibrating can take seconds (S4 allocates GiBs of VRAM).
        if let Some(end) = self.heartbeat(|| self.build(dev, sub, spec, seed, &mut loads))? {
            return Ok(end);
        }
        if spec.windows.is_some() && loads[0].1.work_per_submission() == 0.0 {
            return Ok(End::Skipped("unsupported"));
        }
        // The calibration is not part of the rate.
        self.restart_rate();
        let end = self.submit_loop(sub, spec, seed, &mut loads);
        if spec.windows.is_some() && end.is_ok() {
            // A submission error that only skips the phase leaves its window open: close
            // it (a no-op when already closed), so the next phase can open its own.
            if let Err(e) = sub.window_end() {
                tracing::warn!(error = ?e, "cannot close the benchmark window");
            }
        }
        end
    }

    /// Creates and prepares the loads of `spec`; `Some` when the phase ends before submitting.
    fn build(
        &self,
        dev: &D,
        sub: &mut dyn Submit,
        spec: &Phase,
        seed: u64,
        loads: &mut Vec<(KernelId, Box<dyn GpuWorkload>)>,
    ) -> Result<Option<End>, Fatal> {
        let integrated = self.plan.gpu.is_some_and(|g| g.integrated);
        let budget = (self.hooks.budget)(dev);
        for kernel in std::iter::once(spec.kernel).chain(spec.alt_kernel) {
            let ctx = PhaseCtx {
                seed,
                integrated,
                inject: self.inject.clone().filter(|i| i.kernel == kernel),
                budget,
            };
            match (self.hooks.workload)(kernel, dev, &ctx) {
                Ok(Some(load)) => loads.push((kernel, load)),
                Ok(None) if loads.is_empty() => return Ok(Some(End::Skipped("unsupported"))),
                Ok(None) => {
                    tracing::info!(kernel = ?kernel, "no alt kernel: the main one runs alone")
                }
                Err(e) => return self.failed(kernel, e).map(Some),
            }
        }
        let target_ms = submit_target_ms(integrated);
        for (kernel, load) in loads.iter_mut() {
            if let Err(e) = load.prepare(sub, target_ms, self.stop) {
                return self.failed(*kernel, e).map(Some);
            }
        }
        Ok(None)
    }

    /// Runs `f` while a thread sends a `Progress` without a rate every [`HEARTBEAT`].
    fn heartbeat<R>(&self, f: impl FnOnce() -> R) -> R {
        let (done, beat) = mpsc::channel::<()>();
        let (out, clock) = (self.out, self.hooks.clock);
        let (start, phase_start) = (self.start, self.phase_start);
        let base = Progress {
            phase: self.phase,
            phase_elapsed_ms: 0,
            elapsed_ms: 0,
            checks: self.checks,
            errors: self.errors,
            current_core: None,
            cores: Vec::new(),
            memory_bytes: 0,
            rate: None,
            load_percent: None,
        };
        thread::scope(|s| {
            s.spawn(move || {
                // Ends when `done` is dropped, right after `f`.
                while beat.recv_timeout(HEARTBEAT) == Err(RecvTimeoutError::Timeout) {
                    let now = clock();
                    out(LoadMessage::Progress(Progress {
                        phase_elapsed_ms: ms(now - phase_start),
                        elapsed_ms: ms(now - start),
                        ..base.clone()
                    }));
                }
            });
            let result = f();
            drop(done);
            result
        })
    }

    /// Submits until the phase ends, pacing by the mode (DG10) and checking once a second.
    /// `alt_kernel` takes every other submission.
    fn submit_loop(
        &mut self,
        sub: &mut dyn Submit,
        spec: &Phase,
        seed: u64,
        loads: &mut Loads,
    ) -> Result<End, Fatal> {
        let phase_ms = u64::from(spec.duration_s) * 1000;
        let mut alternate = Alternate::new(seed);
        let mut next = 0;
        let mut checked_at = self.now();
        // The end of the pause after a partial-load submission.
        let mut idle_until: Option<Instant> = None;
        // A benchmark phase (DH4): one load, steady, a window open from here.
        let windows = spec.windows.map(usize::from);
        let (bench_kernel, work) = (loads[0].0, loads[0].1.work_per_submission());
        let mut window_from = self.submissions;
        if windows.is_some() {
            if let Err(e) = sub.window_begin() {
                return self.failed(bench_kernel, e);
            }
        }
        loop {
            let now = self.now();
            let elapsed = ms(now - self.phase_start);
            let ending = if self.stop.load(Ordering::Relaxed) {
                Some(End::Stopped)
            } else if elapsed >= phase_ms {
                Some(End::Done)
            } else {
                None
            };
            if let Some(ending) = ending {
                // The open window is cut short: it does not count.
                if windows.is_some() {
                    if let Some(end) = self.close_window(sub, bench_kernel, None)? {
                        return Ok(end);
                    }
                }
                return Ok(self.check(sub, spec, loads)?.unwrap_or(ending));
            }
            // 0 is the idle part of `pause_resume`.
            let level = match spec.mode {
                LoadMode::Ramp => pace::ramp_level(elapsed, phase_ms),
                LoadMode::Alternate => alternate.level_at(elapsed),
                LoadMode::PauseResume if !pace::pause_resume_active(elapsed) => 0,
                _ => 100,
            };
            self.level = matches!(spec.mode, LoadMode::Ramp | LoadMode::Alternate).then_some(level);
            if now - checked_at >= CHECK_EVERY {
                checked_at = now;
                if let Some(wanted) = windows {
                    let done = (self.submissions - window_from) as f64 * work;
                    if let Some(end) = self.close_window(sub, bench_kernel, Some(done))? {
                        return Ok(end);
                    }
                    if self.rates.len() == wanted {
                        return Ok(self.check(sub, spec, loads)?.unwrap_or(End::Done));
                    }
                }
                if let Some(end) = self.check(sub, spec, loads)? {
                    return Ok(end);
                }
                if windows.is_some() {
                    // The check waited for the GPU: the next window starts after it.
                    if let Err(e) = sub.window_begin() {
                        return self.failed(bench_kernel, e);
                    }
                    window_from = self.submissions;
                    checked_at = self.now();
                }
            }
            if level == 0 || idle_until.is_some_and(|t| now < t && level < 100) {
                // A pause lowers the load only with nothing in flight.
                if let Err(e) = sub.finish() {
                    return self.failed(loads[0].0, e);
                }
                thread::sleep(IDLE_SLICE);
                continue;
            }
            idle_until = None;
            let count = loads.len();
            let (kernel, load) = &mut loads[next];
            next = (next + 1) % count;
            let submitted = if level < 100 {
                // Timed alone: the submissions in flight end first, and so does this one.
                sub.finish()
                    .and_then(|()| {
                        let t0 = self.now();
                        load.submit(sub)?;
                        sub.finish()?;
                        Ok(self.now() - t0)
                    })
                    .map(|took| {
                        let idle = pace::idle_after_ms(level, took.as_secs_f64() * 1e3);
                        idle_until = Some(self.now() + Duration::from_secs_f64(idle / 1e3));
                    })
            } else {
                load.submit(sub)
            };
            if let Err(e) = submitted {
                return self.failed(*kernel, e);
            }
            self.submissions += 1;
        }
    }

    /// Ends the window of a benchmark phase and, given the `work` it did, records its rate:
    /// the first [`GPU_BENCH_WARMUP_S`] windows with a rate are the warm-up (DH4).
    fn close_window(
        &mut self,
        sub: &mut dyn Submit,
        kernel: KernelId,
        work: Option<f64>,
    ) -> Result<Option<End>, Fatal> {
        let ms = match sub.window_end() {
            Ok(ms) => ms,
            Err(e) => return self.failed(kernel, e).map(Some),
        };
        if let Some(rate) = work.and_then(|work| window_rate(work, ms)) {
            self.window_rate = Some(rate);
            self.valid_windows += 1;
            if self.valid_windows > GPU_BENCH_WARMUP_S {
                self.rates.push(rate);
            }
        }
        Ok(None)
    }

    /// Reads every load's checks, reports them and sends `Progress`. `FirstError` after a
    /// mismatch with `stop_on_error`; `Skipped` when a load failed.
    fn check(
        &mut self,
        sub: &mut dyn Submit,
        spec: &Phase,
        loads: &mut Loads,
    ) -> Result<Option<End>, Fatal> {
        let mut first_error = false;
        for (kernel, load) in loads.iter_mut() {
            let checked = match load.check(sub) {
                Ok(checked) => checked,
                Err(e) => return self.failed(*kernel, e).map(Some),
            };
            self.checks += checked.checks;
            for (code, value) in checked.notices {
                if let Some(i) = PER_ERROR_NOTICES.iter().position(|c| *c == code) {
                    if self.notices_sent[i] == ERRORS_PER_PHASE {
                        continue;
                    }
                    self.notices_sent[i] += 1;
                }
                self.send(LoadMessage::Notice(Notice {
                    phase: self.phase,
                    code,
                    value: Some(value),
                }));
            }
            for m in checked.mismatches {
                self.errors += 1;
                first_error |= spec.stop_on_error;
                if self.errors_sent < ERRORS_PER_PHASE {
                    self.errors_sent += 1;
                    self.error(
                        *kernel,
                        ErrorKind::Mismatch,
                        m.iteration,
                        m.expected,
                        m.actual,
                    );
                }
            }
        }
        self.progress();
        Ok(first_error.then_some(End::FirstError))
    }
}

/// Units a second of a window that did `work` in `ms` of GPU time; `None` for a disjoint
/// window or one without GPU time, so no rate is ever `NaN` or infinite.
fn window_rate(work: f64, ms: Option<f64>) -> Option<f64> {
    ms.filter(|&ms| ms > 0.0)
        .map(|ms| work * 1e3 / ms)
        .filter(|rate| rate.is_finite())
}
