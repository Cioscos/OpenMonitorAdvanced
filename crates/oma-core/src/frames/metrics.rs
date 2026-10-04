//! Frame metrics: FPS, lows, stutter, latency means and bottleneck.

use crate::frames::{FrameKind, FrameSample};

/// Displayed frames per second: `1000 * N / sum(ms_between_display_change)`
/// over the displayed frames that carry the value.
pub fn displayed_fps(frames: &[FrameSample]) -> Option<f64> {
    let (n, sum) = frames
        .iter()
        .filter(|f| f.displayed)
        .filter_map(|f| f.ms_between_display_change)
        .fold((0u32, 0.0), |(n, s), ms| (n + 1, s + ms));
    rate(n, sum)
}

/// Presented frames per second: `1000 * N / sum(ms_between_presents)`.
pub fn presented_fps(frames: &[FrameSample]) -> Option<f64> {
    let sum: f64 = frames.iter().map(|f| f.ms_between_presents).sum();
    rate(u32::try_from(frames.len()).unwrap_or(u32::MAX), sum)
}

fn rate(n: u32, sum_ms: f64) -> Option<f64> {
    (n > 0 && sum_ms > 0.0).then(|| 1000.0 * f64::from(n) / sum_ms)
}

/// How the 1% and 0.1% lows are computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LowDefinition {
    /// Longest frames summed until they reach `p * total time`.
    Integral,
    /// Nearest-rank percentile of the frametimes.
    Percentile,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lows {
    pub one_percent: f64,
    pub point_one_percent: f64,
}

/// 1% and 0.1% lows in FPS from frametimes in milliseconds; `None` with
/// fewer than two values.
pub fn lows(frametimes_ms: &[f64], def: LowDefinition) -> Option<Lows> {
    if frametimes_ms.len() < 2 {
        return None;
    }
    let mut sorted = frametimes_ms.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let (one, point_one) = match def {
        LowDefinition::Integral => (integral_low(&sorted, 0.01), integral_low(&sorted, 0.001)),
        LowDefinition::Percentile => (
            percentile_low(&sorted, 0.99),
            percentile_low(&sorted, 0.999),
        ),
    };
    Some(Lows {
        one_percent: one,
        point_one_percent: point_one,
    })
}

/// `ascending` is sorted shortest first.
fn integral_low(ascending: &[f64], p: f64) -> f64 {
    let threshold = p * ascending.iter().sum::<f64>();
    let mut acc = 0.0;
    for &ft in ascending.iter().rev() {
        acc += ft;
        if acc >= threshold {
            return 1000.0 / ft;
        }
    }
    // Unreachable for p <= 1; fall back to the shortest frame.
    1000.0 / ascending[0]
}

