//! The lifecycle shared by the CPU and the GPU benchmark controllers: state, clock, the
//! stop grace, the hang watch, flags, the 5 s samples, the helper's version and the end.

use super::bench::{BenchAction, BenchEnd, BenchState};
use super::file::{ScoreFile, ScoreSample};
use crate::load::run::{OVERRUN_MS, SILENT_PIPE_MS};
use crate::load::{Clock, SensorSample};

const SAMPLE_EVERY_MS: u64 = 5_000;
const STOP_GRACE_MS: u64 = 3_000;

pub(super) struct Lifecycle {
    pub(super) state: BenchState,
    start_mono: u64,
    mono: u64,
    stop_deadline: Option<u64>,
    /// The last helper message; a running benchmark silent for longer than
    /// `SILENT_PIPE_MS` is hung, like a stress test.
    last_msg: u64,
    /// The plan's length at its caps: past it plus `OVERRUN_MS`, hung.
    plan_ms: u64,
    flags: Vec<&'static str>,
    pub(super) samples: Vec<ScoreSample>,
    last_sample_t: Option<u64>,
    pub(super) load_version: Option<String>,
    pub(super) score_id: Option<String>,
    pub(super) error: Option<String>,
}

impl Lifecycle {
    pub(super) fn new(plan_ms: u64, now: Clock) -> Self {
        Self {
            state: BenchState::Starting,
            start_mono: now.mono_ms,
            mono: now.mono_ms,
            stop_deadline: None,
            last_msg: now.mono_ms,
            plan_ms,
            flags: vec![],
            samples: vec![],
            last_sample_t: None,
            load_version: None,
            score_id: None,
            error: None,
        }
    }

    pub(super) fn is_finished(&self) -> bool {
        matches!(
            self.state,
            BenchState::Done | BenchState::Stopped | BenchState::Failed
        )
    }

    pub(super) fn flag(&mut self, f: &'static str) {
        if !self.flags.contains(&f) {
            self.flags.push(f);
        }
    }

    pub(super) fn has_flag(&self, f: &str) -> bool {
        self.flags.contains(&f)
    }

    /// The flags present, in `order`.
    pub(super) fn flag_names(&self, order: &[&str]) -> Vec<String> {
        order
            .iter()
            .filter(|f| self.flags.contains(f))
            .map(|f| f.to_string())
            .collect()
    }

    fn tick(&mut self, now: Clock) {
        self.mono = now.mono_ms.max(self.mono);
    }

    /// A helper message arrived: false once finished (ignore it).
    pub(super) fn on_message(&mut self, now: Clock) -> bool {
        if self.is_finished() {
            return false;
        }
        self.tick(now);
        self.last_msg = self.mono;
        if self.state == BenchState::Starting {
            self.state = BenchState::Running;
        }
        true
    }

    /// Keeps one sample every 5 s.
    pub(super) fn record(&mut self, sample: &SensorSample, now: Clock) {
        self.tick(now);
        let t = self.mono - self.start_mono;
        if self.last_sample_t.is_none_or(|p| t - p >= SAMPLE_EVERY_MS) {
            self.last_sample_t = Some(t);
            self.samples.push(ScoreSample {
                t_ms: t,
                temp_c: sample.temp_c,
                power_w: sample.power_w,
                clock_mhz: sample.clock_mhz,
            });
        }
    }

    pub(super) fn on_user_stop(&mut self, now: Clock) -> Vec<BenchAction> {
        if !matches!(self.state, BenchState::Starting | BenchState::Running) {
            return vec![];
        }
        self.tick(now);
        self.state = BenchState::Stopping;
        self.stop_deadline = Some(self.mono + STOP_GRACE_MS);
        vec![BenchAction::SendStop]
    }

    /// Kills a process that ignores our stop request, or that is hung: silent
    /// for more than `SILENT_PIPE_MS`, or running past its plan by `OVERRUN_MS`.
    pub(super) fn on_clock(&mut self, now: Clock) -> Vec<BenchAction> {
        if self.state == BenchState::Running {
            self.tick(now);
            let silent = self.mono - self.last_msg > SILENT_PIPE_MS;
            let overrun = self.mono - self.start_mono > self.plan_ms + OVERRUN_MS;
            if silent || overrun {
                let mut out = vec![BenchAction::Kill];
                out.extend(self.end_failed("hung"));
                return out;
            }
        }
        if self.state != BenchState::Stopping {
            return vec![];
        }
        self.tick(now);
        match self.stop_deadline {
            Some(d) if self.mono >= d => {
                let mut out = vec![BenchAction::Kill];
                out.extend(self.end_stopped());
                out
            }
            _ => vec![],
        }
    }

    pub(super) fn on_exit(&mut self, now: Clock) -> Vec<BenchAction> {
        if self.is_finished() {
            return vec![];
        }
        self.tick(now);
        if self.state == BenchState::Stopping {
            self.end_stopped()
        } else {
            self.end_failed("exited")
        }
    }

    pub(super) fn end_stopped(&mut self) -> Vec<BenchAction> {
        self.state = BenchState::Stopped;
        vec![BenchAction::Finished(BenchEnd::Stopped)]
    }

    pub(super) fn end_failed(&mut self, why: &str) -> Vec<BenchAction> {
        self.state = BenchState::Failed;
        self.error = Some(why.into());
        vec![BenchAction::Finished(BenchEnd::Failed(why.into()))]
    }

    /// Ends the run with `file` saved.
    pub(super) fn end_saved(&mut self, file: ScoreFile) -> Vec<BenchAction> {
        self.state = BenchState::Done;
        self.score_id = Some(file.id.clone());
        let id = file.id.clone();
        vec![
            BenchAction::Save(Box::new(file)),
            BenchAction::Finished(BenchEnd::Saved(id)),
        ]
    }
}
