//! The CPU benchmark (M8a2) on the runner's `oma-perf-runner` thread: the
//! plan of `bench_plan`, the helper and the pure `BenchController`, fed every
//! 250 ms with the helper's messages and the sampler's CPU readings, and every
//! 5 s with the battery and the other processes' CPU share (DB7). It shares
//! the runner's `active` slot with the stress test (DB8): one job at a time,
//! the same stop and shutdown. A stopped benchmark saves nothing (DB9).

use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oma_core::load::{resolve_cpu_sensors, Clock, SensorSample};
use oma_core::scores::{
    bench_plan, cpu_baseline, BenchAction, BenchContext, BenchController, BenchEnd, BenchState,
    BenchStatus, Device, ScoreFile,
};
use oma_ipc::load::{Isa, LoadMessage, StopRequest};

use super::host::{HostEvent, StartFailure};
use super::runner::{
    clock_at, core_count, lock, plan_message, seed_of, unix_now_ms, Active, Control, LoadLink,
    PerformanceRunner, RunnerDeps, StartError, EXIT_GRACE, SAMPLE_QUEUE, TICK,
};
use super::store::to_rfc3339;
use crate::i18n::t;
use crate::notifier::launch_for_score_cpu;
use crate::tray::language_for;

/// Emitted to the main window on every state change and twice a second during
/// the benchmark, only while a window is open; the payload is a [`BenchStatus`].
pub const EVENT_BENCH: &str = "performance-bench";

const EMIT_EVERY: Duration = Duration::from_millis(500);
/// Battery and other processes' CPU (DB7).
const POLL_EVERY: Duration = Duration::from_secs(5);

impl PerformanceRunner {
    /// Starts the CPU benchmark; its score id, or `Busy` while a test or a
    /// benchmark runs. A helper that does not start saves nothing: the status
    /// ends `failed` with the reason's i18n key in `error`.
    pub fn start_bench(&self) -> Result<String, StartError> {
        let mut active = lock(&self.active);
        if active.as_ref().is_some_and(Active::running) {
            return Err(StartError::Busy);
        }
        if let Some(old) = active.take() {
            let _ = old.thread.join();
        }
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
        // Set before the thread runs, so a later status from it is never overwritten.
        let previous = lock(&self.bench_status).replace(ctl.status());

        let (samples, sample_rx) = mpsc::sync_channel(SAMPLE_QUEUE);
        let control = Arc::new(Control::default());
        let worker = BenchWorker {
            deps: Arc::clone(deps),
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
                    worker.run(ctl, plan, logical)
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

    /// The benchmark in progress or the last one; `None` before any.
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
    ctl: BenchController,
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

    fn run(mut self, mut ctl: BenchController, plan: oma_ipc::load::Plan, logical: u32) {
        #[cfg(windows)]
        let _awake = oma_win::power::KeepAwake::new();
        self.publish(ctl.status());
        let run = match plan_message(plan) {
            Ok(run) => run,
            Err(failure) => return self.failed(&ctl, &failure),
        };
        // A priming sample: PDH needs two collections, and the handshake separates them,
        // so the poll after the launch already has the share (DB7: "all'avvio").
        let mut busy = self.deps.machine.busy_probe(logical);
        let _ = busy();
        let (tx, rx) = mpsc::channel();
        let host = match (self.deps.launcher)(tx) {
            Ok(host) => host,
            Err(failure) => return self.failed(&ctl, &failure),
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
    fn failed(mut self, ctl: &BenchController, failure: &StartFailure) {
        tracing::warn!(%failure, "the benchmark did not start");
        let mut status = ctl.status();
        status.state = BenchState::Failed;
        status.error = Some(failure.i18n_key().into());
        self.publish(status);
    }

    /// The battery and the other processes' CPU share (DB7).
    fn poll(&self, r: &mut BenchRun, busy: &mut dyn FnMut() -> Option<f64>) {
        if let Some(on) = self.deps.machine.on_battery() {
            r.ctl.on_battery(on);
        }
        if let Some(share) = busy() {
            r.ctl.on_busy_share(share);
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
        let body = if file.valid {
            let points =
                |p: Option<u32>| p.map_or_else(|| "\u{2013}".to_owned(), |p| p.to_string());
            let (single, multi) = (points(file.scores.single), points(file.scores.multi));
            t(
                lang,
                "performance.toast.benchDone",
                &[("single", single.as_str()), ("multi", multi.as_str())],
            )
        } else {
            t(lang, "performance.toast.benchInvalid", &[])
        };
        self.deps.toaster.show(
            t(lang, "performance.score.title", &[]),
            body,
            launch_for_score_cpu(),
        );
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
        assert_eq!(s.isa, Isa::Avx2);
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
    fn busy_share_is_known_from_the_start() {
        // The first PDH sample gives nothing: it is taken before the launch, so
        // the poll right after it already has a value (DB7).
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
            s.flags.contains(&"busy_system".to_string()),
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
}