fn percentile_low(ascending: &[f64], q: f64) -> f64 {
    let n = ascending.len();
    // The epsilon keeps products like 0.99 * 1000 from rounding up a rank.
    let rank = ((q * n as f64 - 1e-9).ceil() as usize).clamp(1, n);
    1000.0 / ascending[rank - 1]
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stutter {
    pub count: u32,
    pub time_percent: f64,
}

const STUTTER_HISTORY_S: f64 = 2.0;
const STUTTER_MIN_HISTORY: usize = 10;
const STUTTER_RATIO: f64 = 2.5;
const STUTTER_MIN_EXTRA_MS: f64 = 8.0;

/// Counts displayed frames that are both above 2.5x the median of the
/// displayed frames in the preceding 2 s and more than 8 ms above it.
/// `frames` must be ordered by `t_s`.
pub fn stutter(frames: &[FrameSample]) -> Stutter {
    let shown: Vec<(f64, f64)> = frames
        .iter()
        .filter(|f| f.displayed)
        .filter_map(|f| f.ms_between_display_change.map(|ms| (f.t_s, ms)))
        .collect();
    let total: f64 = shown.iter().map(|&(_, ms)| ms).sum();
    let mut count = 0u32;
    let mut stutter_ms = 0.0;
    let mut start = 0;
    let mut scratch: Vec<f64> = Vec::new();
    for (i, &(t, ft)) in shown.iter().enumerate() {
        while shown[start].0 < t - STUTTER_HISTORY_S {
            start += 1;
        }
        if i - start < STUTTER_MIN_HISTORY {
            continue;
        }
        scratch.clear();
        scratch.extend(shown[start..i].iter().map(|&(_, ms)| ms));
        scratch.sort_by(|a, b| a.total_cmp(b));
        let median = median_of_sorted(&scratch);
        if ft > STUTTER_RATIO * median && ft - median > STUTTER_MIN_EXTRA_MS {
            count += 1;
            stutter_ms += ft;
        }
    }
    Stutter {
        count,
        time_percent: if total > 0.0 {
            stutter_ms / total * 100.0
        } else {
            0.0
        },
    }
}

fn median_of_sorted(v: &[f64]) -> f64 {
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Mean PC latency (ms) over the displayed frames that have the value.
pub fn mean_pc_latency(frames: &[FrameSample]) -> Option<f64> {
    mean(frames, |f| f.ms_pc_latency)
}

/// Mean time from present to display (ms) over the displayed frames that
/// have the value.
pub fn mean_display_latency(frames: &[FrameSample]) -> Option<f64> {
    mean(frames, |f| f.ms_until_displayed)
}

fn mean(frames: &[FrameSample], get: impl Fn(&FrameSample) -> Option<f64>) -> Option<f64> {
    let (n, sum) = frames
        .iter()
        .filter(|f| f.displayed)
        .filter_map(get)
        .fold((0u32, 0.0), |(n, s), v| (n + 1, s + v));
    (n > 0).then(|| sum / f64::from(n))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bottleneck {
    Gpu,
    Cpu,
    Unknown,
}

const BOTTLENECK_MIN_FRAMES: usize = 30;
const BOTTLENECK_GPU_SHARE: f64 = 0.75;
const BOTTLENECK_BUSY_RATIO: f64 = 0.9;

/// GPU-bound when at least 75% of the app frames have a GPU busy time of at
/// least 90% of the app frametime, CPU-bound otherwise. With frame
/// generation only the frames with a PCL id are app frames.
pub fn bottleneck(frames: &[FrameSample], fg_suspected: bool) -> Bottleneck {
    if fg_suspected {
        return Bottleneck::Unknown;
    }
    let has_pcl = frames.iter().any(|f| f.pcl_frame_id.is_some());
    let mut valid = 0usize;
    let mut gpu = 0usize;
    for f in frames {
        if f.kind != FrameKind::App || (has_pcl && f.pcl_frame_id.is_none()) {
            continue;
        }
        let (Some(busy), Some(app)) = (f.ms_gpu_busy, f.ms_app_frametime) else {
            continue;
        };
        valid += 1;
        if busy >= BOTTLENECK_BUSY_RATIO * app {
            gpu += 1;
        }
    }
    if valid < BOTTLENECK_MIN_FRAMES {
        Bottleneck::Unknown
    } else if gpu as f64 >= BOTTLENECK_GPU_SHARE * valid as f64 {
        Bottleneck::Gpu
    } else {
        Bottleneck::Cpu
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::{FrameKind, FrameSample};

    fn f(t_s: f64, ft: f64) -> FrameSample {
        FrameSample {
            t_s,
            swapchain: 1,
            kind: FrameKind::App,
            displayed: true,
            ms_between_presents: ft,
            ms_between_display_change: Some(ft),
            ms_until_displayed: Some(ft),
            ms_app_frametime: Some(ft),
            ms_pc_latency: Some(ft),
            ms_gpu_busy: Some(ft),
            pcl_frame_id: None,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    /// Frames laid out back to back from t = 0, one per frametime.
    fn run(fts: &[f64]) -> Vec<FrameSample> {
        let mut t = 0.0;
        fts.iter()
            .map(|&ft| {
                let s = f(t, ft);
                t += ft / 1000.0;
                s
            })
            .collect()
    }

    #[test]
    fn displayed_fps_is_count_over_total_time() {
        let frames = run(&[10.0, 10.0, 20.0, 20.0]);
        assert!(close(displayed_fps(&frames).unwrap(), 1000.0 * 4.0 / 60.0));
        assert!(close(presented_fps(&frames).unwrap(), 1000.0 * 4.0 / 60.0));
        assert_eq!(displayed_fps(&[]), None);
        assert_eq!(presented_fps(&[]), None);
    }

    #[test]
    fn displayed_fps_ignores_frames_not_displayed() {
        let mut frames = run(&[10.0, 10.0, 10.0, 10.0]);
        frames[1].displayed = false;
        frames[2].ms_between_display_change = None;
        assert!(close(displayed_fps(&frames).unwrap(), 100.0));
        frames[0].displayed = false;
        frames[3].displayed = false;
        assert_eq!(displayed_fps(&frames), None);
    }

    #[test]
    fn integral_lows_match_a_hand_computed_case() {
        let mut fts = vec![10.0; 990];
        fts.extend([20.0; 9]);
        fts.push(100.0);
        let l = lows(&fts, LowDefinition::Integral).unwrap();
        assert!(close(l.one_percent, 50.0));
        assert!(close(l.point_one_percent, 10.0));
    }

    #[test]
    fn percentile_lows_use_nearest_rank() {
        let mut fts = vec![10.0; 990];
        fts.extend([40.0; 10]);
        let l = lows(&fts, LowDefinition::Percentile).unwrap();
        assert!(close(l.one_percent, 100.0));
        assert!(close(l.point_one_percent, 25.0));
    }

    #[test]
    fn lows_need_at_least_two_values() {
        assert!(lows(&[], LowDefinition::Integral).is_none());
        assert!(lows(&[10.0], LowDefinition::Percentile).is_none());
        assert!(lows(&[10.0, 10.0], LowDefinition::Integral).is_some());
    }

    #[test]
    fn stutter_needs_both_conditions() {
        let mut fts = vec![10.0; 200];
        fts.push(30.0);
        fts.push(24.0);
        let s = stutter(&run(&fts));
        assert_eq!(s.count, 1);
        let total: f64 = fts.iter().sum();
        assert!(close(s.time_percent, 30.0 / total * 100.0));
    }

    #[test]
    fn stutter_ignores_small_absolute_spikes() {
        let mut fts = vec![2.0; 200];
        fts.push(6.0);
        let s = stutter(&run(&fts));
        assert_eq!(s.count, 0);
        assert!(close(s.time_percent, 0.0));
    }

    #[test]
    fn stutter_skips_frames_without_enough_history() {
        let mut fts = vec![10.0; 5];
        fts.push(100.0);
        assert_eq!(stutter(&run(&fts)).count, 0);
        assert_eq!(stutter(&[]).count, 0);
    }

    fn gpu_frames(n: usize, above: usize) -> Vec<FrameSample> {
        (0..n)
            .map(|i| {
                let mut s = f(i as f64 * 0.01, 10.0);
                s.ms_gpu_busy = Some(if i < above { 9.0 } else { 5.0 });
                s
            })
            .collect()
    }

    #[test]
    fn bottleneck_is_gpu_at_seventy_five_percent() {
        assert_eq!(bottleneck(&gpu_frames(100, 75), false), Bottleneck::Gpu);
    }

    #[test]
    fn bottleneck_is_cpu_below() {
        assert_eq!(bottleneck(&gpu_frames(100, 74), false), Bottleneck::Cpu);
    }

    #[test]
    fn bottleneck_unknown_below_thirty_frames() {
        assert_eq!(bottleneck(&gpu_frames(29, 29), false), Bottleneck::Unknown);
        assert_eq!(bottleneck(&gpu_frames(30, 30), false), Bottleneck::Gpu);
    }

    #[test]
    fn bottleneck_unknown_when_fg_is_suspected() {
        assert_eq!(bottleneck(&gpu_frames(100, 100), true), Bottleneck::Unknown);
    }

    #[test]
    fn bottleneck_ignores_generated_frames() {
        let mut frames = gpu_frames(100, 100);
        for s in frames.iter_mut().step_by(2) {
            s.kind = FrameKind::GeneratedOther;
        }
        // 50 app frames remain, all GPU-bound.
        assert_eq!(bottleneck(&frames, false), Bottleneck::Gpu);
        for s in frames.iter_mut().skip(1).step_by(2).take(30) {
            s.kind = FrameKind::GeneratedOther;
        }
        assert_eq!(bottleneck(&frames, false), Bottleneck::Unknown);
    }

    #[test]
    fn bottleneck_uses_only_pcl_frames_when_present() {
        let mut frames = gpu_frames(100, 0); // all CPU-bound
        for (i, s) in frames.iter_mut().enumerate() {
            if i < 40 {
                s.pcl_frame_id = Some(i as u64 + 1);
                s.ms_gpu_busy = Some(9.5);
            }
        }
        // The 60 frames without an id would say Cpu; only the 40 GPU-heavy count.
        assert_eq!(bottleneck(&frames, false), Bottleneck::Gpu);
        // Fewer than 30 frames with an id: Unknown even though 100 are valid.
        for s in frames.iter_mut().skip(29) {
            s.pcl_frame_id = None;
        }
        assert_eq!(bottleneck(&frames, false), Bottleneck::Unknown);
    }

    #[test]
    fn latency_means_use_displayed_frames_with_values() {
        let mut frames = run(&[10.0, 10.0, 10.0, 10.0]);
        frames[0].ms_pc_latency = Some(20.0);
        frames[1].ms_pc_latency = Some(40.0);
        frames[2].ms_pc_latency = None;
        frames[3].ms_pc_latency = Some(1000.0);
        frames[3].displayed = false;
        assert!(close(mean_pc_latency(&frames).unwrap(), 30.0));
        frames[0].ms_until_displayed = Some(5.0);
        frames[1].ms_until_displayed = Some(15.0);
        frames[2].ms_until_displayed = None;
        frames[3].ms_until_displayed = Some(500.0);
        assert!(close(mean_display_latency(&frames).unwrap(), 10.0));
        assert_eq!(mean_pc_latency(&[]), None);
        assert_eq!(mean_display_latency(&[]), None);
    }
}
