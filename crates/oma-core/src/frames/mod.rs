//! Frame model for the frame engine: one sample per presented frame and a
//! rolling window ordered by presentation time. No Windows code here; the
//! providers that produce the samples live in `oma-win` and the service.

pub mod generation;
pub mod metrics;
pub mod swapchain;
pub mod synthetic;

pub use generation::{
    fg_multiplier, fg_suspected, rendered_fps, source_label, Rendered, RenderedSource,
};
pub use swapchain::pick_swapchain;
pub use synthetic::{synthetic, SyntheticProfile};

use std::collections::VecDeque;

/// Window used for the FPS and latency readouts.
pub const FPS_WINDOW_S: f64 = 1.0;
/// Window used for the 1% / 0.1% lows and the stutter count.
pub const LOWS_WINDOW_S: f64 = 10.0;

/// Where a frame comes from: the application or a frame generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    App,
    GeneratedIntelXefg,
    GeneratedAmdAfmf,
    GeneratedOther,
    Unknown,
}

impl FrameKind {
    /// Parses the protocol string; every unrecognised value is `Unknown`.
    pub fn from_wire(s: &str) -> FrameKind {
        match s {
            "app" => FrameKind::App,
            "generated_intel_xefg" => FrameKind::GeneratedIntelXefg,
            "generated_amd_afmf" => FrameKind::GeneratedAmdAfmf,
            "generated_other" => FrameKind::GeneratedOther,
            _ => FrameKind::Unknown,
        }
    }

    pub fn is_generated(self) -> bool {
        matches!(
            self,
            FrameKind::GeneratedIntelXefg | FrameKind::GeneratedAmdAfmf | FrameKind::GeneratedOther
        )
    }
}

/// One presented frame. `t_s` is the start of the present in seconds
/// (QPC divided by the QPC frequency).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSample {
    pub t_s: f64,
    pub swapchain: u64,
    pub kind: FrameKind,
    pub displayed: bool,
    pub ms_between_presents: f64,
    pub ms_between_display_change: Option<f64>,
    pub ms_until_displayed: Option<f64>,
    pub ms_app_frametime: Option<f64>,
    pub ms_pc_latency: Option<f64>,
    pub ms_gpu_busy: Option<f64>,
    pub pcl_frame_id: Option<u64>,
}

/// Rolling window of frames, kept ordered by `t_s` (frame time, not arrival
/// time) and trimmed to `max_age_s` behind the newest frame.
#[derive(Debug, Clone)]
pub struct FrameWindow {
    max_age_s: f64,
    frames: VecDeque<FrameSample>,
}

impl FrameWindow {
    pub fn new(max_age_s: f64) -> Self {
        Self {
            max_age_s,
            frames: VecDeque::new(),
        }
    }

    /// Inserts the frame at its time position and drops frames older than
    /// `max_age_s` relative to the newest one.
    pub fn push(&mut self, f: FrameSample) {
        let at = self.frames.partition_point(|x| x.t_s <= f.t_s);
        self.frames.insert(at, f);
        let newest = self.frames.back().map_or(f.t_s, |x| x.t_s);
        let cutoff = newest - self.max_age_s;
        while self.frames.front().is_some_and(|x| x.t_s < cutoff) {
            self.frames.pop_front();
        }
    }

    /// Frames with `t_s >= newest - seconds`, oldest first.
    pub fn last(&self, seconds: f64) -> Vec<FrameSample> {
        let Some(newest) = self.frames.back().map(|x| x.t_s) else {
            return Vec::new();
        };
        let cutoff = newest - seconds;
        let start = self.frames.partition_point(|x| x.t_s < cutoff);
        self.frames.iter().skip(start).copied().collect()
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(t_s: f64) -> FrameSample {
        FrameSample {
            t_s,
            swapchain: 1,
            kind: FrameKind::App,
            displayed: true,
            ms_between_presents: 10.0,
            ms_between_display_change: Some(10.0),
            ms_until_displayed: None,
            ms_app_frametime: None,
            ms_pc_latency: None,
            ms_gpu_busy: None,
            pcl_frame_id: None,
        }
    }

    #[test]
    fn from_wire_maps_known_values_and_defaults_to_unknown() {
        assert_eq!(FrameKind::from_wire("app"), FrameKind::App);
        assert_eq!(
            FrameKind::from_wire("generated_intel_xefg"),
            FrameKind::GeneratedIntelXefg
        );
        assert_eq!(
            FrameKind::from_wire("generated_amd_afmf"),
            FrameKind::GeneratedAmdAfmf
        );
        assert_eq!(
            FrameKind::from_wire("generated_other"),
            FrameKind::GeneratedOther
        );
        assert_eq!(FrameKind::from_wire("unknown"), FrameKind::Unknown);
        assert_eq!(FrameKind::from_wire("???"), FrameKind::Unknown);
        assert!(FrameKind::GeneratedOther.is_generated());
        assert!(!FrameKind::App.is_generated());
        assert!(!FrameKind::Unknown.is_generated());
    }

    #[test]
    fn window_drops_frames_older_than_max_age() {
        let mut w = FrameWindow::new(2.0);
        for i in 0..=50 {
            w.push(sample(f64::from(i) * 0.1));
        }
        let all = w.last(100.0);
        assert!((all[0].t_s - 3.0).abs() < 1e-9);
        assert_eq!(all.len(), 21);
    }

    #[test]
    fn window_last_returns_the_trailing_seconds() {
        let mut w = FrameWindow::new(10.0);
        for i in 0..=30 {
            w.push(sample(f64::from(i) * 0.1));
        }
        let got = w.last(1.0);
        assert_eq!(got.len(), 11);
        assert!((got[0].t_s - 2.0).abs() < 1e-9);
        assert!(FrameWindow::new(1.0).last(1.0).is_empty());
    }

    #[test]
    fn window_uses_frame_time_not_arrival() {
        let mut w = FrameWindow::new(10.0);
        w.push(sample(1.0));
        w.push(sample(2.0));
        w.push(sample(1.5));
        let got = w.last(10.0);
        let ts: Vec<f64> = got.iter().map(|f| f.t_s).collect();
        assert_eq!(ts, vec![1.0, 1.5, 2.0]);
        assert_eq!(w.last(0.6).len(), 2);
        w.clear();
        assert!(w.last(10.0).is_empty());
    }
}
