//! The disk benchmark controller (DC5, DC12, DC13): the lifecycle of the CPU and GPU
//! ones, the best of the three measures of each test, and the read and write needles.

use std::collections::BTreeMap;

use oma_ipc::load::{FinishReason, IoStats, LoadMessage, PhaseDone};

#[cfg(test)]
use super::bench::BenchEnd;
use super::bench::{BenchAction, BenchState, BenchStatus, SegmentState};
use super::disk::{disk_points, disk_tests, DiskBaseline, DiskProfile};
use super::file::{Device, DiskRate, KernelRate, ScoreFile, Scores, FORMAT};
use super::lifecycle::Lifecycle;
use super::plan::{BenchMode, BenchStep};
use super::workloads::BenchKernel;
#[cfg(test)]
use crate::load::run::SILENT_PIPE_MS;
use crate::load::{Clock, SensorSample, LOAD_EXIT_IO};

/// Flags in the order they are saved (DC12).
const FLAG_ORDER: [&str; 8] = [
    "battery",
    "thermal",
    "other_io",
    "compressible",
    "virtual_disk",
    "removable",
    "io_error",
    "disk_full",
];
/// Any of these makes the score invalid.
const INVALID: [&str; 2] = ["io_error", "disk_full"];

/// Phase length plus the pause before it, and the fill's bookkeeping length (DC5).
const PHASE_MS: u64 = 10_000;
const FILL_MS: u64 = 900_000;

#[derive(Debug, Clone)]
pub struct DiskBenchContext {
    pub id: String,
    /// RFC 3339, UTC.
    pub at: String,
    pub device: Device,
    pub device_id: String,
    pub profile: DiskProfile,
    pub compressible: bool,
    pub virtual_disk: bool,
    pub removable: bool,
    /// The disk's temperature threshold, degrees Celsius.
    pub threshold_c: f64,
    pub on_battery: Option<bool>,
    pub baseline: &'static DiskBaseline,
    pub app_version: String,
}

/// The best measure of a test and direction, with the mean of `io_bps` sampled in it.
#[derive(Debug, Clone)]
struct Best {
    rate: DiskRate,
    io_bps: Option<f64>,
}

pub struct DiskBenchController {
    steps: Vec<BenchStep>,
    ctx: DiskBenchContext,
    life: Lifecycle,
    segments: Vec<SegmentState>,
    step: Option<usize>,
    live_read: Option<f64>,
    live_write: Option<f64>,
    best: BTreeMap<(BenchMode, BenchKernel), Best>,
    /// The step now running and the sum and count of its `io_bps` samples.
    active: Option<(usize, f64, u32)>,
}

fn disk_rate(s: &IoStats) -> Option<DiskRate> {
    if s.bytes == 0 || s.elapsed_us == 0 {
        return None;
    }
    let us = s.elapsed_us as f64;
    let r = DiskRate {
        mbs: s.bytes as f64 / us,
        iops: s.ios as f64 * 1e6 / us,
        mean_lat_us: s.mean_lat_us,
        p99_lat_us: s.p99_lat_us,
    };
    [r.mbs, r.iops, r.mean_lat_us, r.p99_lat_us]
        .iter()
        .all(|v| v.is_finite())
        .then_some(r)
}

