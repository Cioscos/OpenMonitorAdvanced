//! The GPU benchmark controller (DH7, DH9, DH11): the CPU one's lifecycle, with the
//! windows' median and spread per load and the Compute and Graphics points.

use std::collections::BTreeMap;

use oma_ipc::load::{ErrorKind, FinishReason, LoadMessage, PhaseDone};

#[cfg(test)]
use super::bench::BenchEnd;
use super::bench::{group_over, BenchAction, BenchState, BenchStatus, SegmentState, BUSY_LIMIT};
use super::file::{Device, KernelRate, ScoreFile, Scores, FORMAT};
use super::gpu::{gpu_points, median_spread, GpuBaseline, GpuLoad, GPU_CAP_S, GPU_LOADS};
use super::lifecycle::Lifecycle;
use super::plan::{BenchMode, BenchStep};
use super::score::SCALE_POINTS;
use super::workloads::BenchKernel;
#[cfg(test)]
use crate::load::run::SILENT_PIPE_MS;
use crate::load::{Clock, SensorSample};

/// Flags in the order they are saved (DH9).
const FLAG_ORDER: [&str; 7] = [
    "battery",
    "throttling",
    "busy_gpu",
    "vram_reduced",
    "compute_error",
    "device_lost",
    "hung",
];
/// Any of these makes the score invalid.
const INVALID: [&str; 3] = ["compute_error", "device_lost", "hung"];

#[derive(Debug, Clone)]
pub struct GpuBenchContext {
    pub id: String,
    /// RFC 3339, UTC.
    pub at: String,
    pub device: Device,
    pub device_id: String,
    pub integrated: bool,
    pub on_battery: Option<bool>,
    pub baseline: &'static GpuBaseline,
    pub app_version: String,
}

pub struct GpuBenchController {
    steps: Vec<BenchStep>,
    ctx: GpuBenchContext,
    life: Lifecycle,
    segments: Vec<SegmentState>,
    step: Option<usize>,
    live_points: Option<f64>,
    /// Per load: median of the windows (display units) and spread.
    results: BTreeMap<BenchKernel, (f64, f64)>,
    shader_digest: Option<String>,
}

fn load(id: BenchKernel) -> Option<&'static GpuLoad> {
    GPU_LOADS.iter().find(|l| l.id == id)
}

