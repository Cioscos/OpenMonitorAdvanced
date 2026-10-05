//! Rendered (pre frame generation) FPS, the cascade of sources and the
//! frame generation heuristic.

use crate::frames::{FrameKind, FrameSample};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderedSource {
    FrameType,
    Reflex,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rendered {
    Fps { fps: f64, source: RenderedSource },
    FgSuspected,
    Unavailable,
}

/// Label for the origin of a rendered-FPS figure.
pub fn source_label(source: RenderedSource, frames: &[FrameSample]) -> &'static str {
    match source {
        RenderedSource::Reflex => "Reflex",
        RenderedSource::FrameType => {
            let count = |k: FrameKind| frames.iter().filter(|f| f.kind == k).count();
            generated_label(
                count(FrameKind::GeneratedIntelXefg),
                count(FrameKind::GeneratedAmdAfmf),
                count(FrameKind::GeneratedOther),
            )
        }
    }
}

/// Label of the frame generator that produced most of the generated frames.
pub(crate) fn generated_label(xefg: usize, afmf: usize, other: usize) -> &'static str {
    if xefg > afmf + other {
        "XeSS-FG"
    } else if afmf > xefg + other {
        "AFMF"
    } else {
        "FG"
    }
}

/// Rendered FPS cascade: the frame type when the capture tells generated
/// frames apart, then the PC-latency frame ids (Reflex), then the heuristic.
pub fn rendered_fps(frames: &[FrameSample]) -> Rendered {
    if frames.iter().any(|f| f.kind.is_generated()) {
        let (mut app, mut total_ms) = (0u32, 0.0);
        for f in frames.iter().filter(|f| f.displayed) {
            if let Some(ms) = f.ms_between_display_change {
                total_ms += ms;
                app += u32::from(f.kind == FrameKind::App);
            }
        }
        if total_ms > 0.0 {
            return Rendered::Fps {
                fps: 1000.0 * f64::from(app) / total_ms,
                source: RenderedSource::FrameType,
            };
        }
    }
    let mut with_id = frames
        .iter()
        .filter_map(|f| f.pcl_frame_id.map(|id| (f.t_s, id)));
    if let Some(first) = with_id.next() {
        if let Some(last) = with_id.next_back() {
            let dt = last.0 - first.0;
            if dt > 0.0 && last.1 > first.1 {
                return Rendered::Fps {
                    fps: (last.1 - first.1) as f64 / dt,
                    source: RenderedSource::Reflex,
                };
            }
        }
    }
    if fg_suspected(frames) {
        Rendered::FgSuspected
    } else {
        Rendered::Unavailable
    }
}

const WINDOW_S: f64 = 2.0;
const MIN_WINDOW_FRAMES: usize = 8;
const MIN_ALTERNATION: f64 = 0.9;
const MIN_RATIO: f64 = 1.8;

/// Frame generation without any frame label: presents alternate between a
/// short and a long interval. True when the median over 2 s windows (at
/// least 8 frames each) has an alternation of 0.9 or more and a long/short
/// ratio of 1.8 or more. `frames` must be ordered by `t_s`.
pub fn fg_suspected(frames: &[FrameSample]) -> bool {
    let Some(first) = frames.first() else {
        return false;
    };
    let mut alternations = Vec::new();
    let mut ratios = Vec::new();
    let mut start = 0;
    while start < frames.len() {
        let index = ((frames[start].t_s - first.t_s) / WINDOW_S).floor();
        let limit = first.t_s + (index + 1.0) * WINDOW_S;
        let len = frames[start..].partition_point(|f| f.t_s < limit);
        let end = start + len.max(1);
        let values: Vec<f64> = frames[start..end]
            .iter()
            .map(|f| f.ms_between_presents)
            .collect();
        if values.len() >= MIN_WINDOW_FRAMES {
            alternations.push(alternation(&values));
            ratios.push(split_ratio(&values));
        }
        start = end;
    }
    if alternations.is_empty() {
        return false;
    }
    median(&mut alternations) >= MIN_ALTERNATION && median(&mut ratios) >= MIN_RATIO
}

/// Share of consecutive pairs of differences with opposite signs.
fn alternation(values: &[f64]) -> f64 {
    let diffs: Vec<f64> = values.windows(2).map(|w| w[1] - w[0]).collect();
    let pairs = diffs.len().saturating_sub(1);
    if pairs == 0 {
        return 0.0;
    }
    let flips = diffs.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
    flips as f64 / pairs as f64
}

