//! The CPU (M8a2) and GPU (M8b2) benchmarks on the runner's `oma-perf-runner`
//! thread: the plan of `bench_plan` or `gpu_bench_plan`, the helper and the pure
//! `BenchController` or `GpuBenchController`, fed every 250 ms with the helper's
//! messages and the sampler's CPU or GPU readings, and every 5 s with the battery
//! and the other processes' CPU or GPU share (DB7, DH9, DH10). They share the
//! runner's `active` slot with the stress test (DB8, DH12): one job at a time,
//! the same stop and shutdown. A stopped benchmark saves nothing (DB9).

use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use oma_core::load::{resolve_cpu_sensors, BuildError, Clock, SensorSample};
use oma_core::scores::{
    bench_plan, cpu_baseline, gpu_baseline, gpu_bench_plan, BenchAction, BenchContext,
    BenchController, BenchEnd, BenchState, BenchStatus, Device, GpuBenchContext,
    GpuBenchController, ScoreFile,
};
use oma_ipc::load::{GpuTarget, Isa, LoadMessage, Plan, StopRequest};

use super::host::{HostEvent, StartFailure};
use super::runner::{
    clock_at, core_count, lock, plan_message, seed_of, unix_now_ms, Active, Control, LoadLink,
    PerformanceRunner, RunnerDeps, StartError, EXIT_GRACE, SAMPLE_QUEUE, TICK,
};
use super::store::to_rfc3339;
use crate::i18n::t;
use crate::notifier::{launch_for_score_cpu, launch_for_score_gpu};
use crate::tray::language_for;

/// Emitted to the main window on every state change and twice a second during
/// the benchmark, only while a window is open; the payload is a [`BenchStatus`].
pub const EVENT_BENCH: &str = "performance-bench";

const EMIT_EVERY: Duration = Duration::from_millis(500);
/// Battery and other processes' CPU or GPU share (DB7, DH10).
const POLL_EVERY: Duration = Duration::from_secs(5);

/// The controller the worker drives: the CPU or the GPU one, same lifecycle.
trait Ctl: Send {
    fn on_load(&mut self, msg: &LoadMessage, now: Clock) -> Vec<BenchAction>;
    fn on_sample(&mut self, sample: &SensorSample, now: Clock) -> Vec<BenchAction>;
    /// The other processes' share of the CPU or of the GPU, 0-1.
    fn on_busy(&mut self, share: f64);
    fn on_battery(&mut self, on_battery: bool);
    fn on_user_stop(&mut self, now: Clock) -> Vec<BenchAction>;
    fn on_clock(&mut self, now: Clock) -> Vec<BenchAction>;
    fn on_exit(&mut self, code: Option<i32>, now: Clock) -> Vec<BenchAction>;
    fn status(&self) -> BenchStatus;
}

macro_rules! impl_ctl {
    ($t:ty, $busy:ident) => {
        impl Ctl for $t {
            fn on_load(&mut self, msg: &LoadMessage, now: Clock) -> Vec<BenchAction> {
                <$t>::on_load(self, msg, now)
            }
            fn on_sample(&mut self, sample: &SensorSample, now: Clock) -> Vec<BenchAction> {
                <$t>::on_sample(self, sample, now)
            }
            fn on_busy(&mut self, share: f64) {
                <$t>::$busy(self, share);
            }
            fn on_battery(&mut self, on_battery: bool) {
                <$t>::on_battery(self, on_battery);
            }
            fn on_user_stop(&mut self, now: Clock) -> Vec<BenchAction> {
                <$t>::on_user_stop(self, now)
            }
            fn on_clock(&mut self, now: Clock) -> Vec<BenchAction> {
                <$t>::on_clock(self, now)
            }
            fn on_exit(&mut self, code: Option<i32>, now: Clock) -> Vec<BenchAction> {
                <$t>::on_exit(self, code, now)
            }
            fn status(&self) -> BenchStatus {
                <$t>::status(self)
            }
        }
    };
}

impl_ctl!(BenchController, on_busy_share);
impl_ctl!(GpuBenchController, on_busy_gpu);

/// Whose share of the machine the 5 s poll reads.
enum Busy {
    /// The CPU, with this many logical processors.
    Cpu(u32),
    /// The GPU of this `device_id`.
    Gpu(String),
}

/// A benchmark ready to start.
struct BenchJob {
    id: String,
    ctl: Box<dyn Ctl>,
    plan: Plan,
    busy: Busy,
    /// Physical cores (the CPU sensors); 0 for a GPU.
    cores: usize,
    /// The GPU under test: the sampler reads its sensors.
    gpu: Option<String>,
    /// The controller's clock origin, also the thread's.
    epoch: Instant,
}

