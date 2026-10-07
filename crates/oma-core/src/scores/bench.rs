//! The benchmark controller (DB5-DB7, DB11): a pure state machine driven by load
//! messages, sensor samples and a clock, like the stress `RunController`. It performs no
//! I/O; the runner executes the returned `BenchAction`s.

use std::collections::BTreeMap;

use oma_ipc::load::{ErrorKind, FinishReason, Isa, LoadMessage, PhaseDone};
use serde::Serialize;

use super::file::{Device, KernelRate, ScoreFile, Scores, FORMAT};
use super::lifecycle::Lifecycle;
use super::plan::{BenchMode, BenchStep, CAP_S, WARMUP_PAUSE_MS};
use super::score::{
    median3, per_second_to_units, points, rate, rate_from_workers, scaling, Baseline, SCALE_POINTS,
};
use super::workloads::{BenchKernel, WORKLOADS};
#[cfg(test)]
use crate::load::run::{OVERRUN_MS, SILENT_PIPE_MS};
use crate::load::{Clock, SensorSample};

pub(super) const BUSY_LIMIT: f64 = 0.10;
const HOT_WITHOUT_TJMAX_C: f64 = 93.0;
const HOT_MARGIN_C: f64 = 2.0;
/// Flags in the order they are saved.
const FLAG_ORDER: [&str; 6] = [
    "battery",
    "thermal_throttle",
    "busy_system",
    "virtual_machine",
    "no_sensors",
    "compute_error",
];

