//! Benchmark session: an accumulator over the displayed frames of one
//! capture and the summary computed from it. Memory is one `f32` per
//! displayed frame plus a handful of counters, capped at
//! [`MAX_SESSION_FRAMES`].

use serde::{Deserialize, Serialize};

use crate::frames::generation::generated_label;
use crate::frames::metrics::{lows, LowDefinition, StutterCounter};
use crate::frames::{fg_multiplier, FrameKind, FrameSample, Rendered, RenderedSource};

/// Displayed frames kept per session: one hour at 1000 FPS.
pub const MAX_SESSION_FRAMES: usize = 3_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryLows {
    pub one_percent: f64,
    pub point_one_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub duration_s: f64,
    pub frames_total: u64,
    pub frames_displayed: u64,
    pub frames_generated: u64,
    pub fps_displayed: f64,
    pub fps_rendered: Option<f64>,
    /// `XeSS-FG`, `AFMF`, `FG` or `Reflex`.
    pub rendered_source: Option<String>,
    pub lows_integral: SummaryLows,
    pub lows_percentile: SummaryLows,
    pub frametime_min_ms: f64,
    pub frametime_max_ms: f64,
    pub stutter_count: u32,
    pub stutter_percent: f64,
    pub fg_multiplier: Option<f64>,
    pub latency_pc_ms: Option<f64>,
    pub latency_display_ms: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct SessionAccumulator {
    cap: usize,
    /// Frametimes of the displayed frames that carry the value.
    frametimes: Vec<f32>,
    sum_ms: f64,
    min_ms: f64,
    max_ms: f64,
    total: u64,
    /// Generated frames: XeSS-FG, AFMF, other.
    generated: [u64; 3],
    app_displayed: u64,
    first_t: f64,
    last_t: f64,
    first_pcl: Option<(f64, u64)>,
    last_pcl: Option<(f64, u64)>,
    pc_latency: (f64, u64),
    display_latency: (f64, u64),
    stutter: StutterCounter,
}

impl Default for SessionAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionAccumulator {
    pub fn new() -> Self {
        Self::with_cap(MAX_SESSION_FRAMES)
    }

    /// A session with a smaller frame cap; intended for tests and tools.
    pub fn with_cap(cap: usize) -> Self {
        Self {
            cap,
            frametimes: Vec::new(),
            sum_ms: 0.0,
            min_ms: f64::INFINITY,
            max_ms: 0.0,
            total: 0,
            generated: [0; 3],
            app_displayed: 0,
            first_t: 0.0,
            last_t: 0.0,
            first_pcl: None,
            last_pcl: None,
            pc_latency: (0.0, 0),
            display_latency: (0.0, 0),
            stutter: StutterCounter::new(),
        }
    }

    /// Adds a frame, in `t_s` order. False, with the frame left out, once the
    /// displayed frames have reached the cap.
    pub fn push(&mut self, f: &FrameSample) -> bool {
        if self.frametimes.len() >= self.cap {
            return false;
        }
        if self.total == 0 {
            self.first_t = f.t_s;
        }
        self.last_t = f.t_s;
        self.total += 1;
        match f.kind {
            FrameKind::GeneratedIntelXefg => self.generated[0] += 1,
            FrameKind::GeneratedAmdAfmf => self.generated[1] += 1,
            FrameKind::GeneratedOther => self.generated[2] += 1,
            _ => {}
        }
        if let Some(id) = f.pcl_frame_id {
            self.first_pcl.get_or_insert((f.t_s, id));
            self.last_pcl = Some((f.t_s, id));
        }
        if let (true, Some(ms)) = (f.displayed, f.ms_between_display_change) {
            self.frametimes.push(ms as f32);
            self.sum_ms += ms;
            self.min_ms = self.min_ms.min(ms);
            self.max_ms = self.max_ms.max(ms);
            self.app_displayed += u64::from(f.kind == FrameKind::App);
            self.stutter.push(f.t_s, ms);
            for (slot, v) in [
                (&mut self.pc_latency, f.ms_pc_latency),
                (&mut self.display_latency, f.ms_until_displayed),
            ] {
                if let Some(v) = v {
                    slot.0 += v;
                    slot.1 += 1;
                }
            }
        }
        true
    }

    pub fn frames(&self) -> u64 {
        self.total
    }

    /// First to last `t_s` seen.
    pub fn duration_s(&self) -> f64 {
        self.last_t - self.first_t
    }

    /// `None` without displayed frames.
    pub fn summary(&self) -> Option<SessionSummary> {
        let n = self.frametimes.len();
        if n == 0 || self.sum_ms <= 0.0 {
            return None;
        }
        let fps_displayed = 1000.0 * n as f64 / self.sum_ms;
        let fts: Vec<f64> = self.frametimes.iter().map(|&v| f64::from(v)).collect();
        // `lows` needs two values; a one-frame session reports its own rate.
        let low = |def| {
            lows(&fts, def).map_or(
                SummaryLows {
                    one_percent: fps_displayed,
                    point_one_percent: fps_displayed,
                },
                |l| SummaryLows {
                    one_percent: l.one_percent,
                    point_one_percent: l.point_one_percent,
                },
            )
        };
        let generated: u64 = self.generated.iter().sum();
        // DD8: the driver's frame type, then the PCL frame ids, else nothing.
        let (rendered, source) = if generated > 0 {
            let app = 1000.0 * self.app_displayed as f64 / self.sum_ms;
            let label = generated_label(
                self.generated[0] as usize,
                self.generated[1] as usize,
                self.generated[2] as usize,
            );
            (Some(app), Some(label))
        } else {
            match (self.first_pcl, self.last_pcl) {
                (Some((t0, id0)), Some((t1, id1))) if t1 > t0 && id1 > id0 => {
                    (Some((id1 - id0) as f64 / (t1 - t0)), Some("Reflex"))
                }
                _ => (None, None),
            }
        };
        let mean = |(sum, n): (f64, u64)| (n > 0).then(|| sum / n as f64);
        let stutter = self.stutter.result();
        Some(SessionSummary {
            duration_s: self.duration_s(),
            frames_total: self.total,
            frames_displayed: n as u64,
            frames_generated: generated,
            fps_displayed,
            fps_rendered: rendered,
            rendered_source: source.map(str::to_owned),
            lows_integral: low(LowDefinition::Integral),
            lows_percentile: low(LowDefinition::Percentile),
            frametime_min_ms: self.min_ms,
            frametime_max_ms: self.max_ms,
            stutter_count: stutter.count,
            stutter_percent: stutter.time_percent,
            fg_multiplier: rendered.and_then(|fps| {
                fg_multiplier(
                    Some(fps_displayed),
                    &Rendered::Fps {
                        fps,
                        source: RenderedSource::FrameType,
                    },
                )
            }),
            latency_pc_ms: mean(self.pc_latency),
            latency_display_ms: mean(self.display_latency),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(t_s: f64, displayed: bool) -> FrameSample {
        FrameSample {
            t_s,
            swapchain: 1,
            kind: FrameKind::App,
            displayed,
            ms_between_presents: 10.0,
            ms_between_display_change: displayed.then_some(10.0),
            ms_until_displayed: None,
            ms_app_frametime: None,
            ms_pc_latency: None,
            ms_gpu_busy: None,
            pcl_frame_id: None,
        }
    }

    #[test]
    fn summary_none_without_displayed_frames() {
        let mut s = SessionAccumulator::new();
        assert!(s.summary().is_none());
        assert!(s.push(&frame(0.0, false)));
        assert_eq!(s.frames(), 1);
        assert!(s.summary().is_none());
    }

    #[test]
    fn push_refuses_beyond_the_cap() {
        let mut s = SessionAccumulator::with_cap(3);
        for i in 0..3 {
            assert!(s.push(&frame(f64::from(i) * 0.01, true)));
        }
        assert!(!s.push(&frame(0.03, true)));
        assert_eq!(s.frames(), 3);
        assert_eq!(s.summary().unwrap().frames_displayed, 3);
    }

    #[test]
    fn summary_serializes_every_key() {
        let mut s = SessionAccumulator::new();
        for i in 0..20 {
            s.push(&frame(f64::from(i) * 0.01, true));
        }
        let v = serde_json::to_value(s.summary().unwrap()).unwrap();
        let obj = v.as_object().unwrap();
        for key in [
            "durationS",
            "framesTotal",
            "framesDisplayed",
            "framesGenerated",
            "fpsDisplayed",
            "fpsRendered",
            "renderedSource",
            "lowsIntegral",
            "lowsPercentile",
            "frametimeMinMs",
            "frametimeMaxMs",
            "stutterCount",
            "stutterPercent",
            "fgMultiplier",
            "latencyPcMs",
            "latencyDisplayMs",
        ] {
            assert!(obj.contains_key(key), "{key}");
        }
        assert_eq!(obj.len(), 16);
        assert!(obj["fpsRendered"].is_null());
        assert!(obj["lowsIntegral"]["onePercent"].is_number());
        assert!(obj["lowsIntegral"]["pointOnePercent"].is_number());
        let back: SessionSummary = serde_json::from_value(v).unwrap();
        assert_eq!(back, s.summary().unwrap());
    }
}