impl PerformanceRunner {
    /// The free slot, the last job's thread joined; `Busy` while a test or a benchmark runs.
    fn free_slot(&self) -> Result<MutexGuard<'_, Option<Active>>, StartError> {
        let mut active = lock(&self.active);
        if active.as_ref().is_some_and(Active::running) {
            return Err(StartError::Busy);
        }
        if let Some(old) = active.take() {
            let _ = old.thread.join();
        }
        Ok(active)
    }

    /// Starts the CPU benchmark; its score id, or `Busy` while a test or a
    /// benchmark runs. A helper that does not start saves nothing: the status
    /// ends `failed` with the reason's i18n key in `error`.
    pub fn start_bench(&self) -> Result<String, StartError> {
        let active = self.free_slot()?;
        let deps = &self.deps;
        let id = oma_win::overlay_pipe::random_uuid_v4()
            .map_err(|e| StartError::System(e.to_string()))?;
        let topology = deps
            .machine
            .topology()
            .map_err(|e| StartError::System(e.to_string()))?;
        let isa = deps.machine.isa().first().copied().unwrap_or(Isa::Sse2);
        let (plan, steps) = bench_plan(&topology, isa, seed_of(&id));
        let logical = topology.logical.len() as u32;
        let cores = core_count(&topology);
        let ctx = BenchContext {
            id: id.clone(),
            at: to_rfc3339(unix_now_ms()),
            isa,
            device: Device {
                model: topology.brand.clone(),
                cores: cores as u32,
                logical,
                ..Device::default()
            },
            logical,
            tjmax_c: resolve_cpu_sensors(&(deps.schema)(), 0).tjmax_c,
            service_available: (deps.service_available)(),
            on_battery: deps.machine.on_battery(),
            hypervisor: topology.hypervisor,
            baseline: cpu_baseline(),
            app_version: deps.app_version.clone(),
        };
        let epoch = Instant::now();
        let ctl = BenchController::new(steps, ctx, clock_at(epoch, deps.machine.as_ref()));
        self.spawn_bench(
            active,
            BenchJob {
                id,
                ctl: Box::new(ctl),
                plan,
                busy: Busy::Cpu(logical),
                cores,
                gpu: None,
                epoch,
            },
        )
    }

    /// Starts the benchmark of GPU `device_id`, which must be one of the machine's
    /// (else `Plan(NoGpu)`: an id from the UI is never trusted); its score id, or
    /// `Busy` while a test or a benchmark runs.
    pub fn start_gpu_bench(&self, device_id: &str) -> Result<String, StartError> {
        let active = self.free_slot()?;
        let deps = &self.deps;
        let adapter = deps
            .machine
            .gpus()
            .into_iter()
            .find(|g| g.device_id == device_id)
            .ok_or(StartError::Plan(BuildError::NoGpu))?;
        let id = oma_win::overlay_pipe::random_uuid_v4()
            .map_err(|e| StartError::System(e.to_string()))?;
        let target = GpuTarget {
            luid: adapter.luid,
            integrated: adapter.integrated,
        };
        let (plan, steps) = gpu_bench_plan(target, seed_of(&id));
        let ctx = GpuBenchContext {
            id: id.clone(),
            at: to_rfc3339(unix_now_ms()),
            device: Device {
                model: adapter.name.clone(),
                vendor_id: Some(adapter.vendor_id),
                dedicated_bytes: Some(adapter.dedicated_bytes),
                ..Device::default()
            },
            device_id: adapter.device_id.clone(),
            integrated: adapter.integrated,
            on_battery: deps.machine.on_battery(),
            baseline: gpu_baseline(),
            app_version: deps.app_version.clone(),
        };
        let epoch = Instant::now();
        let ctl = GpuBenchController::new(steps, ctx, clock_at(epoch, deps.machine.as_ref()));
        self.spawn_bench(
            active,
            BenchJob {
                id,
                ctl: Box::new(ctl),
                plan,
                busy: Busy::Gpu(adapter.device_id.clone()),
                cores: 0,
                gpu: Some(adapter.device_id),
                epoch,
            },
        )
    }

    fn spawn_bench(
        &self,
        mut active: MutexGuard<'_, Option<Active>>,
        job: BenchJob,
    ) -> Result<String, StartError> {
        let BenchJob {
            id,
            ctl,
            plan,
            busy,
            cores,
            gpu,
            epoch,
        } = job;
        // Set before the thread runs, so a later status from it is never overwritten.
        let previous = lock(&self.bench_status).replace(ctl.status());

        let (samples, sample_rx) = mpsc::sync_channel(SAMPLE_QUEUE);
        let control = Arc::new(Control::default());
        let worker = BenchWorker {
            deps: Arc::clone(&self.deps),
            control: Arc::clone(&control),
            samples: sample_rx,
            status: Arc::clone(&self.bench_status),
            epoch,
            last_state: None,
            last_emit: None,
        };
        let thread = std::thread::Builder::new()
            .name("oma-perf-runner".into())
            .spawn(move || {
                let (deps, status) = (Arc::clone(&worker.deps), Arc::clone(&worker.status));
                let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker.run(ctl, plan, busy)
                }));
                if run.is_err() {
                    after_panic(&deps, &status);
                }
            })
            .map_err(|e| {
                *lock(&self.bench_status) = previous;
                StartError::System(e.to_string())
            })?;
        *active = Some(Active {
            thread,
            control,
            samples,
            cores,
            sensors: None,
            gpu,
            bench: true,
        });
        Ok(id)
    }

    /// Stops the benchmark in progress (nothing is saved); never a stress test.
    pub fn stop_bench(&self) {
        if let Some(a) = lock(&self.active).as_ref().filter(|a| a.bench) {
            a.control.stop.store(true, Ordering::Release);
        }
    }

    /// The benchmark in progress or the last one, CPU or GPU; `None` before any.
    pub fn bench_status(&self) -> Option<BenchStatus> {
        lock(&self.bench_status).clone()
    }
}