#[derive(Debug, Clone, PartialEq)]
pub enum BenchAction {
    SendStop,
    Kill,
    Save(Box<ScoreFile>),
    Finished(BenchEnd),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchEnd {
    /// The id of the saved score.
    Saved(String),
    Stopped,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchState {
    Starting,
    Running,
    Stopping,
    Done,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentState {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct BenchContext {
    pub id: String,
    /// RFC 3339, UTC.
    pub at: String,
    pub isa: Isa,
    pub device: Device,
    pub logical: u32,
    pub tjmax_c: Option<f64>,
    pub service_available: bool,
    pub on_battery: Option<bool>,
    pub hypervisor: bool,
    pub baseline: &'static Baseline,
    pub app_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchStatus {
    /// `"cpu"` or `"gpu"` (DH12).
    pub category: String,
    /// The GPU's `device_id`; `None` for the CPU.
    pub device_id: Option<String>,
    pub state: BenchState,
    pub step: Option<usize>,
    pub steps: Vec<BenchStep>,
    pub segments: Vec<SegmentState>,
    pub live_points: Option<f64>,
    pub single: Option<u32>,
    pub multi: Option<u32>,
    pub compute: Option<u32>,
    pub graphics: Option<u32>,
    pub flags: Vec<String>,
    pub score_id: Option<String>,
    pub error: Option<String>,
}

/// True once every step of `mode` is over, and `mode` has at least one step.
pub(super) fn group_over(steps: &[BenchStep], segments: &[SegmentState], mode: BenchMode) -> bool {
    let mut own = steps
        .iter()
        .zip(segments)
        .filter(|(s, _)| s.mode == mode)
        .peekable();
    own.peek().is_some() && own.all(|(_, g)| matches!(g, SegmentState::Done | SegmentState::Failed))
}

pub struct BenchController {
    steps: Vec<BenchStep>,
    ctx: BenchContext,
    life: Lifecycle,
    segments: Vec<SegmentState>,
    step: Option<usize>,
    live_points: Option<f64>,
    /// Rates of the repetitions 1-3 (the warm-up never counts).
    reps: BTreeMap<(BenchMode, BenchKernel), Vec<f64>>,
}

impl BenchController {
    pub fn new(steps: Vec<BenchStep>, ctx: BenchContext, now: Clock) -> Self {
        let plan_ms = steps
            .iter()
            .map(|s| {
                let pause = if s.rep == 0 { WARMUP_PAUSE_MS } else { 0 };
                u64::from(CAP_S) * 1000 + u64::from(pause)
            })
            .sum();
        let mut c = Self {
            segments: vec![SegmentState::Pending; steps.len()],
            steps,
            life: Lifecycle::new(plan_ms, now),
            step: None,
            live_points: None,
            reps: BTreeMap::new(),
            ctx,
        };
        if c.ctx.on_battery == Some(true) {
            c.life.flag("battery");
        }
        if c.ctx.hypervisor {
            c.life.flag("virtual_machine");
        }
        if !c.ctx.service_available {
            c.life.flag("no_sensors");
        }
        c
    }

    pub fn is_finished(&self) -> bool {
        self.life.is_finished()
    }

    pub fn on_load(&mut self, msg: &LoadMessage, now: Clock) -> Vec<BenchAction> {
        if !self.life.on_message(now) {
            return vec![];
        }
        match msg {
            LoadMessage::Hello(h) => self.life.load_version = Some(h.version.clone()),
            LoadMessage::Progress(p) => {
                let i = p.phase as usize;
                if let Some(seg) = self.segments.get_mut(i) {
                    self.step = Some(i);
                    if *seg == SegmentState::Pending {
                        *seg = SegmentState::Running;
                    }
                    // A pause, a reference or a phase start sends 0: the needle holds.
                    if let Some(v) = p.rate.filter(|r| *r > 0.0).and_then(|r| self.live(i, r)) {
                        self.live_points = Some(v);
                    }
                }
            }
            LoadMessage::PhaseDone(d) => self.phase_done(d),
            // An invalid reference is our defect, not the CPU's (as in the stress test): the
            // phase is skipped and its PhaseDone marks the step failed.
            LoadMessage::Error(e) if e.kind == ErrorKind::ReferenceInvalid => {}
            LoadMessage::Error(e) => {
                // A wrong result or a hung thread: the run is invalid either way.
                if let Some(seg) = self.segments.get_mut(e.phase as usize) {
                    *seg = SegmentState::Failed;
                }
                return self.save_invalid();
            }
            LoadMessage::Finished(f) => {
                return match f.reason {
                    _ if self.life.state == BenchState::Stopping => self.life.end_stopped(),
                    FinishReason::Completed => self.save(),
                    FinishReason::FirstError => self.save_invalid(),
                    FinishReason::Stopped => self.life.end_stopped(),
                    FinishReason::Failed => self.life.end_failed("failed"),
                };
            }
            LoadMessage::Notice(_)
            | LoadMessage::Run(_)
            | LoadMessage::Stop(_)
            | LoadMessage::Topology(_) => {}
        }
        vec![]
    }

    /// Needle: `1500 x rate / reference` of the kernel and mode of step `i` (DB6).
    fn live(&self, i: usize, rate: f64) -> Option<f64> {
        let s = self.steps.get(i)?;
        let w = WORKLOADS.iter().find(|w| w.id == s.kernel)?;
        self.live_units(i, per_second_to_units(w, rate))
    }

    /// Needle for a rate already in the workload's true units.
    fn live_units(&self, i: usize, units: f64) -> Option<f64> {
        let s = self.steps.get(i)?;
        let table = self.ctx.baseline.table(s.mode)?;
        let v = SCALE_POINTS * units / table.get(&s.kernel)?;
        v.is_finite().then_some(v)
    }

    fn phase_done(&mut self, d: &PhaseDone) {
        let i = d.phase as usize;
        let Some(step) = self.steps.get(i).copied() else {
            return;
        };
        let w = WORKLOADS.iter().find(|w| w.id == step.kernel);
        let r = match (w, d.work_ms) {
            (Some(w), _) if d.skipped.is_none() && !d.workers.is_empty() => {
                rate_from_workers(w, &d.workers)
            }
            (Some(w), Some(ms)) if d.skipped.is_none() => rate(w, d.checks, ms),
            _ => None,
        };
        // A warm-up is done as soon as it ran; a repetition needs a usable rate.
        let ok = d.skipped.is_none() && (step.rep == 0 || r.is_some());
        self.segments[i] = if ok {
            SegmentState::Done
        } else {
            SegmentState::Failed
        };
        if let (Some(r), true) = (r, step.rep > 0) {
            self.reps
                .entry((step.mode, step.kernel))
                .or_default()
                .push(r);
        }
        self.step = Some((i + 1).min(self.steps.len().saturating_sub(1)));
        // The needle holds the rep's rate only while the next step has the same mode:
        // another mode's gauge must not show it.
        let same_mode = self.steps.get(i + 1).is_none_or(|n| n.mode == step.mode);
        if r.is_some() {
            self.live_points = r.filter(|_| same_mode).and_then(|r| self.live_units(i, r));
        }
    }

    pub fn on_sample(&mut self, sample: &SensorSample, now: Clock) -> Vec<BenchAction> {
        if self.is_finished() {
            return vec![];
        }
        if let Some(t) = sample.temp_c {
            let hot = match self.ctx.tjmax_c {
                Some(tj) => t >= tj - HOT_MARGIN_C,
                None => t >= HOT_WITHOUT_TJMAX_C,
            };
            if hot {
                self.life.flag("thermal_throttle");
            }
        }
        self.life.record(sample, now);
        vec![]
    }

    /// Share of the total CPU used by the other processes, 0-1.
    pub fn on_busy_share(&mut self, share: f64) {
        if !self.is_finished() && share > BUSY_LIMIT {
            self.life.flag("busy_system");
        }
    }

    pub fn on_battery(&mut self, on_battery: bool) {
        if !self.is_finished() && on_battery {
            self.life.flag("battery");
        }
    }

    pub fn on_user_stop(&mut self, now: Clock) -> Vec<BenchAction> {
        self.life.on_user_stop(now)
    }

    /// Kills a process that ignores our stop request, or that is hung: silent
    /// for more than `SILENT_PIPE_MS`, or running past its plan by `OVERRUN_MS`.
    /// Call it on every tick.
    pub fn on_clock(&mut self, now: Clock) -> Vec<BenchAction> {
        self.life.on_clock(now)
    }

    pub fn on_exit(&mut self, _code: Option<i32>, now: Clock) -> Vec<BenchAction> {
        self.life.on_exit(now)
    }

    /// Medians of the kernels of a mode that have a rate.
    fn medians(&self, mode: BenchMode) -> BTreeMap<BenchKernel, f64> {
        WORKLOADS
            .iter()
            .filter_map(|w| Some((w.id, median3(self.reps.get(&(mode, w.id))?)?)))
            .collect()
    }

    fn save_invalid(&mut self) -> Vec<BenchAction> {
        self.life.flag("compute_error");
        let mut out = vec![BenchAction::SendStop];
        out.extend(self.save());
        out
    }

    fn save(&mut self) -> Vec<BenchAction> {
        let (single, multi) = (
            self.medians(BenchMode::Single),
            self.medians(BenchMode::Multi),
        );
        let b = self.ctx.baseline;
        let file = ScoreFile {
            format: FORMAT,
            id: self.ctx.id.clone(),
            at: self.ctx.at.clone(),
            category: "cpu".into(),
            score_version: b.version.clone(),
            provisional: b.provisional,
            isa: Some(self.ctx.isa),
            shader_digest: None,
            scores: Scores {
                single: points(&single, &b.single),
                multi: points(&multi, &b.multi),
                compute: None,
                graphics: None,
            },
            kernels: WORKLOADS
                .iter()
                .map(|w| KernelRate {
                    id: w.id,
                    unit: w.unit.into(),
                    single: single.get(&w.id).copied(),
                    multi: multi.get(&w.id).copied(),
                    value: None,
                    spread: None,
                })
                .collect(),
            device: self.ctx.device.clone(),
            flags: self.life.flag_names(&FLAG_ORDER),
            valid: !self.life.has_flag("compute_error"),
            scaling: scaling(&single, &multi, self.ctx.logical),
            samples: self.life.samples.clone(),
            app_version: self.ctx.app_version.clone(),
            load_version: self.life.load_version.clone(),
        };
        self.life.end_saved(file)
    }

    /// Points of a mode once all its steps are over (DB6), else `None`.
    fn finished_points(&self, mode: BenchMode) -> Option<u32> {
        if !group_over(&self.steps, &self.segments, mode) {
            return None;
        }
        points(&self.medians(mode), self.ctx.baseline.table(mode)?)
    }

    pub fn status(&self) -> BenchStatus {
        BenchStatus {
            category: "cpu".into(),
            device_id: None,
            state: self.life.state,
            step: self.step,
            steps: self.steps.clone(),
            segments: self.segments.clone(),
            live_points: self.live_points,
            single: self.finished_points(BenchMode::Single),
            multi: self.finished_points(BenchMode::Multi),
            compute: None,
            graphics: None,
            flags: self.life.flag_names(&FLAG_ORDER),
            score_id: self.life.score_id.clone(),
            error: self.life.error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scores::cpu_baseline;
    use oma_ipc::load::{
        ComputeError, ErrorKind, Finished, LoadHello, PhaseDone, Progress, WorkerDone,
    };

    fn clock(t: u64) -> Clock {
        Clock {
            mono_ms: t,
            wall_ms: 1_000_000_000_000 + t as i64,
            asleep_ms: 0,
        }
    }

    fn steps() -> Vec<BenchStep> {
        let mut v = vec![];
        for mode in [BenchMode::Single, BenchMode::Multi] {
            for w in &WORKLOADS {
                for rep in 0..=3 {
                    v.push(BenchStep {
                        kernel: w.id,
                        mode,
                        rep,
                    });
                }
            }
        }
        v
    }

    fn ctx() -> BenchContext {
        BenchContext {
            id: "0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d".into(),
            at: "2026-10-07T10:00:00Z".into(),
            isa: Isa::Avx2,
            device: Device {
                model: "CPU".into(),
                cores: 8,
                logical: 16,
                ..Device::default()
            },
            logical: 16,
            tjmax_c: Some(89.0),
            service_available: true,
            on_battery: Some(false),
            hypervisor: false,
            baseline: cpu_baseline(),
            app_version: "0.5.0".into(),
        }
    }

    fn ctl_with(ctx: BenchContext) -> BenchController {
        BenchController::new(steps(), ctx, clock(0))
    }

    fn ctl() -> BenchController {
        ctl_with(ctx())
    }

    fn done(i: usize, work_ms: Option<u64>) -> LoadMessage {
        let s = steps()[i];
        let w = WORKLOADS.iter().find(|w| w.id == s.kernel).unwrap();
        LoadMessage::PhaseDone(PhaseDone {
            phase: i as u32,
            checks: w.iterations,
            errors: 0,
            duration_ms: 1000,
            skipped: None,
            work_ms,
            workers: vec![],
            rates: vec![],
        })
    }

    fn progress(phase: u32, rate: Option<f64>) -> LoadMessage {
        progress_at(phase, 1000, rate)
    }

    fn progress_at(phase: u32, phase_elapsed_ms: u64, rate: Option<f64>) -> LoadMessage {
        LoadMessage::Progress(Progress {
            phase,
            phase_elapsed_ms,
            elapsed_ms: 0,
            checks: 1,
            errors: 0,
            current_core: None,
            cores: vec![],
            memory_bytes: 0,
            rate,
            load_percent: None,
        })
    }

    fn finished(reason: FinishReason) -> LoadMessage {
        LoadMessage::Finished(Finished {
            reason,
            checks: 1,
            errors: 0,
        })
    }

    /// Every step done with the given work time.
    fn run_all(c: &mut BenchController, work_ms: u64) {
        for i in 0..48 {
            c.on_load(&progress(i as u32, None), clock(1000 + i as u64));
            c.on_load(&done(i, Some(work_ms)), clock(1000 + i as u64));
        }
    }

    fn saved(a: &[BenchAction]) -> &ScoreFile {
        a.iter()
            .find_map(|x| match x {
                BenchAction::Save(f) => Some(&**f),
                _ => None,
            })
            .expect("a Save action")
    }

    #[test]
    fn full_run_saves_both_scores() {
        let mut c = ctl();
        c.on_load(
            &LoadMessage::Hello(LoadHello {
                protocol_version: 2,
                version: "9".into(),
                isa: vec![],
                shader_digest: None,
            }),
            clock(10),
        );
        run_all(&mut c, 1000);
        let st = c.status();
        assert!(st.single.is_some() && st.multi.is_some());
        let a = c.on_load(&finished(FinishReason::Completed), clock(60_000));
        let f = saved(&a);
        assert_eq!(f.format, FORMAT);
        assert_eq!(f.score_version, "cpu-1");
        assert_eq!(f.provisional, cpu_baseline().provisional);
        assert_eq!(f.scores.single, st.single);
        assert_eq!(f.scores.multi, st.multi);
        assert!(f.valid && f.flags.is_empty());
        assert!(f.scaling.is_some());
        assert_eq!(f.kernels.len(), 6);
        assert_eq!(f.load_version.as_deref(), Some("9"));
        // A CPU file writes `isa` and leaves the GPU fields null (DH11).
        assert_eq!(f.isa, Some(Isa::Avx2));
        assert_eq!((f.scores.compute, f.scores.graphics), (None, None));
        assert!(f
            .kernels
            .iter()
            .all(|k| k.value.is_none() && k.spread.is_none()));
        assert_eq!(
            (f.device.device_id.as_ref(), f.shader_digest.as_ref()),
            (None, None)
        );
        assert_eq!(
            a.last(),
            Some(&BenchAction::Finished(BenchEnd::Saved(f.id.clone())))
        );
        let st = c.status();
        assert_eq!(st.state, BenchState::Done);
        assert_eq!(st.score_id.as_deref(), Some(f.id.as_str()));
        assert!(c.on_load(&progress(0, None), clock(61_000)).is_empty());
    }

    #[test]
    fn warmup_reps_do_not_count() {
        let mut c = ctl();
        // Ntt single: a very fast warm-up, then reps of 1, 2 and 4 s.
        for (i, ms) in [1, 1000, 2000, 4000].into_iter().enumerate() {
            c.on_load(&done(i, Some(ms)), clock(1000));
        }
        let a = c.on_load(&finished(FinishReason::Completed), clock(2000));
        let want = rate(&WORKLOADS[0], WORKLOADS[0].iterations, 2000).unwrap();
        assert_eq!(saved(&a).kernels[0].single, Some(want));
    }

    #[test]
    fn capped_reps_still_score() {
        let mut c = ctl();
        run_all(&mut c, 30_000);
        let a = c.on_load(&finished(FinishReason::Completed), clock(2000));
        let f = saved(&a);
        assert!(f.scores.single.is_some() && f.scores.multi.is_some() && f.valid);
    }

    #[test]
    fn live_points_follow_progress_rate() {
        let mut c = ctl();
        let b = cpu_baseline();
        let ntt = &WORKLOADS[0];
        // Progress.rate is iterations per second of all threads; Ntt counts in Mop/s.
        let raw = |units: f64| units * 1e6 / ntt.work_per_iteration;
        // Step 0 is the Ntt warm-up in single mode: the reference is single/ntt.
        c.on_load(
            &progress(0, Some(raw(b.single[&BenchKernel::Ntt]))),
            clock(1000),
        );
        let v = c.status().live_points.unwrap();
        assert!((v - 1500.0).abs() < 1e-9, "{v}");
        // Step 24 is the Ntt warm-up in multi mode.
        c.on_load(
            &progress(24, Some(raw(b.multi[&BenchKernel::Ntt] / 2.0))),
            clock(2000),
        );
        let v = c.status().live_points.unwrap();
        assert!((v - 750.0).abs() < 1e-9, "{v}");
        c.on_load(&progress(24, None), clock(3000));
        assert_eq!(c.status().step, Some(24));
    }

    #[test]
    fn needle_ignores_zero_rate_progress() {
        let mut c = ctl();
        c.on_load(&progress_at(0, 1500, Some(0.0)), clock(1000));
        assert_eq!(c.status().live_points, None);
        c.on_load(&progress(0, Some(1e6)), clock(2000));
        let v = c.status().live_points.unwrap();
        // A pause, a reference or a phase start (rate 0 or absent) leaves it alone.
        c.on_load(&progress_at(1, 0, Some(0.0)), clock(3000));
        c.on_load(&progress_at(1, 0, None), clock(3000));
        assert_eq!(c.status().live_points, Some(v));
        assert_eq!(c.status().step, Some(1));
    }

    fn done_with(i: usize, workers: Vec<WorkerDone>) -> LoadMessage {
        let mut d = done(i, Some(1000));
        if let LoadMessage::PhaseDone(p) = &mut d {
            p.workers = workers;
        }
        d
    }

    fn wd(iterations: u64, work_ms: u64) -> WorkerDone {
        WorkerDone {
            logical: 0,
            iterations,
            work_ms,
        }
    }

    #[test]
    fn needle_is_not_carried_over_to_the_next_mode() {
        let mut c = ctl();
        // Step 23 is the last single rep, step 24 the first multi warm-up.
        c.on_load(&done_with(22, vec![wd(1000, 1000)]), clock(1000));
        assert!(c.status().live_points.is_some());
        c.on_load(&done_with(23, vec![wd(1000, 1000)]), clock(2000));
        assert_eq!(c.status().step, Some(24));
        assert_eq!(c.status().live_points, None);
    }

    #[test]
    fn multi_score_uses_the_per_thread_sum() {
        let mut c = ctl();
        // Multi Ntt reps (steps 25-27): two threads at 1000 it/s and 500 it/s = 1500 it/s,
        // where checks / work_ms would say something else.
        for i in 24..28 {
            let w = vec![wd(1000, 1000), wd(1000, 2000)];
            c.on_load(&done_with(i, w), clock(1000));
        }
        let a = c.on_load(&finished(FinishReason::Completed), clock(2000));
        let want = per_second_to_units(&WORKLOADS[0], 1500.0);
        let got = saved(&a).kernels[0].multi.unwrap();
        assert!((got - want).abs() < 1e-9, "{got} {want}");
        assert_ne!(
            Some(got),
            rate(&WORKLOADS[0], WORKLOADS[0].iterations, 1000)
        );
    }

    #[test]
    fn needle_holds_the_rep_rate_after_phase_done() {
        let mut c = ctl();
        // No Progress with a rate at all: the rep's own workers set the needle.
        let mut d = done(0, Some(1000));
        if let LoadMessage::PhaseDone(p) = &mut d {
            p.workers = vec![
                WorkerDone {
                    logical: 0,
                    iterations: 1000,
                    work_ms: 1000,
                },
                WorkerDone {
                    logical: 1,
                    iterations: 1000,
                    work_ms: 2000,
                },
            ];
        }
        c.on_load(&d, clock(1000));
        let w = &WORKLOADS[0];
        let want = SCALE_POINTS * per_second_to_units(w, 1500.0)
            / cpu_baseline().single[&BenchKernel::Ntt];
        let v = c.status().live_points.unwrap();
        assert!((v - want).abs() < 1e-9, "{v} {want}");
        // A zero-rate Progress of the pause does not take it away.
        c.on_load(&progress_at(1, 0, Some(0.0)), clock(2000));
        assert_eq!(c.status().live_points, Some(v));
    }

    #[test]
    fn error_saves_an_invalid_score() {
        let mut c = ctl();
        for i in 0..8 {
            c.on_load(&done(i, Some(1000)), clock(1000));
        }
        let a = c.on_load(
            &LoadMessage::Error(ComputeError {
                phase: 8,
                kernel: oma_ipc::load::KernelId::Hash,
                isa: Isa::Avx2,
                kind: ErrorKind::Mismatch,
                logical: Some(0),
                core: Some(0),
                iteration: 1,
                expected: 1,
                actual: 2,
                seed: 1,
                load_percent: None,
            }),
            clock(2000),
        );
        let f = saved(&a);
        assert!(!f.valid);
        assert!(f.flags.contains(&"compute_error".to_string()));
        assert_eq!((f.scores.single, f.scores.multi), (None, None));
        assert!(matches!(
            a.last(),
            Some(BenchAction::Finished(BenchEnd::Saved(_)))
        ));
        assert_eq!(c.status().segments[8], SegmentState::Failed);
        // FirstError from Finished does the same.
        let mut c = ctl();
        let a = c.on_load(&finished(FinishReason::FirstError), clock(100));
        assert!(!saved(&a).valid);
    }

    #[test]
    fn reference_invalid_is_our_defect_not_a_compute_error() {
        let mut c = ctl();
        let a = c.on_load(
            &LoadMessage::Error(ComputeError {
                phase: 1,
                kernel: oma_ipc::load::KernelId::K5,
                isa: Isa::Avx2,
                kind: ErrorKind::ReferenceInvalid,
                logical: None,
                core: None,
                iteration: 0,
                expected: 0,
                actual: 0,
                seed: 1,
                load_percent: None,
            }),
            clock(1000),
        );
        assert!(a.is_empty(), "{a:?}");
        assert_eq!(c.status().state, BenchState::Running);
        // The skipped PhaseDone that follows marks the step failed.
        let mut d = done(1, None);
        if let LoadMessage::PhaseDone(p) = &mut d {
            p.skipped = Some("reference_invalid".into());
        }
        c.on_load(&d, clock(1100));
        assert_eq!(c.status().segments[1], SegmentState::Failed);
    }

    #[test]
    fn user_stop_saves_nothing() {
        let mut c = ctl();
        c.on_load(&progress(0, None), clock(100));
        assert_eq!(c.on_user_stop(clock(200)), vec![BenchAction::SendStop]);
        assert_eq!(c.status().state, BenchState::Stopping);
        let a = c.on_load(&finished(FinishReason::Stopped), clock(300));
        assert_eq!(a, vec![BenchAction::Finished(BenchEnd::Stopped)]);
        assert_eq!(c.status().state, BenchState::Stopped);
        // An unanswered stop is killed after the grace.
        let mut c = ctl();
        c.on_user_stop(clock(1000));
        assert!(c.on_clock(clock(3999)).is_empty());
        let a = c.on_clock(clock(4000));
        assert_eq!(
            a,
            vec![BenchAction::Kill, BenchAction::Finished(BenchEnd::Stopped)]
        );
    }

    #[test]
    fn early_exit_fails_without_saving() {
        let mut c = ctl();
        c.on_load(&progress(0, None), clock(100));
        let a = c.on_exit(Some(-1), clock(200));
        assert_eq!(
            a,
            vec![BenchAction::Finished(BenchEnd::Failed("exited".into()))]
        );
        assert_eq!(c.status().state, BenchState::Failed);
        assert_eq!(c.status().error.as_deref(), Some("exited"));
    }

    fn hot(t: f64) -> SensorSample {
        SensorSample {
            temp_c: Some(t),
            ..Default::default()
        }
    }

    #[test]
    fn hot_sample_flags_thermal_throttle() {
        let mut c = ctl();
        c.on_sample(&hot(86.9), clock(1000));
        assert!(c.status().flags.is_empty());
        c.on_sample(&hot(87.0), clock(2000));
        assert_eq!(c.status().flags, ["thermal_throttle"]);
        // Without Tjmax the line is 93 C.
        let mut c = ctl_with(BenchContext {
            tjmax_c: None,
            ..ctx()
        });
        c.on_sample(&hot(92.9), clock(1000));
        assert!(c.status().flags.is_empty());
        c.on_sample(&hot(93.0), clock(2000));
        assert_eq!(c.status().flags, ["thermal_throttle"]);
    }

    #[test]
    fn samples_are_kept_every_five_seconds() {
        let mut c = ctl();
        for t in [0, 1000, 4999, 5000, 9000, 10_000] {
            c.on_sample(&hot(50.0), clock(t));
        }
        run_all(&mut c, 1000);
        let a = c.on_load(&finished(FinishReason::Completed), clock(11_000));
        let t: Vec<u64> = saved(&a).samples.iter().map(|s| s.t_ms).collect();
        assert_eq!(t, [0, 5000, 10_000]);
    }

    #[test]
    fn busy_share_above_ten_percent_flags() {
        let mut c = ctl();
        c.on_busy_share(0.10);
        assert!(c.status().flags.is_empty());
        c.on_busy_share(0.11);
        assert_eq!(c.status().flags, ["busy_system"]);
    }

    #[test]
    fn battery_and_hypervisor_and_no_service_flags() {
        let mut c = ctl_with(BenchContext {
            on_battery: Some(true),
            hypervisor: true,
            service_available: false,
            ..ctx()
        });
        assert_eq!(
            c.status().flags,
            ["battery", "virtual_machine", "no_sensors"]
        );
        let mut c2 = ctl();
        c2.on_battery(false);
        assert!(c2.status().flags.is_empty());
        c2.on_battery(true);
        assert_eq!(c2.status().flags, ["battery"]);
        // Flags travel to the file, still valid.
        run_all(&mut c, 1000);
        let a = c.on_load(&finished(FinishReason::Completed), clock(5000));
        assert!(saved(&a).valid);
        assert_eq!(saved(&a).flags.len(), 3);
    }

    #[test]
    fn segments_follow_the_steps() {
        let mut c = ctl();
        assert!(c
            .status()
            .segments
            .iter()
            .all(|s| *s == SegmentState::Pending));
        c.on_load(&progress(0, None), clock(100));
        assert_eq!(c.status().segments[0], SegmentState::Running);
        c.on_load(&done(0, None), clock(200));
        assert_eq!(c.status().segments[0], SegmentState::Done);
        // A repetition without a usable rate is failed and is not scored.
        c.on_load(&done(1, Some(0)), clock(300));
        assert_eq!(c.status().segments[1], SegmentState::Failed);
        assert_eq!(c.status().step, Some(2));
        assert_eq!(c.status().segments.len(), 48);
        assert_eq!(c.status().single, None);
    }

    #[test]
    fn silent_pipe_kills_and_fails_without_saving() {
        let mut c = ctl();
        c.on_load(&progress(0, None), clock(1000));
        // Within the silence limit: nothing.
        assert!(c.on_clock(clock(1000 + SILENT_PIPE_MS)).is_empty());
        let a = c.on_clock(clock(1001 + SILENT_PIPE_MS));
        assert_eq!(
            a,
            vec![
                BenchAction::Kill,
                BenchAction::Finished(BenchEnd::Failed("hung".into()))
            ]
        );
        let st = c.status();
        assert_eq!(st.state, BenchState::Failed);
        assert_eq!(st.error.as_deref(), Some("hung"));
        assert!(c.on_clock(clock(60_000)).is_empty());
    }

    #[test]
    fn a_message_resets_the_silence() {
        let mut c = ctl();
        c.on_load(&progress(0, None), clock(0));
        c.on_load(&progress(0, None), clock(4_000));
        assert!(c.on_clock(clock(8_000)).is_empty());
    }

    #[test]
    fn silence_while_stopping_is_not_hung() {
        let mut c = ctl();
        c.on_load(&progress(0, None), clock(0));
        c.on_user_stop(clock(100));
        // The stop grace (3 s) ends it as stopped, never as hung.
        let a = c.on_clock(clock(SILENT_PIPE_MS + 1_000));
        assert_eq!(
            a,
            vec![BenchAction::Kill, BenchAction::Finished(BenchEnd::Stopped)]
        );
    }

    #[test]
    fn running_past_the_plan_is_hung() {
        let mut c = ctl();
        // 48 caps of 30 s plus 12 warm-up pauses of 2 s.
        let plan_ms = 48 * 30_000 + 12 * 2_000;
        let mut t = 0;
        while t <= plan_ms + OVERRUN_MS {
            c.on_load(&progress(0, None), clock(t));
            assert!(c.on_clock(clock(t)).is_empty(), "at {t}");
            t += 1_000;
        }
        c.on_load(&progress(0, None), clock(t));
        assert_eq!(
            c.on_clock(clock(t)),
            vec![
                BenchAction::Kill,
                BenchAction::Finished(BenchEnd::Failed("hung".into()))
            ]
        );
    }
}