impl DiskBenchController {
    pub fn new(steps: Vec<BenchStep>, ctx: DiskBenchContext, now: Clock) -> Self {
        let plan_ms = steps
            .iter()
            .map(|s| {
                if s.kernel == BenchKernel::DiskFill {
                    FILL_MS
                } else {
                    PHASE_MS
                }
            })
            .sum();
        let mut c = Self {
            segments: vec![SegmentState::Pending; steps.len()],
            steps,
            life: Lifecycle::new(plan_ms, now),
            step: None,
            live_read: None,
            live_write: None,
            best: BTreeMap::new(),
            active: None,
            ctx,
        };
        for (on, flag) in [
            (c.ctx.on_battery == Some(true), "battery"),
            (c.ctx.compressible, "compressible"),
            (c.ctx.virtual_disk, "virtual_disk"),
            (c.ctx.removable, "removable"),
        ] {
            if on {
                c.life.flag(flag);
            }
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
        let stopping = self.life.state == BenchState::Stopping;
        match msg {
            LoadMessage::Hello(h) => self.life.load_version = Some(h.version.clone()),
            LoadMessage::Progress(p) => {
                let i = p.phase as usize;
                if let Some(seg) = self.segments.get_mut(i) {
                    self.step = Some(i);
                    if *seg == SegmentState::Pending {
                        *seg = SegmentState::Running;
                    }
                    if self.active.is_none_or(|a| a.0 != i) {
                        self.active = Some((i, 0.0, 0));
                    }
                    if let (Some(d), Some(s)) = (&p.disk, self.steps.get(i)) {
                        let v = |bps: f64| (bps.is_finite() && bps >= 0.0).then_some(bps / 1e6);
                        match (s.kernel, s.mode) {
                            // The fill moves no needle: the UI shows its own text.
                            (BenchKernel::DiskFill, _) => {}
                            (_, BenchMode::Read) => {
                                self.live_read = v(d.read_bps).or(self.live_read)
                            }
                            _ => self.live_write = v(d.write_bps).or(self.live_write),
                        }
                    }
                }
            }
            LoadMessage::PhaseDone(d) => self.phase_done(d),
            // A user stop saves nothing, whatever the helper reports meanwhile.
            LoadMessage::Notice(n) if n.code == "disk_full" && !stopping => {
                return self.save_invalid("disk_full");
            }
            LoadMessage::Error(e) if !stopping => {
                if let Some(seg) = self.segments.get_mut(e.phase as usize) {
                    *seg = SegmentState::Failed;
                }
                return self.save_invalid("io_error");
            }
            LoadMessage::Finished(f) => {
                return match f.reason {
                    _ if stopping => self.life.end_stopped(),
                    FinishReason::Completed => self.save(),
                    FinishReason::FirstError => self.save_invalid("io_error"),
                    FinishReason::Stopped => self.life.end_stopped(),
                    FinishReason::Failed => self.life.end_failed("failed"),
                };
            }
            LoadMessage::Notice(_)
            | LoadMessage::Error(_)
            | LoadMessage::Run(_)
            | LoadMessage::Stop(_)
            | LoadMessage::Topology(_) => {}
        }
        vec![]
    }

    fn phase_done(&mut self, d: &PhaseDone) {
        let i = d.phase as usize;
        let Some(step) = self.steps.get(i).copied() else {
            return;
        };
        let io_bps = match self.active.take() {
            Some((a, sum, n)) if a == i && n > 0 => Some(sum / f64::from(n)),
            _ => None,
        };
        let stats = d.disk.as_ref().map(|s| match step.mode {
            BenchMode::Read => &s.read,
            _ => &s.write,
        });
        let measured = stats.and_then(disk_rate);
        // The fill and the warm-up are not results: they only have to have run.
        let counts = step.kernel != BenchKernel::DiskFill && step.rep > 0;
        let ok = d.skipped.is_none() && (!counts || measured.is_some());
        self.segments[i] = if ok {
            SegmentState::Done
        } else {
            SegmentState::Failed
        };
        self.step = Some((i + 1).min(self.steps.len().saturating_sub(1)));
        let (true, true, Some(rate)) = (ok, counts, measured) else {
            return;
        };
        let key = (step.mode, step.kernel);
        if self.best.get(&key).is_none_or(|b| rate.mbs > b.rate.mbs) {
            self.best.insert(key, Best { rate, io_bps });
        }
        // At the end of a measure the needle holds the test's result.
        let held = Some(self.best[&key].rate.mbs);
        match step.mode {
            BenchMode::Read => self.live_read = held,
            _ => self.live_write = held,
        }
        if step.rep == 3 {
            self.check_other_io(key);
        }
    }

    /// `other_io` (DC12): the best measure's mean `io_bps` is over its rate by more than
    /// 5% and by at least 1 MB/s.
    fn check_other_io(&mut self, key: (BenchMode, BenchKernel)) {
        let Some(b) = self.best.get(&key) else {
            return;
        };
        let rate = b.rate.mbs * 1e6;
        if b.io_bps
            .is_some_and(|io| io * 100.0 > rate * 105.0 && io - rate >= 1e6)
        {
            self.life.flag("other_io");
        }
    }

    pub fn on_sample(&mut self, sample: &SensorSample, now: Clock) -> Vec<BenchAction> {
        if self.is_finished() {
            return vec![];
        }
        if sample.temp_c.is_some_and(|t| t >= self.ctx.threshold_c) {
            self.life.flag("thermal");
        }
        if let (Some(a), Some(io)) = (
            self.active.as_mut(),
            sample.io_bps.filter(|v| v.is_finite()),
        ) {
            a.1 += io;
            a.2 += 1;
        }
        self.life.record(sample, now);
        vec![]
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

    /// Exit 5 is the persistent I/O error: an invalid score; any other early exit fails.
    pub fn on_exit(&mut self, code: Option<i32>, now: Clock) -> Vec<BenchAction> {
        if code == Some(LOAD_EXIT_IO)
            && !self.is_finished()
            && self.life.state != BenchState::Stopping
        {
            self.life.flag("io_error");
            return self.save();
        }
        self.life.on_exit(now)
    }

    fn save_invalid(&mut self, flag: &'static str) -> Vec<BenchAction> {
        self.life.flag(flag);
        let mut out = vec![BenchAction::SendStop];
        out.extend(self.save());
        out
    }

    fn mbs(&self, mode: BenchMode, id: BenchKernel) -> Option<f64> {
        self.best.get(&(mode, id)).map(|b| b.rate.mbs)
    }

    /// Points once the eight B1 measures are in (never for B2).
    fn points(&self) -> Option<u32> {
        if self.ctx.profile != DiskProfile::B1 {
            return None;
        }
        let of = |mode| -> BTreeMap<BenchKernel, f64> {
            disk_tests(self.ctx.profile)
                .iter()
                .filter_map(|t| Some((t.id, self.mbs(mode, t.id)?)))
                .collect()
        };
        disk_points(
            &of(BenchMode::Read),
            &of(BenchMode::Write),
            self.ctx.baseline,
        )
    }

    fn save(&mut self) -> Vec<BenchAction> {
        let keys: Vec<_> = self.best.keys().copied().collect();
        for k in keys {
            self.check_other_io(k);
        }
        let b = self.ctx.baseline;
        let file = ScoreFile {
            format: FORMAT,
            id: self.ctx.id.clone(),
            at: self.ctx.at.clone(),
            category: "disk".into(),
            score_version: b.version.clone(),
            provisional: b.provisional,
            isa: None,
            shader_digest: None,
            scores: Scores {
                single: None,
                multi: None,
                read_mbs: self.mbs(BenchMode::Read, BenchKernel::Seq1mQ8t1),
                write_mbs: self.mbs(BenchMode::Write, BenchKernel::Seq1mQ8t1),
                points: self.points(),
                ..Scores::default()
            },
            kernels: disk_tests(self.ctx.profile)
                .iter()
                .map(|t| {
                    let rate = |mode| self.best.get(&(mode, t.id)).map(|b| b.rate.clone());
                    KernelRate {
                        id: t.id,
                        unit: "MB/s".into(),
                        single: None,
                        multi: None,
                        value: None,
                        spread: None,
                        read: rate(BenchMode::Read),
                        write: rate(BenchMode::Write),
                    }
                })
                .collect(),
            device: Device {
                device_id: Some(self.ctx.device_id.clone()),
                ..self.ctx.device.clone()
            },
            flags: self.life.flag_names(&FLAG_ORDER),
            valid: !INVALID.iter().any(|f| self.life.has_flag(f)),
            scaling: None,
            samples: self.life.samples.clone(),
            app_version: self.ctx.app_version.clone(),
            load_version: self.life.load_version.clone(),
            disk_profile: Some(self.ctx.profile),
        };
        self.life.end_saved(file)
    }

    /// The current step is the fill while `steps[step].kernel` is `DiskFill`: the UI then
    /// shows `performance.score.disk.fill`.
    pub fn status(&self) -> BenchStatus {
        BenchStatus {
            category: "disk".into(),
            device_id: Some(self.ctx.device_id.clone()),
            state: self.life.state,
            step: self.step,
            steps: self.steps.clone(),
            segments: self.segments.clone(),
            live_points: None,
            single: None,
            multi: None,
            compute: None,
            graphics: None,
            read_mbs: self.mbs(BenchMode::Read, BenchKernel::Seq1mQ8t1),
            write_mbs: self.mbs(BenchMode::Write, BenchKernel::Seq1mQ8t1),
            points: self.points(),
            live_read: self.live_read,
            live_write: self.live_write,
            flags: self.life.flag_names(&FLAG_ORDER),
            score_id: self.life.score_id.clone(),
            error: self.life.error.clone(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::scores::{disk_baseline, disk_bench_plan, B1_TESTS};
    use oma_ipc::load::{
        ComputeError, DiskPhaseStats, DiskProgress, ErrorKind, FinishReason, Finished, IoStats,
        Isa, KernelId, Notice, PhaseDone, Progress,
    };

    const DEVICE: &str = "disk-0";

    fn clock(t: u64) -> Clock {
        Clock {
            mono_ms: t,
            wall_ms: 1_000_000_000_000 + t as i64,
            asleep_ms: 0,
        }
    }

    fn steps_of(profile: DiskProfile) -> Vec<BenchStep> {
        disk_bench_plan("C:\\Temp\\oma".into(), 500 << 30, false, profile, 1).1
    }

    fn ctx_of(profile: DiskProfile) -> DiskBenchContext {
        DiskBenchContext {
            id: "0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d".into(),
            at: "2026-10-08T10:00:00Z".into(),
            device: Device {
                model: "Samsung 990".into(),
                kind: Some("nvme".into()),
                ..Device::default()
            },
            device_id: DEVICE.into(),
            profile,
            compressible: false,
            virtual_disk: false,
            removable: false,
            threshold_c: 80.0,
            on_battery: Some(false),
            baseline: disk_baseline(),
            app_version: "0.6.0".into(),
        }
    }

    fn ctl_with(ctx: DiskBenchContext) -> DiskBenchController {
        DiskBenchController::new(steps_of(ctx.profile), ctx, clock(0))
    }

    fn ctl() -> DiskBenchController {
        ctl_with(ctx_of(DiskProfile::B1))
    }

    fn stats(mbs: f64) -> IoStats {
        IoStats {
            bytes: (mbs * 1e6) as u64,
            ios: 1000,
            elapsed_us: 1_000_000,
            mean_lat_us: 100.0 + mbs,
            p99_lat_us: 300.0,
        }
    }

    /// A `PhaseDone` of step `i` measuring `mbs` in the step's direction.
    fn done_at(c: &DiskBenchController, i: usize, mbs: f64) -> LoadMessage {
        let write = c.steps[i].mode == BenchMode::Write;
        let (r, w) = if write {
            (stats(0.0), stats(mbs))
        } else {
            (stats(mbs), stats(0.0))
        };
        LoadMessage::PhaseDone(PhaseDone {
            phase: i as u32,
            checks: 0,
            errors: 0,
            duration_ms: 5000,
            skipped: None,
            work_ms: None,
            workers: vec![],
            rates: vec![],
            disk: Some(DiskPhaseStats { read: r, write: w }),
        })
    }

    fn progress(phase: u32, read_bps: f64, write_bps: f64) -> LoadMessage {
        LoadMessage::Progress(Progress {
            phase,
            phase_elapsed_ms: 1000,
            elapsed_ms: 0,
            checks: 0,
            errors: 0,
            current_core: None,
            cores: vec![],
            memory_bytes: 0,
            rate: None,
            load_percent: None,
            disk: Some(DiskProgress {
                read_bps,
                write_bps,
                read_bytes: 0,
                written_bytes: 0,
                iops: 0.0,
            }),
        })
    }

    fn finished(reason: FinishReason) -> LoadMessage {
        LoadMessage::Finished(Finished {
            reason,
            checks: 0,
            errors: 0,
        })
    }

    fn sample(temp: Option<f64>, io: Option<f64>) -> SensorSample {
        SensorSample {
            temp_c: temp,
            io_bps: io,
            ..SensorSample::default()
        }
    }

    /// The reference rate of step `i` (1000 for the tests B2 has and the scale lacks).
    fn reference(c: &DiskBenchController, i: usize) -> f64 {
        let s = c.steps[i];
        let b = disk_baseline();
        let t = if s.mode == BenchMode::Read {
            &b.read
        } else {
            &b.write
        };
        t.get(&s.kernel).copied().unwrap_or(1000.0)
    }

    /// Runs steps `range`: warm-up at half the reference, measures at 0.9, 1.0, 0.95.
    fn run(c: &mut DiskBenchController, range: std::ops::Range<usize>) {
        for i in range {
            let f = match c.steps[i].rep {
                0 => 0.5,
                1 => 0.9,
                2 => 1.0,
                _ => 0.95,
            };
            let mbs = reference(c, i) * f;
            c.on_load(&progress(i as u32, 0.0, 0.0), clock(1000 + i as u64));
            c.on_load(&done_at(c, i, mbs), clock(1000 + i as u64));
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

    fn near(a: Option<f64>, b: f64) {
        let a = a.expect("a value");
        assert!((a - b).abs() < 1e-6 * b.max(1.0), "{a} vs {b}");
    }

    #[test]
    fn full_b1_run_saves_reads_writes_and_points() {
        let mut c = ctl();
        run(&mut c, 0..33);
        let st = c.status();
        near(st.read_mbs, 7000.0);
        near(st.write_mbs, 6000.0);
        assert_eq!(st.points, Some(1000));
        let a = c.on_load(&finished(FinishReason::Completed), clock(60_000));
        let f = saved(&a);
        assert_eq!((f.format, f.category.as_str()), (FORMAT, "disk"));
        assert_eq!(f.score_version, "disk-1");
        assert_eq!(
            (f.isa, f.scaling, f.disk_profile),
            (None, None, Some(DiskProfile::B1))
        );
        near(f.scores.read_mbs, 7000.0);
        near(f.scores.write_mbs, 6000.0);
        assert_eq!(f.scores.points, Some(1000));
        assert_eq!(f.kernels.len(), 4);
        for (k, t) in f.kernels.iter().zip(&B1_TESTS) {
            assert_eq!((k.id, k.unit.as_str()), (t.id, "MB/s"));
            assert!(k.read.is_some() && k.write.is_some());
        }
        let r = f.kernels[0].read.as_ref().unwrap();
        assert!((r.iops - 1000.0).abs() < 1e-9 && r.p99_lat_us == 300.0);
        assert_eq!(f.device.device_id.as_deref(), Some(DEVICE));
        assert_eq!(f.device.kind.as_deref(), Some("nvme"));
        assert!(f.valid && f.flags.is_empty());
        assert_eq!(c.status().state, BenchState::Done);
        assert_eq!(c.status().score_id.as_deref(), Some(f.id.as_str()));
    }

    #[test]
    fn best_of_three_is_kept() {
        let mut c = ctl();
        run(&mut c, 0..5); // SEQ1M Q8T1 read: 0.5 warm-up, 0.9, 1.0, 0.95
        near(c.status().read_mbs, 7000.0);
        let a = c.on_load(&finished(FinishReason::Completed), clock(9_000));
        let f = saved(&a);
        near(f.kernels[0].read.as_ref().map(|r| r.mbs), 7000.0);
        assert!(f.kernels[0].write.is_none() && f.scores.points.is_none());
    }

    #[test]
    fn warmup_is_not_a_result() {
        let mut c = ctl();
        run(&mut c, 0..2); // fill + the read warm-up
        let st = c.status();
        assert_eq!(st.read_mbs, None);
        assert_eq!(st.segments[1], SegmentState::Done);
        assert_eq!(st.segments[0], SegmentState::Done);
    }

    #[test]
    fn live_needles_follow_their_direction() {
        let mut c = ctl();
        c.on_load(&progress(0, 0.0, 900e6), clock(10)); // the fill moves no needle
        assert_eq!((c.status().live_read, c.status().live_write), (None, None));
        c.on_load(&progress(2, 3000e6, 77e6), clock(20));
        let st = c.status();
        near(st.live_read, 3000.0);
        assert_eq!(st.live_write, None);
        c.on_load(&progress(18, 5e6, 2500e6), clock(30));
        let st = c.status();
        near(st.live_read, 3000.0);
        near(st.live_write, 2500.0);
        assert_eq!(st.step, Some(18));
    }

    #[test]
    fn needle_holds_the_result_at_the_end_of_a_test() {
        let mut c = ctl();
        run(&mut c, 0..5);
        near(c.status().live_read, 7000.0);
    }

    #[test]
    fn io_error_saves_an_invalid_score() {
        let mut c = ctl();
        run(&mut c, 0..5);
        let a = c.on_load(
            &LoadMessage::Error(ComputeError {
                phase: 5,
                kernel: KernelId::DiskBench,
                isa: Isa::Sse2,
                kind: ErrorKind::IoError,
                logical: None,
                core: None,
                iteration: 1,
                expected: 0,
                actual: 23,
                seed: 1,
                load_percent: None,
                transient: None,
            }),
            clock(9_000),
        );
        assert_eq!(a[0], BenchAction::SendStop);
        let f = saved(&a);
        assert!(!f.valid);
        assert_eq!(f.flags, ["io_error"]);
        near(f.scores.read_mbs, 7000.0);
        assert_eq!(f.scores.points, None);
    }

    #[test]
    fn exit_five_saves_an_invalid_score() {
        let mut c = ctl();
        run(&mut c, 0..5);
        let a = c.on_exit(Some(5), clock(9_000));
        let f = saved(&a);
        assert!(!f.valid && f.flags == ["io_error"]);
        assert_eq!(c.status().state, BenchState::Done);
    }

    #[test]
    fn disk_full_saves_an_invalid_score() {
        let mut c = ctl();
        run(&mut c, 0..5);
        let a = c.on_load(
            &LoadMessage::Notice(Notice {
                phase: 5,
                code: "disk_full".into(),
                value: None,
            }),
            clock(9_000),
        );
        assert_eq!(a[0], BenchAction::SendStop);
        let f = saved(&a);
        assert!(!f.valid && f.flags == ["disk_full"]);
    }

    /// Runs the five steps of SEQ1M Q8T1 read at `mbs`, with `io` B/s sampled in each.
    fn measured_with_io(c: &mut DiskBenchController, mbs: f64, io: f64) {
        for i in 1..5 {
            c.on_load(&progress(i as u32, 0.0, 0.0), clock(1000 + i as u64));
            c.on_sample(&sample(None, Some(io)), clock(1100 + i as u64));
            c.on_load(&done_at(c, i, mbs), clock(1200 + i as u64));
        }
    }

    #[test]
    fn other_io_above_five_percent_flags() {
        let mut c = ctl();
        measured_with_io(&mut c, 1000.0, 1100e6);
        assert_eq!(c.status().flags, ["other_io"]);
        // Exactly 5% does not.
        let mut c = ctl();
        measured_with_io(&mut c, 1000.0, 1050e6);
        assert!(c.status().flags.is_empty());
    }

    #[test]
    fn other_io_below_one_mb_s_does_not() {
        let mut c = ctl();
        measured_with_io(&mut c, 0.5, 0.9e6); // 80% above, but only 0.4 MB/s
        assert!(c.status().flags.is_empty());
    }

    #[test]
    fn hot_sample_flags_thermal() {
        let mut c = ctl();
        c.on_sample(&sample(Some(79.9), None), clock(100));
        assert!(c.status().flags.is_empty());
        c.on_sample(&sample(Some(80.0), None), clock(200));
        assert_eq!(c.status().flags, ["thermal"]);
    }

    #[test]
    fn compressible_virtual_and_removable_flag_from_the_context() {
        let mut x = ctx_of(DiskProfile::B1);
        x.compressible = true;
        x.virtual_disk = true;
        x.removable = true;
        x.on_battery = Some(true);
        let c = ctl_with(x);
        assert_eq!(
            c.status().flags,
            ["battery", "compressible", "virtual_disk", "removable"]
        );
    }

    #[test]
    fn b2_run_has_no_points() {
        let mut c = ctl_with(ctx_of(DiskProfile::B2));
        run(&mut c, 0..33);
        assert_eq!(c.status().points, None);
        let a = c.on_load(&finished(FinishReason::Completed), clock(60_000));
        let f = saved(&a);
        assert_eq!(f.disk_profile, Some(DiskProfile::B2));
        assert_eq!(f.scores.points, None);
        assert!(f.valid);
        assert_eq!(f.kernels.len(), 4);
        assert_eq!(f.kernels[1].id, crate::scores::BenchKernel::Seq128kQ32t1);
        assert!(f.kernels[1].read.is_some());
    }

    #[test]
    fn user_stop_saves_nothing() {
        let mut c = ctl();
        run(&mut c, 0..5);
        assert_eq!(c.on_user_stop(clock(9_000)), vec![BenchAction::SendStop]);
        let a = c.on_load(&finished(FinishReason::Stopped), clock(9_100));
        assert_eq!(a, vec![BenchAction::Finished(BenchEnd::Stopped)]);
        assert_eq!(c.status().state, BenchState::Stopped);
    }

    #[test]
    fn early_exit_fails_without_saving() {
        let mut c = ctl();
        run(&mut c, 0..3);
        let a = c.on_exit(Some(1), clock(9_000));
        assert_eq!(
            a,
            vec![BenchAction::Finished(BenchEnd::Failed("exited".into()))]
        );
        assert_eq!(c.status().state, BenchState::Failed);
    }

    #[test]
    fn status_carries_category_disk_and_device() {
        let st = ctl().status();
        assert_eq!(st.category, "disk");
        assert_eq!(st.device_id.as_deref(), Some(DEVICE));
        assert_eq!(st.steps.len(), 33);
        assert_eq!(
            (st.single, st.multi, st.compute, st.graphics),
            (None, None, None, None)
        );
    }

    #[test]
    fn silent_helper_is_killed_as_hung() {
        let mut c = ctl();
        run(&mut c, 0..1);
        let a = c.on_clock(clock(1000 + SILENT_PIPE_MS + 1));
        assert_eq!(a[0], BenchAction::Kill);
    }
}