/// The thread panicked: the status ends `failed` (`crashed`), so the tray mark
/// goes and a new job can start. Nothing is saved.
fn after_panic(deps: &RunnerDeps, status: &Mutex<Option<BenchStatus>>) {
    tracing::error!("the benchmark runner panicked");
    let mut st = lock(status).clone();
    if let Some(st) = st.as_mut() {
        st.state = BenchState::Failed;
        st.error = Some("crashed".into());
        (deps.on_bench)(st);
        if (deps.window_open)() {
            (deps.emit_bench)(st);
        }
    }
    *lock(status) = st;
}

/// The benchmark's `oma-perf-runner` thread.
struct BenchWorker {
    deps: Arc<RunnerDeps>,
    control: Arc<Control>,
    samples: Receiver<(SensorSample, bool)>,
    status: Arc<Mutex<Option<BenchStatus>>>,
    epoch: Instant,
    last_state: Option<BenchState>,
    last_emit: Option<Instant>,
}

/// The controller, the helper it acts on, and how the run ended.
struct BenchRun {
    ctl: Box<dyn Ctl>,
    host: Box<dyn LoadLink>,
    killed: bool,
    end: Option<BenchEnd>,
    saved: Option<Box<ScoreFile>>,
}

impl BenchWorker {
    fn clock(&self) -> Clock {
        clock_at(self.epoch, self.deps.machine.as_ref())
    }

    /// Copies the status for the commands; tells the tray on a state change
    /// and the window on a change or twice a second, while one is open.
    fn publish(&mut self, status: BenchStatus) {
        let changed = self.last_state != Some(status.state);
        if changed {
            self.last_state = Some(status.state);
            (self.deps.on_bench)(&status);
        }
        let due = self.last_emit.is_none_or(|t| t.elapsed() >= EMIT_EVERY);
        if (changed || due) && (self.deps.window_open)() {
            self.last_emit = Some(Instant::now());
            (self.deps.emit_bench)(&status);
        }
        *lock(&self.status) = Some(status);
    }

    /// The probe of the 5 s poll. The CPU one is primed by the poll right after the
    /// handshake (PDH needs two collections), so its first share covers the first 5 s
    /// of the run (DB7: "all'avvio"), not the launch, when the antivirus scans
    /// `oma-load.exe`. The GPU one reads the provider's table.
    fn busy_probe(&self, busy: Busy) -> Box<dyn FnMut() -> Option<f64>> {
        match busy {
            Busy::Cpu(logical) => self.deps.machine.busy_probe(logical),
            Busy::Gpu(device_id) => {
                let deps = Arc::clone(&self.deps);
                Box::new(move || deps.machine.gpu_busy_share(&device_id))
            }
        }
    }

    fn run(mut self, mut ctl: Box<dyn Ctl>, plan: Plan, busy: Busy) {
        #[cfg(windows)]
        let _awake = oma_win::power::KeepAwake::new();
        self.publish(ctl.status());
        let run = match plan_message(plan) {
            Ok(run) => run,
            Err(failure) => return self.failed(ctl.as_ref(), &failure),
        };
        let mut busy = self.busy_probe(busy);
        let (tx, rx) = mpsc::channel();
        let host = match (self.deps.launcher)(tx) {
            Ok(host) => host,
            Err(failure) => return self.failed(ctl.as_ref(), &failure),
        };
        // The handshake already took the `Hello`: the controller gets the version from it.
        let _ = ctl.on_load(&LoadMessage::Hello(host.hello().clone()), self.clock());
        let mut r = BenchRun {
            ctl,
            host,
            killed: false,
            end: None,
            saved: None,
        };
        self.poll(&mut r, &mut busy);
        if self.control.stop.swap(false, Ordering::AcqRel) {
            // Stopped during the handshake: the plan never runs.
            let a = r.ctl.on_user_stop(self.clock());
            self.exec(&mut r, a);
        } else if let Err(err) = r.host.send(&run) {
            // The pipe is gone: `Closed` and `Exited` follow.
            tracing::warn!(%err, "cannot send the benchmark plan to oma-load");
        }
        self.drive(&mut r, &rx, &mut busy);
        // Closes the pipe and the Job: the helper is gone whatever it was doing.
        r.host.kill();
        self.publish(r.ctl.status());
        self.toast(&r);
    }

    /// The helper did not start: nothing saved, the reason's key in `error`.
    fn failed(mut self, ctl: &dyn Ctl, failure: &StartFailure) {
        tracing::warn!(%failure, "the benchmark did not start");
        let mut status = ctl.status();
        status.state = BenchState::Failed;
        status.error = Some(failure.i18n_key().into());
        self.publish(status);
    }

    /// The battery and the other processes' CPU or GPU share (DB7, DH9).
    fn poll(&self, r: &mut BenchRun, busy: &mut dyn FnMut() -> Option<f64>) {
        if let Some(on) = self.deps.machine.on_battery() {
            r.ctl.on_battery(on);
        }
        if let Some(share) = busy() {
            r.ctl.on_busy(share);
        }
    }