impl GpuBenchController {
    pub fn new(steps: Vec<BenchStep>, ctx: GpuBenchContext, now: Clock) -> Self {
        let plan_ms = steps.len() as u64 * u64::from(GPU_CAP_S) * 1000;
        let mut c = Self {
            segments: vec![SegmentState::Pending; steps.len()],
            steps,
            life: Lifecycle::new(plan_ms, now),
            step: None,
            live_points: None,
            results: BTreeMap::new(),
            shader_digest: None,
            ctx,
        };
        if c.ctx.on_battery == Some(true) {
            c.life.flag("battery");
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
            LoadMessage::Hello(h) => {
                self.life.load_version = Some(h.version.clone());
                self.shader_digest = h.shader_digest.clone();
            }
            LoadMessage::Progress(p) => {
                let i = p.phase as usize;
                if let Some(seg) = self.segments.get_mut(i) {
                    self.step = Some(i);
                    if *seg == SegmentState::Pending {
                        *seg = SegmentState::Running;
                    }
                    // Before the first window the rate is absent: the needle holds.
                    if let Some(v) = p.rate.filter(|r| *r > 0.0).and_then(|r| self.live(i, r)) {
                        self.live_points = Some(v);
                    }
                }
            }
            LoadMessage::PhaseDone(d) => self.phase_done(d),
            LoadMessage::Notice(n) if n.code == "vram_reduced" => self.life.flag("vram_reduced"),
            // Our defect, not the GPU's: the skipped PhaseDone fails the step.
            LoadMessage::Error(e) if e.kind == ErrorKind::ReferenceInvalid => {}
            LoadMessage::Error(e) => {
                if let Some(seg) = self.segments.get_mut(e.phase as usize) {
                    *seg = SegmentState::Failed;
                }
                return self.save_invalid(match e.kind {
                    ErrorKind::DeviceLost => "device_lost",
                    ErrorKind::Hung => "hung",
                    _ => "compute_error",
                });
            }
            LoadMessage::Finished(f) => {
                return match f.reason {
                    _ if self.life.state == BenchState::Stopping => self.life.end_stopped(),
                    FinishReason::Completed => self.save(),
                    FinishReason::FirstError => self.save_invalid("compute_error"),
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

    /// Needle for a rate in base units per second of step `i`'s load (DH7).
    fn live(&self, i: usize, rate: f64) -> Option<f64> {
        let s = self.steps.get(i)?;
        self.live_units(i, rate / load(s.kernel)?.per_unit)
    }

    /// Needle for a rate in display units: `1500 x rate / reference`.
    fn live_units(&self, i: usize, units: f64) -> Option<f64> {
        let s = self.steps.get(i)?;
        let reference = self.ctx.baseline.table(s.mode)?.get(&s.kernel)?;
        let v = SCALE_POINTS * units / reference;
        v.is_finite().then_some(v)
    }

    fn phase_done(&mut self, d: &PhaseDone) {
        let i = d.phase as usize;
        let Some(step) = self.steps.get(i).copied() else {
            return;
        };
        let r = match load(step.kernel) {
            Some(l) if d.skipped.is_none() => {
                let display: Vec<f64> = d.rates.iter().map(|r| r / l.per_unit).collect();
                median_spread(&display)
            }
            _ => None,
        };
        self.segments[i] = if r.is_some() {
            SegmentState::Done
        } else {
            SegmentState::Failed
        };
        if let Some(r) = r {
            self.results.insert(step.kernel, r);
        }
        self.step = Some((i + 1).min(self.steps.len().saturating_sub(1)));
        // The needle holds the load's median only while the next step is of the same
        // group: the other gauge must not show it.
        let same_mode = self.steps.get(i + 1).is_none_or(|n| n.mode == step.mode);
        if let Some((median, _)) = r {
            self.live_points = Some(median)
                .filter(|_| same_mode)
                .and_then(|m| self.live_units(i, m));
        }
    }

    pub fn on_sample(&mut self, sample: &SensorSample, now: Clock) -> Vec<BenchAction> {
        if self.is_finished() {
            return vec![];
        }
        // Only the thermal flag: GPU Boost runs at its power limit by design (DH9).
        if sample.thermal_throttling == Some(true) {
            self.life.flag("throttling");
        }
        self.life.record(sample, now);
        vec![]
    }

    /// The largest share of a GPU engine used by another process, 0-1 (DH10).
    pub fn on_busy_gpu(&mut self, share: f64) {
        if !self.is_finished() && share > BUSY_LIMIT {
            self.life.flag("busy_gpu");
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

    /// Call it on every tick (see `BenchController::on_clock`).
    pub fn on_clock(&mut self, now: Clock) -> Vec<BenchAction> {
        self.life.on_clock(now)
    }

    pub fn on_exit(&mut self, _code: Option<i32>, now: Clock) -> Vec<BenchAction> {
        self.life.on_exit(now)
    }

    fn medians(&self) -> BTreeMap<BenchKernel, f64> {
        self.results.iter().map(|(k, (m, _))| (*k, *m)).collect()
    }

    fn save_invalid(&mut self, flag: &'static str) -> Vec<BenchAction> {
        self.life.flag(flag);
        let mut out = vec![BenchAction::SendStop];
        out.extend(self.save());
        out
    }

    fn save(&mut self) -> Vec<BenchAction> {
        let b = self.ctx.baseline;
        let medians = self.medians();
        let file = ScoreFile {
            format: FORMAT,
            id: self.ctx.id.clone(),
            at: self.ctx.at.clone(),
            category: "gpu".into(),
            score_version: b.version.clone(),
            provisional: b.provisional,
            isa: None,
            shader_digest: self.shader_digest.clone(),
            scores: Scores {
                single: None,
                multi: None,
                compute: gpu_points(&medians, BenchMode::Compute, b),
                graphics: gpu_points(&medians, BenchMode::Graphics, b),
            },
            kernels: GPU_LOADS
                .iter()
                .map(|l| {
                    let r = self.results.get(&l.id);
                    KernelRate {
                        id: l.id,
                        unit: l.unit.into(),
                        single: None,
                        multi: None,
                        value: r.map(|r| r.0),
                        spread: r.map(|r| r.1),
                    }
                })
                .collect(),
            device: Device {
                cores: 0,
                logical: 0,
                device_id: Some(self.ctx.device_id.clone()),
                integrated: Some(self.ctx.integrated),
                ..self.ctx.device.clone()
            },
            flags: self.life.flag_names(&FLAG_ORDER),
            valid: !INVALID.iter().any(|f| self.life.has_flag(f)),
            scaling: None,
            samples: self.life.samples.clone(),
            app_version: self.ctx.app_version.clone(),
            load_version: self.life.load_version.clone(),
        };
        self.life.end_saved(file)
    }

    /// Points of a group once all its steps are over (DH7), else `None`.
    fn finished_points(&self, mode: BenchMode) -> Option<u32> {
        if !group_over(&self.steps, &self.segments, mode) {
            return None;
        }
        gpu_points(&self.medians(), mode, self.ctx.baseline)
    }

    pub fn status(&self) -> BenchStatus {
        BenchStatus {
            category: "gpu".into(),
            device_id: Some(self.ctx.device_id.clone()),
            state: self.life.state,
            step: self.step,
            steps: self.steps.clone(),
            segments: self.segments.clone(),
            live_points: self.live_points,
            single: None,
            multi: None,
            compute: self.finished_points(BenchMode::Compute),
            graphics: self.finished_points(BenchMode::Graphics),
            flags: self.life.flag_names(&FLAG_ORDER),
            score_id: self.life.score_id.clone(),
            error: self.life.error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scores::{gpu_baseline, gpu_bench_plan, GPU_LOADS};
    use oma_ipc::load::{
        ComputeError, Finished, GpuTarget, Isa, LoadHello, Notice, PhaseDone, Progress,
    };

    const DEVICE: &str = "gpu-10de-2704";

    fn clock(t: u64) -> Clock {
        Clock {
            mono_ms: t,
            wall_ms: 1_000_000_000_000 + t as i64,
            asleep_ms: 0,
        }
    }

    fn steps() -> Vec<BenchStep> {
        gpu_bench_plan(
            GpuTarget {
                luid: 1,
                integrated: false,
            },
            1,
        )
        .1
    }

    fn ctx() -> GpuBenchContext {
        GpuBenchContext {
            id: "0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d".into(),
            at: "2026-10-07T10:00:00Z".into(),
            device: Device {
                model: "NVIDIA GeForce RTX 4080".into(),
                vendor_id: Some(0x10de),
                dedicated_bytes: Some(16 << 30),
                ..Device::default()
            },
            device_id: DEVICE.into(),
            integrated: false,
            on_battery: Some(false),
            baseline: gpu_baseline(),
            app_version: "0.5.0".into(),
        }
    }

    fn ctl_with(ctx: GpuBenchContext) -> GpuBenchController {
        GpuBenchController::new(steps(), ctx, clock(0))
    }

    fn ctl() -> GpuBenchController {
        ctl_with(ctx())
    }

    /// The baseline of load `i` in base units per second, times `f`.
    fn base(i: usize, f: f64) -> f64 {
        let l = &GPU_LOADS[i];
        let b = gpu_baseline();
        let r = b.compute.get(&l.id).or(b.graphics.get(&l.id)).unwrap();
        r * l.per_unit * f
    }

    fn done_rates(i: usize, rates: Vec<f64>) -> LoadMessage {
        LoadMessage::PhaseDone(PhaseDone {
            phase: i as u32,
            checks: 0,
            errors: 0,
            duration_ms: 9000,
            skipped: None,
            work_ms: None,
            workers: vec![],
            rates,

            disk: None,
        })
    }

    /// Five windows around the baseline: median = baseline x f, spread 2%.
    fn done(i: usize, f: f64) -> LoadMessage {
        let v = [0.99, 1.0, 1.01, 1.0, 1.0].map(|k| base(i, f * k));
        done_rates(i, v.to_vec())
    }

    fn skipped(i: usize, why: &str) -> LoadMessage {
        let mut d = done_rates(i, vec![]);
        if let LoadMessage::PhaseDone(p) = &mut d {
            p.skipped = Some(why.into());
        }
        d
    }

    fn progress(phase: u32, rate: Option<f64>) -> LoadMessage {
        LoadMessage::Progress(Progress {
            phase,
            phase_elapsed_ms: 1000,
            elapsed_ms: 0,
            checks: 0,
            errors: 0,
            current_core: None,
            cores: vec![],
            memory_bytes: 0,
            rate,
            load_percent: None,

            disk: None,
        })
    }

    fn finished(reason: FinishReason) -> LoadMessage {
        LoadMessage::Finished(Finished {
            reason,
            checks: 0,
            errors: 0,
        })
    }

    fn error(phase: u32, kind: ErrorKind) -> LoadMessage {
        LoadMessage::Error(ComputeError {
            phase,
            kernel: GPU_LOADS[phase as usize].kernel,
            isa: Isa::Sse2,
            kind,
            logical: None,
            core: None,
            iteration: 1,
            expected: 1,
            actual: 2,
            seed: 1,
            load_percent: None,

            transient: None,
        })
    }

    fn run(c: &mut GpuBenchController, range: std::ops::Range<usize>) {
        for i in range {
            c.on_load(&progress(i as u32, None), clock(1000 + i as u64));
            c.on_load(&done(i, 1.0), clock(1000 + i as u64));
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
    fn full_gpu_run_saves_both_scores() {
        let mut c = ctl();
        c.on_load(
            &LoadMessage::Hello(LoadHello {
                protocol_version: 5,
                version: "9".into(),
                isa: vec![],
                shader_digest: Some("0123456789abcdef".into()),
            }),
            clock(10),
        );
        run(&mut c, 0..3);
        // Compute shows as soon as its three loads are over.
        let st = c.status();
        assert_eq!((st.compute, st.graphics), (Some(1500), None));
        run(&mut c, 3..6);
        let st = c.status();
        assert_eq!((st.compute, st.graphics), (Some(1500), Some(1500)));
        let a = c.on_load(&finished(FinishReason::Completed), clock(60_000));
        let f = saved(&a);
        assert_eq!(f.format, FORMAT);
        assert_eq!(f.category, "gpu");
        assert_eq!(f.score_version, "gpu-1");
        assert_eq!(f.provisional, gpu_baseline().provisional);
        assert_eq!((f.isa, f.scaling), (None, None));
        assert_eq!(f.shader_digest.as_deref(), Some("0123456789abcdef"));
        assert_eq!(f.load_version.as_deref(), Some("9"));
        assert_eq!(
            (
                f.scores.single,
                f.scores.multi,
                f.scores.compute,
                f.scores.graphics
            ),
            (None, None, Some(1500), Some(1500))
        );
        assert_eq!(f.kernels.len(), 6);
        for (k, l) in f.kernels.iter().zip(&GPU_LOADS) {
            assert_eq!((k.id, k.unit.as_str()), (l.id, l.unit));
            assert_eq!((k.single, k.multi), (None, None));
            let want = base(GPU_LOADS.iter().position(|x| x.id == l.id).unwrap(), 1.0);
            let v = k.value.unwrap();
            assert!((v - want / l.per_unit).abs() < 1e-9 * v, "{v}");
            assert!((k.spread.unwrap() - 0.02).abs() < 1e-9);
        }
        assert_eq!(f.device.device_id.as_deref(), Some(DEVICE));
        assert_eq!(f.device.integrated, Some(false));
        assert_eq!(f.device.vendor_id, Some(0x10de));
        assert_eq!((f.device.cores, f.device.logical), (0, 0));
        assert!(f.valid && f.flags.is_empty());
        assert_eq!(
            a.last(),
            Some(&BenchAction::Finished(BenchEnd::Saved(f.id.clone())))
        );
        assert_eq!(c.status().state, BenchState::Done);
        assert_eq!(c.status().score_id.as_deref(), Some(f.id.as_str()));
        assert!(c.on_load(&progress(0, None), clock(61_000)).is_empty());
    }

    #[test]
    fn live_points_follow_progress_rate() {
        let mut c = ctl();
        c.on_load(&progress(0, Some(base(0, 0.5))), clock(1000));
        let v = c.status().live_points.unwrap();
        assert!((v - 750.0).abs() < 1e-9, "{v}");
        // A zero or absent rate holds the needle.
        c.on_load(&progress(0, Some(0.0)), clock(1100));
        c.on_load(&progress(0, None), clock(1200));
        assert_eq!(c.status().live_points, Some(v));
        // At the end of a phase the needle holds the load's median (next step: Compute).
        c.on_load(&done(0, 2.0), clock(2000));
        let v = c.status().live_points.unwrap();
        assert!((v - 3000.0).abs() < 1e-6, "{v}");
        assert_eq!(c.status().step, Some(1));
        // The last Compute load: the Graphics gauge must not show it.
        c.on_load(&done(1, 1.0), clock(3000));
        c.on_load(&done(2, 1.0), clock(4000));
        assert_eq!(c.status().live_points, None);
        // A Graphics load reads its own reference.
        c.on_load(&progress(4, Some(base(4, 1.0))), clock(5000));
        let v = c.status().live_points.unwrap();
        assert!((v - 1500.0).abs() < 1e-9, "{v}");
    }

    #[test]
    fn skipped_load_leaves_its_group_without_points() {
        let mut c = ctl();
        run(&mut c, 0..2);
        c.on_load(&skipped(2, "vram"), clock(2000));
        run(&mut c, 3..6);
        let st = c.status();
        assert_eq!(st.segments[2], SegmentState::Failed);
        assert_eq!((st.compute, st.graphics), (None, Some(1500)));
        let a = c.on_load(&finished(FinishReason::Completed), clock(3000));
        let f = saved(&a);
        assert_eq!((f.scores.compute, f.scores.graphics), (None, Some(1500)));
        assert_eq!((f.kernels[2].value, f.kernels[2].spread), (None, None));
        assert!(f.valid);
        // No valid window (zero, NaN or none at all) fails the step too.
        let mut c = ctl();
        c.on_load(&done_rates(0, vec![0.0, f64::NAN]), clock(1000));
        c.on_load(&done_rates(1, vec![]), clock(1000));
        let st = c.status();
        assert_eq!(st.segments[..2], [SegmentState::Failed; 2]);
        assert_eq!(st.live_points, None);
        // Fewer than five windows (cap or stop) still score.
        let mut c = ctl();
        c.on_load(
            &done_rates(0, vec![base(0, 1.0), base(0, 1.0)]),
            clock(1000),
        );
        assert_eq!(c.status().segments[0], SegmentState::Done);
    }

    #[test]
    fn mismatch_saves_an_invalid_score() {
        let mut c = ctl();
        run(&mut c, 0..1);
        let a = c.on_load(&error(1, ErrorKind::Mismatch), clock(2000));
        assert_eq!(a.first(), Some(&BenchAction::SendStop));
        let f = saved(&a);
        assert!(!f.valid);
        assert_eq!(f.flags, ["compute_error"]);
        assert_eq!(f.scores.compute, None);
        assert!(f.kernels[0].value.is_some());
        assert!(matches!(
            a.last(),
            Some(BenchAction::Finished(BenchEnd::Saved(_)))
        ));
        assert_eq!(c.status().segments[1], SegmentState::Failed);
        // FirstError from Finished does the same.
        let mut c = ctl();
        let a = c.on_load(&finished(FinishReason::FirstError), clock(100));
        assert!(!saved(&a).valid);
        assert_eq!(saved(&a).flags, ["compute_error"]);
    }

    #[test]
    fn device_lost_saves_an_invalid_score() {
        let mut c = ctl();
        run(&mut c, 0..3);
        let a = c.on_load(&error(3, ErrorKind::DeviceLost), clock(2000));
        let f = saved(&a);
        assert!(!f.valid);
        assert_eq!(f.flags, ["device_lost"]);
        // The loads done until then are kept.
        assert_eq!((f.scores.compute, f.scores.graphics), (Some(1500), None));
        assert_eq!(c.status().state, BenchState::Done);
        // The Finished that may follow changes nothing.
        assert!(c
            .on_load(&finished(FinishReason::FirstError), clock(2100))
            .is_empty());
    }

    #[test]
    fn hung_saves_an_invalid_score() {
        let mut c = ctl();
        let a = c.on_load(&error(4, ErrorKind::Hung), clock(2000));
        let f = saved(&a);
        assert!(!f.valid);
        assert_eq!(f.flags, ["hung"]);
        assert_eq!(c.status().segments[4], SegmentState::Failed);
    }

    #[test]
    fn reference_invalid_fails_only_the_step() {
        let mut c = ctl();
        let a = c.on_load(&error(0, ErrorKind::ReferenceInvalid), clock(1000));
        assert!(a.is_empty(), "{a:?}");
        assert_eq!(c.status().state, BenchState::Running);
        c.on_load(&skipped(0, "reference_invalid"), clock(1100));
        assert_eq!(c.status().segments[0], SegmentState::Failed);
    }

    #[test]
    fn vram_reduced_notice_flags_the_score() {
        let mut c = ctl();
        let notice = |code: &str| {
            LoadMessage::Notice(Notice {
                phase: 2,
                code: code.into(),
                value: Some(512 << 20),
            })
        };
        c.on_load(&notice("vram_allocated"), clock(100));
        assert!(c.status().flags.is_empty());
        c.on_load(&notice("vram_reduced"), clock(200));
        assert_eq!(c.status().flags, ["vram_reduced"]);
        run(&mut c, 0..6);
        let a = c.on_load(&finished(FinishReason::Completed), clock(3000));
        let f = saved(&a);
        assert!(f.valid);
        assert_eq!(f.flags, ["vram_reduced"]);
    }

    #[test]
    fn thermal_sample_flags_throttling_but_power_does_not() {
        let mut c = ctl();
        let power = SensorSample {
            temp_c: Some(110.0),
            throttling: Some(true),
            thermal_throttling: Some(false),
            ..Default::default()
        };
        c.on_sample(&power, clock(1000));
        assert!(c.status().flags.is_empty());
        let thermal = SensorSample {
            throttling: Some(true),
            thermal_throttling: Some(true),
            ..Default::default()
        };
        c.on_sample(&thermal, clock(7000));
        assert_eq!(c.status().flags, ["throttling"]);
        run(&mut c, 0..6);
        let a = c.on_load(&finished(FinishReason::Completed), clock(8000));
        let f = saved(&a);
        assert!(f.valid);
        // One sample every 5 s.
        let t: Vec<u64> = f.samples.iter().map(|s| s.t_ms).collect();
        assert_eq!(t, [1000, 7000]);
    }

    #[test]
    fn busy_gpu_above_ten_percent_flags() {
        let mut c = ctl();
        c.on_busy_gpu(0.10);
        assert!(c.status().flags.is_empty());
        c.on_busy_gpu(0.11);
        assert_eq!(c.status().flags, ["busy_gpu"]);
        // Saved in the order of DH9, battery first.
        c.on_sample(
            &SensorSample {
                thermal_throttling: Some(true),
                ..Default::default()
            },
            clock(100),
        );
        c.on_battery(true);
        assert_eq!(c.status().flags, ["battery", "throttling", "busy_gpu"]);
        let c = ctl_with(GpuBenchContext {
            on_battery: Some(true),
            ..ctx()
        });
        assert_eq!(c.status().flags, ["battery"]);
    }

    #[test]
    fn user_stop_saves_nothing() {
        let mut c = ctl();
        c.on_load(&progress(0, None), clock(100));
        assert_eq!(c.on_user_stop(clock(200)), vec![BenchAction::SendStop]);
        assert_eq!(c.status().state, BenchState::Stopping);
        let a = c.on_load(&finished(FinishReason::Completed), clock(300));
        assert_eq!(a, vec![BenchAction::Finished(BenchEnd::Stopped)]);
        assert_eq!(c.status().state, BenchState::Stopped);
        // An unanswered stop is killed after the grace.
        let mut c = ctl();
        c.on_user_stop(clock(1000));
        assert!(c.on_clock(clock(3999)).is_empty());
        assert_eq!(
            c.on_clock(clock(4000)),
            vec![BenchAction::Kill, BenchAction::Finished(BenchEnd::Stopped)]
        );
    }

    #[test]
    fn early_exit_fails_without_saving() {
        let mut c = ctl();
        c.on_load(&progress(0, None), clock(100));
        let a = c.on_exit(Some(4), clock(200));
        assert_eq!(
            a,
            vec![BenchAction::Finished(BenchEnd::Failed("exited".into()))]
        );
        assert_eq!(c.status().state, BenchState::Failed);
        assert!(c.is_finished());
    }

    #[test]
    fn silence_past_the_limit_is_hung_without_saving() {
        let mut c = ctl();
        c.on_load(&progress(0, None), clock(1000));
        assert_eq!(
            c.on_clock(clock(1001 + SILENT_PIPE_MS)),
            vec![
                BenchAction::Kill,
                BenchAction::Finished(BenchEnd::Failed("hung".into()))
            ]
        );
    }

    #[test]
    fn status_carries_category_and_device() {
        let c = ctl();
        let st = c.status();
        assert_eq!(st.category, "gpu");
        assert_eq!(st.device_id.as_deref(), Some(DEVICE));
        assert_eq!(st.state, BenchState::Starting);
        assert_eq!(st.steps.len(), 6);
        assert!(st.segments.iter().all(|s| *s == SegmentState::Pending));
        assert_eq!(
            (st.single, st.multi, st.compute, st.graphics),
            (None, None, None, None)
        );
        assert!(st
            .steps
            .iter()
            .all(|s| s.mode == BenchMode::Compute || s.mode == BenchMode::Graphics));
    }
}