/// Median of the values above the median over the median of the rest.
fn split_ratio(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    let mid = median(&mut sorted);
    let mut upper: Vec<f64> = values.iter().copied().filter(|&v| v > mid).collect();
    let mut lower: Vec<f64> = values.iter().copied().filter(|&v| v <= mid).collect();
    if upper.is_empty() || lower.is_empty() {
        return 1.0;
    }
    let low = median(&mut lower);
    if low <= 0.0 {
        return 1.0;
    }
    median(&mut upper) / low
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Displayed over rendered FPS; needs a rendered figure from the cascade.
pub fn fg_multiplier(displayed: Option<f64>, rendered: &Rendered) -> Option<f64> {
    match (displayed, rendered) {
        (Some(d), Rendered::Fps { fps, .. }) if *fps > 0.0 => Some(d / fps),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::FrameKind;

    fn fr(t_s: f64, kind: FrameKind, presents_ms: f64, display_ms: f64) -> FrameSample {
        FrameSample {
            t_s,
            swapchain: 1,
            kind,
            displayed: true,
            ms_between_presents: presents_ms,
            ms_between_display_change: Some(display_ms),
            ms_until_displayed: None,
            ms_app_frametime: None,
            ms_pc_latency: None,
            ms_gpu_busy: None,
            pcl_frame_id: None,
        }
    }

    fn fps_of(r: Rendered) -> (f64, RenderedSource) {
        match r {
            Rendered::Fps { fps, source } => (fps, source),
            other => panic!("expected Fps, got {other:?}"),
        }
    }

    #[test]
    fn frame_type_cascade_counts_app_frames() {
        let frames: Vec<FrameSample> = (0..100)
            .map(|i| {
                let kind = if i % 2 == 0 {
                    FrameKind::App
                } else {
                    FrameKind::GeneratedIntelXefg
                };
                fr(f64::from(i) * 0.01, kind, 10.0, 10.0)
            })
            .collect();
        let (fps, source) = fps_of(rendered_fps(&frames));
        assert_eq!(source, RenderedSource::FrameType);
        // 100 displayed over 1 s, half of them app frames.
        assert!((fps - 50.0).abs() < 1e-9);
        assert_eq!(source_label(source, &frames), "XeSS-FG");
    }

    #[test]
    fn source_labels() {
        let mk = |k| vec![fr(0.0, k, 10.0, 10.0), fr(0.01, FrameKind::App, 10.0, 10.0)];
        assert_eq!(
            source_label(RenderedSource::FrameType, &mk(FrameKind::GeneratedAmdAfmf)),
            "AFMF"
        );
        assert_eq!(
            source_label(RenderedSource::FrameType, &mk(FrameKind::GeneratedOther)),
            "FG"
        );
        assert_eq!(source_label(RenderedSource::Reflex, &[]), "Reflex");
    }

    #[test]
    fn reflex_rate_uses_the_id_range_not_the_row_count() {
        let mut frames: Vec<FrameSample> = [0.0, 0.01, 0.03, 0.04]
            .iter()
            .map(|&t| fr(t, FrameKind::App, 10.0, 10.0))
            .collect();
        for (f, id) in frames.iter_mut().zip([100u64, 101, 103, 104]) {
            f.pcl_frame_id = Some(id);
        }
        let (fps, source) = fps_of(rendered_fps(&frames));
        assert_eq!(source, RenderedSource::Reflex);
        assert!((fps - 100.0).abs() < 1e-6);
        assert_eq!(source_label(source, &frames), "Reflex");
    }

    fn cycle(values: &[f64], seconds: f64) -> Vec<FrameSample> {
        let mut t = 0.0;
        let mut out = Vec::new();
        let mut i = 0;
        while t < seconds {
            let v = values[i % values.len()];
            out.push(fr(t, FrameKind::App, v, v));
            t += v / 1000.0;
            i += 1;
        }
        out
    }

    #[test]
    fn fg_suspected_fires_on_alternating_presents() {
        let frames = cycle(&[0.25, 13.0], 3.0);
        assert!(fg_suspected(&frames));
        assert_eq!(rendered_fps(&frames), Rendered::FgSuspected);
    }

    #[test]
    fn fg_suspected_stays_quiet_on_jitter() {
        let mut x: u64 = 0x1234_5678_9ABC_DEF1;
        let mut t = 0.0;
        let mut frames = Vec::new();
        while t < 3.0 {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            let u = (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
            let v = 9.0 + 2.0 * u;
            frames.push(fr(t, FrameKind::App, v, v));
            t += v / 1000.0;
        }
        assert!(!fg_suspected(&frames));
    }

    #[test]
    fn no_evidence_gives_unavailable() {
        assert_eq!(rendered_fps(&[]), Rendered::Unavailable);
        let steady = cycle(&[10.0], 3.0);
        assert_eq!(rendered_fps(&steady), Rendered::Unavailable);
        // Too few frames per window to judge.
        let few = cycle(&[0.25, 13.0, 0.25], 0.03);
        assert!(!fg_suspected(&few));
    }

    #[test]
    fn multiplier_needs_both_values() {
        let r = Rendered::Fps {
            fps: 60.0,
            source: RenderedSource::Reflex,
        };
        assert!((fg_multiplier(Some(120.0), &r).unwrap() - 2.0).abs() < 1e-9);
        assert_eq!(fg_multiplier(None, &r), None);
        assert_eq!(fg_multiplier(Some(120.0), &Rendered::FgSuspected), None);
        assert_eq!(fg_multiplier(Some(120.0), &Rendered::Unavailable), None);
    }
}