    fn exec(&self, r: &mut BenchRun, actions: Vec<BenchAction>) {
        for action in actions {
            match action {
                BenchAction::SendStop => {
                    if !r.killed {
                        if let Err(err) = r.host.send(&LoadMessage::Stop(StopRequest {})) {
                            tracing::warn!(%err, "cannot ask oma-load to stop");
                        }
                    }
                }
                BenchAction::Kill => {
                    r.host.kill();
                    r.killed = true;
                }
                BenchAction::Save(file) => {
                    if let Err(err) = self.deps.store.save_score(&file) {
                        tracing::warn!(%err, "cannot save the benchmark score");
                    }
                    r.saved = Some(file);
                }
                BenchAction::Finished(end) => r.end = Some(end),
            }
        }
    }

    /// The 250 ms loop, until the controller's end.
    fn drive(
        &mut self,
        r: &mut BenchRun,
        rx: &Receiver<HostEvent>,
        busy: &mut dyn FnMut() -> Option<f64>,
    ) {
        let mut exited: Option<(Option<i32>, Instant)> = None;
        let mut closed = false;
        let mut exit_seen = false;
        let mut forced = false;
        let mut next_poll = Instant::now() + POLL_EVERY;
        loop {
            let mut events = Vec::new();
            match rx.recv_timeout(TICK) {
                Ok(e) => events.push(e),
                Err(RecvTimeoutError::Timeout) => {}
                // Every sender gone: only the clock is left.
                Err(RecvTimeoutError::Disconnected) => std::thread::sleep(TICK),
            }
            events.extend(rx.try_iter());
            for event in events {
                if r.killed || r.end.is_some() {
                    break;
                }
                match event {
                    HostEvent::Message(m) => {
                        let a = r.ctl.on_load(&m, self.clock());
                        self.exec(r, a);
                    }
                    HostEvent::Closed => closed = true,
                    HostEvent::Exited(code) => {
                        exited.get_or_insert((code, Instant::now()));
                    }
                }
            }
            // `Closed` comes after the last message: an exit waits for it.
            if let Some((code, at)) = exited {
                if !exit_seen && !r.killed && (closed || at.elapsed() >= EXIT_GRACE) {
                    exit_seen = true;
                    let a = r.ctl.on_exit(code, self.clock());
                    self.exec(r, a);
                }
            }
            while let Ok((sample, _service)) = self.samples.try_recv() {
                let a = r.ctl.on_sample(&sample, self.clock());
                self.exec(r, a);
            }
            if Instant::now() >= next_poll {
                next_poll += POLL_EVERY;
                self.poll(r, busy);
            }
            if self.control.stop.swap(false, Ordering::AcqRel) {
                let a = r.ctl.on_user_stop(self.clock());
                self.exec(r, a);
            }
            let late = self.control.deadline().is_some_and(|d| Instant::now() >= d);
            if late && !forced && r.end.is_none() {
                // The app is leaving: the helper goes, nothing is saved.
                forced = true;
                self.exec(r, vec![BenchAction::Kill]);
                let a = r.ctl.on_exit(Some(0), self.clock());
                self.exec(r, a);
            }
            let a = r.ctl.on_clock(self.clock());
            self.exec(r, a);
            if r.end.is_some() {
                return;
            }
            self.publish(r.ctl.status());
        }
    }

    /// Only for a saved score, with the window not visible and the app staying (DB9).
    fn toast(&self, r: &BenchRun) {
        if self.control.deadline().is_some() || (self.deps.window_visible)() {
            return;
        }
        let (Some(BenchEnd::Saved(_)), Some(file)) = (&r.end, &r.saved) else {
            return;
        };
        let lang = language_for(self.deps.settings.snapshot().general.language);
        let points = |p: Option<u32>| p.map_or_else(|| "\u{2013}".to_owned(), |p| p.to_string());
        let gpu = file.category == "gpu";
        let body = match (gpu, file.valid) {
            (false, true) => {
                let (single, multi) = (points(file.scores.single), points(file.scores.multi));
                t(
                    lang,
                    "performance.toast.benchDone",
                    &[("single", single.as_str()), ("multi", multi.as_str())],
                )
            }
            (false, false) => t(lang, "performance.toast.benchInvalid", &[]),
            (true, true) => {
                let compute = points(file.scores.compute);
                let graphics = points(file.scores.graphics);
                t(
                    lang,
                    "performance.toast.gpuBenchDone",
                    &[
                        ("compute", compute.as_str()),
                        ("graphics", graphics.as_str()),
                    ],
                )
            }
            (true, false) => t(lang, "performance.toast.gpuBenchInvalid", &[]),
        };
        let (title, launch) = if gpu {
            let device = file.device.device_id.as_deref().unwrap_or_default();
            (
                t(lang, "performance.score.gpu.title", &[]),
                launch_for_score_gpu(device),
            )
        } else {
            (
                t(lang, "performance.score.title", &[]),
                launch_for_score_cpu(),
            )
        };
        self.deps.toaster.show(title, body, launch);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use oma_core::scores::{bench_plan, BenchState, WORKLOADS};
    use oma_ipc::load::{FinishReason, Isa, LoadMessage, PhaseDone};

    use super::super::host::StartFailure;
    use super::super::runner::tests::{
        blocked, finished, progress, request, rig_on, rig_with, scripted, topology, wait_idle,
        FakeMachine, Rig, Script,
    };
    use super::super::runner::{PerformanceRunner, StartError};
    use crate::notifier::launch_for_score_cpu;
    use crate::window::{quit_action, QuitAction, QuitSource};

    /// One `PhaseDone` per step of the plan, each repetition about a second.
    fn all_phases_done() -> Vec<LoadMessage> {
        let (_, steps) = bench_plan(&topology(), Isa::Avx2, 1);
        steps
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let w = WORKLOADS.iter().find(|w| w.id == s.kernel).unwrap();
                LoadMessage::PhaseDone(PhaseDone {
                    phase: i as u32,
                    checks: w.iterations,
                    errors: 0,
                    duration_ms: 1000,
                    skipped: None,
                    work_ms: Some(1000),
                    workers: vec![],
                    rates: vec![],
                })
            })
            .collect()
    }

    fn full_run() -> Script {
        let mut on_run = vec![progress(0)];
        on_run.extend(all_phases_done());
        on_run.push(finished(FinishReason::Completed, 0));
        Script {
            on_run,
            exit_after_run: true,
            ..Default::default()
        }
    }

    fn wait_bench_running(runner: &PerformanceRunner) {
        let until = Instant::now() + Duration::from_secs(3);
        while runner.bench_status().map(|s| s.state) != Some(BenchState::Running) {
            assert!(Instant::now() < until, "never running");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn bench_runs_to_a_saved_score() {
        let machine = FakeMachine {
            busy: Some(0.5),
            ..Default::default()
        };
        let rig = rig_on("bench-saved", scripted(full_run()), machine);
        assert!(rig.runner.bench_status().is_none());
        let id = rig.runner.start_bench().unwrap();
        wait_idle(&rig.runner);
        let list = rig.runner.store().list_scores();
        assert_eq!(list.len(), 1, "{list:?}");
        let s = rig.runner.store().load_score(&id).unwrap().unwrap();
        assert!(s.valid);
        assert!(s.scores.single.is_some() && s.scores.multi.is_some());
        assert_eq!(s.load_version.as_deref(), Some("9.9.9"));
        assert_eq!(s.device.model, "Test CPU");
        assert_eq!(s.isa, Some(Isa::Avx2));
        // The busy poll at the start, and the missing service.
        assert!(
            s.flags.contains(&"busy_system".to_string()),
            "{:?}",
            s.flags
        );
        assert!(s.flags.contains(&"no_sensors".to_string()));
        // No stress session.
        assert!(rig.runner.store().list().is_empty());
        let st = rig.runner.bench_status().unwrap();
        assert_eq!(st.state, BenchState::Done);
        assert_eq!(st.score_id.as_deref(), Some(id.as_str()));
        // Starting and done reached the window and the tray; the mark is gone.
        assert!(rig.bench_emitted.load(Ordering::SeqCst) >= 2);
        assert!(!rig.bench_mark.load(Ordering::SeqCst));
        // The window is visible: no toast.
        assert!(rig.toasts.0.lock().unwrap().is_empty());
        assert!(!rig.runner.is_running());
    }

    #[test]
    fn bench_while_stress_runs_is_busy() {
        let (launcher, release) = blocked();
        let rig = rig_with("bench-busy", launcher);
        rig.runner.start(request()).unwrap();
        assert!(matches!(rig.runner.start_bench(), Err(StartError::Busy)));
        release.send(()).unwrap();
        wait_idle(&rig.runner);
        assert!(rig.runner.bench_status().is_none());
    }

    #[test]
    fn stress_while_bench_runs_is_busy() {
        let (launcher, release) = blocked();
        let rig = rig_with("stress-busy", launcher);
        rig.runner.start_bench().unwrap();
        assert!(rig.runner.bench_running());
        assert!(!rig.runner.stress_running());
        assert!(matches!(rig.runner.start(request()), Err(StartError::Busy)));
        assert!(matches!(rig.runner.start_bench(), Err(StartError::Busy)));
        release.send(()).unwrap();
        wait_idle(&rig.runner);
        // The launcher failed: nothing saved, the page gets the reason.
        let st = rig.runner.bench_status().unwrap();
        assert_eq!(st.state, BenchState::Failed);
        assert_eq!(st.error.as_deref(), Some(StartFailure::Timeout.i18n_key()));
        assert!(rig.runner.store().list_scores().is_empty());
        assert!(rig.runner.store().list().is_empty());
    }

    fn stoppable() -> Script {
        Script {
            on_run: vec![progress(0)],
            on_stop: vec![finished(FinishReason::Stopped, 0)],
            exit_after_stop: true,
            ..Default::default()
        }
    }

    #[test]
    fn bench_stop_saves_nothing() {
        let rig = rig_with("bench-stop", scripted(stoppable()));
        rig.visible.store(false, Ordering::SeqCst);
        rig.runner.start_bench().unwrap();
        wait_bench_running(&rig.runner);
        assert!(rig.bench_mark.load(Ordering::SeqCst), "the tray dot is on");
        rig.runner.stop_bench();
        wait_idle(&rig.runner);
        assert!(rig.runner.store().list_scores().is_empty());
        assert_eq!(
            rig.runner.bench_status().unwrap().state,
            BenchState::Stopped
        );
        assert!(!rig.bench_mark.load(Ordering::SeqCst));
        // Stopped by the user: no toast, even with the window hidden.
        assert!(rig.toasts.0.lock().unwrap().is_empty());
    }

    #[test]
    fn tray_stop_stops_the_bench() {
        let rig = rig_with("bench-tray-stop", scripted(stoppable()));
        rig.runner.start_bench().unwrap();
        wait_bench_running(&rig.runner);
        // The tray's «Stop the test» calls the shared `stop`.
        rig.runner.stop();
        wait_idle(&rig.runner);
        assert_eq!(
            rig.runner.bench_status().unwrap().state,
            BenchState::Stopped
        );
    }

    #[test]
    fn quit_stops_the_bench_without_asking() {
        // The helper ignores `Stop`: the shutdown kills it.
        let rig = rig_with(
            "bench-quit",
            scripted(Script {
                on_run: vec![progress(0)],
                ..Default::default()
            }),
        );
        rig.visible.store(false, Ordering::SeqCst);
        rig.runner.start_bench().unwrap();
        wait_bench_running(&rig.runner);
        // «Quit» from the tray does not ask: only a stress test does.
        assert_eq!(
            quit_action(QuitSource::Tray, false, false, rig.runner.stress_running()),
            QuitAction::Exit
        );
        let t0 = Instant::now();
        rig.runner.shutdown(Duration::from_millis(300));
        assert!(t0.elapsed() < Duration::from_secs(2));
        assert!(!rig.runner.is_running());
        assert!(rig.runner.store().list_scores().is_empty());
        assert_eq!(
            rig.runner.bench_status().unwrap().state,
            BenchState::Stopped
        );
        assert!(rig.toasts.0.lock().unwrap().is_empty());
    }

    #[test]
    fn busy_share_leaves_out_the_launch() {
        // The first PDH sample gives nothing and is taken after the handshake, so
        // the launch (the antivirus scanning oma-load.exe) is never measured: a run
        // shorter than one poll has no share (DB7).
        let machine = FakeMachine {
            busy: Some(0.5),
            busy_primes: true,
            ..Default::default()
        };
        let rig = rig_on("bench-busy-prime", scripted(full_run()), machine);
        let id = rig.runner.start_bench().unwrap();
        wait_idle(&rig.runner);
        let s = rig.runner.store().load_score(&id).unwrap().unwrap();
        assert!(
            !s.flags.contains(&"busy_system".to_string()),
            "{:?}",
            s.flags
        );
    }

    #[test]
    fn a_silent_helper_is_hung_and_frees_the_slot() {
        let rig = rig_with(
            "bench-hung",
            scripted(Script {
                on_run: vec![progress(0)],
                ..Default::default()
            }),
        );
        let t0 = Instant::now();
        rig.runner.start_bench().unwrap();
        while rig.runner.is_running() {
            assert!(t0.elapsed() < Duration::from_secs(8), "never hung");
            std::thread::sleep(Duration::from_millis(20));
        }
        let st = rig.runner.bench_status().unwrap();
        assert_eq!(st.state, BenchState::Failed);
        assert_eq!(st.error.as_deref(), Some("hung"));
        assert!(rig.runner.store().list_scores().is_empty());
        assert!(!rig.bench_mark.load(Ordering::SeqCst));
        // The slot is free again: a stress test starts (and is shut down at once).
        assert!(rig.runner.start(request()).is_ok());
        rig.runner.shutdown(Duration::from_millis(300));
    }

    fn assert_no_toast(rig: &Rig) {
        assert!(rig.toasts.0.lock().unwrap().is_empty());
    }

    #[test]
    fn bench_toast_only_when_the_window_is_hidden() {
        let rig = rig_with("bench-toast", scripted(full_run()));
        rig.runner.start_bench().unwrap();
        wait_idle(&rig.runner);
        assert_no_toast(&rig);

        rig.visible.store(false, Ordering::SeqCst);
        let id = rig.runner.start_bench().unwrap();
        wait_idle(&rig.runner);
        let s = rig.runner.store().load_score(&id).unwrap().unwrap();
        let toasts = rig.toasts.0.lock().unwrap();
        assert_eq!(toasts.len(), 1);
        let (title, body, launch) = &toasts[0];
        assert_eq!(title, "CPU Benchmark");
        assert_eq!(
            *body,
            format!(
                "Benchmark finished: {} single core, {} multi core.",
                s.scores.single.unwrap(),
                s.scores.multi.unwrap()
            )
        );
        assert_eq!(*launch, launch_for_score_cpu());
    }

    #[test]
    fn invalid_bench_toasts_the_compute_error() {
        let error = LoadMessage::Error(oma_ipc::load::ComputeError {
            phase: 1,
            kernel: oma_ipc::load::KernelId::K5,
            isa: Isa::Avx2,
            kind: oma_ipc::load::ErrorKind::Mismatch,
            logical: Some(0),
            core: Some(0),
            iteration: 3,
            expected: 1,
            actual: 2,
            seed: 1,
            load_percent: None,
        });
        let rig = rig_with(
            "bench-invalid",
            scripted(Script {
                on_run: vec![progress(0), error, finished(FinishReason::FirstError, 1)],
                exit_after_run: true,
                ..Default::default()
            }),
        );
        rig.visible.store(false, Ordering::SeqCst);
        let id = rig.runner.start_bench().unwrap();
        wait_idle(&rig.runner);
        let s = rig.runner.store().load_score(&id).unwrap().unwrap();
        assert!(!s.valid);
        assert_eq!(
            rig.toasts.0.lock().unwrap()[0].1,
            "Benchmark not valid: calculation error."
        );
    }

    use super::super::runner::tests::{
        gpu_machine, gpu_schema, gpu_tick, FakeMachine as Fm, GPU_ID,
    };
    use crate::notifier::{launch_for_score_gpu, launch_target, LaunchTarget};

    /// One `PhaseDone` per GPU load, five windows each.
    fn gpu_phases_done() -> Vec<LoadMessage> {
        (0..oma_core::scores::GPU_LOADS.len())
            .map(|i| {
                LoadMessage::PhaseDone(PhaseDone {
                    phase: i as u32,
                    checks: 0,
                    errors: 0,
                    duration_ms: 9000,
                    skipped: None,
                    work_ms: None,
                    workers: vec![],
                    rates: vec![2e12; 5],
                })
            })
            .collect()
    }

    fn gpu_run(lead: usize, spread: Option<Duration>) -> Script {
        let mut on_run: Vec<LoadMessage> = (0..lead.max(1)).map(|_| progress(0)).collect();
        on_run.extend(gpu_phases_done());
        on_run.push(finished(FinishReason::Completed, 0));
        Script {
            on_run,
            exit_after_run: true,
            spread,
            ..Default::default()
        }
    }

    fn gpu_rig(name: &str, script: Script, busy: Option<f64>) -> Rig {
        let machine = Fm {
            gpu_busy: busy,
            ..gpu_machine()
        };
        rig_on(name, scripted(script), machine)
    }

    #[test]
    fn gpu_bench_runs_to_a_saved_score() {
        let script = gpu_run(1, None);
        let received = script.received.clone();
        let rig = gpu_rig("gpu-bench-saved", script, Some(0.0));
        let id = rig.runner.start_gpu_bench(GPU_ID).unwrap();
        wait_idle(&rig.runner);
        assert_eq!(*received.lock().unwrap(), ["run"]);
        let s = rig.runner.store().load_score(&id).unwrap().unwrap();
        assert_eq!(s.category, "gpu");
        assert!(s.valid, "{:?}", s.flags);
        assert!(s.flags.is_empty(), "{:?}", s.flags);
        assert!(s.scores.compute.is_some() && s.scores.graphics.is_some());
        assert_eq!(s.device.model, "Test RTX");
        assert_eq!(s.device.device_id.as_deref(), Some(GPU_ID));
        assert_eq!(s.device.vendor_id, Some(0x10de));
        assert_eq!(s.device.dedicated_bytes, Some(16 << 30));
        assert_eq!(s.device.integrated, Some(false));
        let st = rig.runner.bench_status().unwrap();
        assert_eq!(st.category, "gpu");
        assert_eq!(st.device_id.as_deref(), Some(GPU_ID));
        assert_eq!(st.state, BenchState::Done);
        assert_eq!(st.score_id.as_deref(), Some(id.as_str()));
        assert!(!rig.bench_mark.load(Ordering::SeqCst));
        assert!(rig.toasts.0.lock().unwrap().is_empty());
    }

    #[test]
    fn unknown_gpu_bench_is_no_gpu() {
        let rig = gpu_rig("gpu-bench-unknown", gpu_run(1, None), None);
        for id in ["gpu/pci-1002-0000", "", "cpu/0"] {
            let err = rig.runner.start_gpu_bench(id).unwrap_err();
            assert_eq!(err.wire(), "build:no_gpu", "{id}");
        }
        assert!(!rig.runner.is_running());
        assert!(rig.runner.bench_status().is_none());
    }

    #[test]
    fn gpu_bench_while_stress_runs_is_busy() {
        let (launcher, release) = blocked();
        let rig = rig_on("gpu-bench-busy", launcher, gpu_machine());
        rig.runner.start(request()).unwrap();
        assert!(matches!(
            rig.runner.start_gpu_bench(GPU_ID),
            Err(StartError::Busy)
        ));
        release.send(()).unwrap();
        wait_idle(&rig.runner);
        assert!(rig.runner.bench_status().is_none());
    }

    #[test]
    fn stress_and_cpu_bench_while_gpu_bench_runs_are_busy() {
        let (launcher, release) = blocked();
        let rig = rig_on("gpu-bench-first", launcher, gpu_machine());
        rig.runner.start_gpu_bench(GPU_ID).unwrap();
        assert!(rig.runner.bench_running());
        assert!(matches!(rig.runner.start(request()), Err(StartError::Busy)));
        assert!(matches!(rig.runner.start_bench(), Err(StartError::Busy)));
        assert!(matches!(
            rig.runner.start_gpu_bench(GPU_ID),
            Err(StartError::Busy)
        ));
        release.send(()).unwrap();
        wait_idle(&rig.runner);
        let st = rig.runner.bench_status().unwrap();
        assert_eq!(
            (st.category.as_str(), st.state),
            ("gpu", BenchState::Failed)
        );
        assert!(rig.runner.store().list_scores().is_empty());
    }

    #[test]
    fn gpu_bench_samples_come_from_gpu_sensors() {
        use oma_core::model::{Label, Sensor, SensorKind, Source, Unit};
        let mut schema = gpu_schema();
        schema.sensors.push(Sensor::new(
            GPU_ID,
            SensorKind::Flag,
            "throttle-thermal",
            Unit::Boolean,
            Label::new("throttle-thermal"),
            Source::Nvml,
        ));
        let mut tick = gpu_tick();
        tick.snapshot.values.push(Some(1.0));
        tick.quality.push(oma_core::provider::Quality::Fresh);
        let rig = gpu_rig(
            "gpu-bench-samples",
            gpu_run(4, Some(Duration::from_millis(100))),
            None,
        );
        let id = rig.runner.start_gpu_bench(GPU_ID).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while rig.runner.is_running() {
            assert!(Instant::now() < until, "the runner did not finish");
            rig.runner.on_tick(&tick, &schema);
            std::thread::sleep(Duration::from_millis(20));
        }
        let s = rig.runner.store().load_score(&id).unwrap().unwrap();
        assert_eq!(
            s.samples.first().and_then(|x| x.temp_c),
            Some(71.0),
            "the GPU core, not the CPU"
        );
        assert_eq!(s.samples[0].power_w, Some(250.0));
        assert!(s.flags.contains(&"throttling".to_string()), "{:?}", s.flags);
        assert!(s.valid);
    }

    #[test]
    fn busy_share_is_polled_every_five_seconds() {
        // About 6 s of benchmark: a read at the start and one at 5 s.
        let machine = Fm {
            gpu_busy: Some(0.5),
            ..gpu_machine()
        };
        let polls = std::sync::Arc::clone(&machine.gpu_busy_polls);
        let rig = rig_on(
            "gpu-bench-poll",
            scripted(gpu_run(120, Some(Duration::from_millis(50)))),
            machine,
        );
        let id = rig.runner.start_gpu_bench(GPU_ID).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while rig.runner.is_running() {
            assert!(Instant::now() < until, "the runner did not finish");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(*polls.lock().unwrap(), [GPU_ID, GPU_ID]);
        let s = rig.runner.store().load_score(&id).unwrap().unwrap();
        assert!(s.flags.contains(&"busy_gpu".to_string()), "{:?}", s.flags);
        // A valid score with a warning.
        assert!(s.valid);
    }

    #[test]
    fn gpu_bench_toast_opens_the_gpu_page() {
        let rig = gpu_rig("gpu-bench-toast", gpu_run(1, None), None);
        rig.visible.store(false, Ordering::SeqCst);
        let id = rig.runner.start_gpu_bench(GPU_ID).unwrap();
        wait_idle(&rig.runner);
        let s = rig.runner.store().load_score(&id).unwrap().unwrap();
        let (title, body, launch) = rig.toasts.0.lock().unwrap()[0].clone();
        assert_eq!(title, "GPU Benchmark");
        assert_eq!(
            body,
            format!(
                "GPU benchmark finished: {} compute, {} graphics.",
                s.scores.compute.unwrap(),
                s.scores.graphics.unwrap()
            )
        );
        assert_eq!(launch, launch_for_score_gpu(GPU_ID));
        assert_eq!(
            launch_target(&launch),
            Some(LaunchTarget::ScoreGpu(GPU_ID.into()))
        );
    }

    #[test]
    fn device_lost_gpu_bench_toasts_not_valid() {
        let error = LoadMessage::Error(oma_ipc::load::ComputeError {
            phase: 0,
            kernel: oma_ipc::load::KernelId::S1,
            isa: Isa::Sse2,
            kind: oma_ipc::load::ErrorKind::DeviceLost,
            logical: None,
            core: None,
            iteration: 0,
            expected: 0,
            actual: 0x887A_0005,
            seed: 1,
            load_percent: None,
        });
        let rig = gpu_rig(
            "gpu-bench-lost",
            Script {
                on_run: vec![progress(0), error, finished(FinishReason::FirstError, 1)],
                exit_after_run: true,
                ..Default::default()
            },
            None,
        );
        rig.visible.store(false, Ordering::SeqCst);
        let id = rig.runner.start_gpu_bench(GPU_ID).unwrap();
        wait_idle(&rig.runner);
        let s = rig.runner.store().load_score(&id).unwrap().unwrap();
        assert!(!s.valid);
        assert!(s.flags.contains(&"device_lost".to_string()));
        assert_eq!(
            rig.toasts.0.lock().unwrap()[0].1,
            "GPU benchmark not valid."
        );
    }
}
