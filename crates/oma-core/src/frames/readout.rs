//! All the frame metrics of one window in one struct, for the overlay and
//! the `frames:` diagnostics line.

use crate::frames::metrics::{
    bottleneck, displayed_fps, lows, mean_display_latency, mean_pc_latency, presented_fps, stutter,
    Bottleneck, LowDefinition, Lows, Stutter,
};
use crate::frames::{
    fg_multiplier, fg_suspected, pick_swapchain, rendered_fps, source_label, FrameSample,
    FrameWindow, Rendered, FG_WINDOW_S, FPS_WINDOW_S, LOWS_WINDOW_S,
};

/// The lows of one requested window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LowReadout {
    pub window_s: u32,
    pub definition: LowDefinition,
    pub lows: Option<Lows>,
}

/// Every figure of the main swapchain, anchored on the newest frame.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameReadout {
    pub fps_displayed: Option<f64>,
    pub fps_presented: Option<f64>,
    pub rendered: Rendered,
    /// `Reflex`, `XeSS-FG`, `AFMF`, `FG` or `FG?`.
    pub rendered_source: Option<&'static str>,
    pub fg_multiplier: Option<f64>,
    pub fg_suspected: bool,
    /// Mean over the last second.
    pub frametime_displayed_ms: Option<f64>,
    /// Mean over the last second.
    pub frametime_app_ms: Option<f64>,
    /// The `(10, Integral)` entry first, then one per requested window.
    pub lows: Vec<LowReadout>,
    pub stutter: Option<Stutter>,
    pub latency_pc_ms: Option<f64>,
    pub latency_display_ms: Option<f64>,
    /// `None` without `track_gpu`.
    pub bottleneck: Option<Bottleneck>,
    pub swapchain: Option<u64>,
}

/// The frames of `frames` (ordered by `t_s`) in the trailing `seconds`.
fn trailing(frames: &[FrameSample], seconds: f64) -> &[FrameSample] {
    let Some(newest) = frames.last().map(|f| f.t_s) else {
        return frames;
    };
    let start = frames.partition_point(|f| f.t_s < newest - seconds);
    &frames[start..]
}

fn displayed_frametimes(frames: &[FrameSample]) -> Vec<f64> {
    frames
        .iter()
        .filter(|f| f.displayed)
        .filter_map(|f| f.ms_between_display_change)
        .collect()
}

/// Reads `window`: FPS and latencies over 1 s, frame generation over 2 s,
/// stutter and bottleneck over 10 s, and the lows over 10 s plus each of
/// `low_windows`. The main swapchain is the one picked over the last 10 s.
pub fn read(
    window: &FrameWindow,
    low_windows: &[(u32, LowDefinition)],
    track_gpu: bool,
) -> FrameReadout {
    let longest = low_windows
        .iter()
        .map(|&(s, _)| f64::from(s))
        .fold(LOWS_WINDOW_S, f64::max);
    let all = window.last(longest);
    let swapchain = pick_swapchain(trailing(&all, LOWS_WINDOW_S));
    let long: Vec<FrameSample> = match swapchain {
        Some(sc) => all.into_iter().filter(|f| f.swapchain == sc).collect(),
        None => Vec::new(),
    };
    let main = trailing(&long, LOWS_WINDOW_S);
    let second = trailing(main, FPS_WINDOW_S);

    let fps_displayed = displayed_fps(second);
    let rendered = match rendered_fps(second) {
        Rendered::Unavailable | Rendered::FgSuspected => {
            if fg_suspected(trailing(main, FG_WINDOW_S)) {
                Rendered::FgSuspected
            } else {
                Rendered::Unavailable
            }
        }
        figure => figure,
    };
    let suspected = rendered == Rendered::FgSuspected;
    let rendered_source = match rendered {
        Rendered::Fps { source, .. } => Some(source_label(source, second)),
        Rendered::FgSuspected => Some("FG?"),
        Rendered::Unavailable => None,
    };

    let frametimes = displayed_frametimes(main);
    let mut lows_out = vec![LowReadout {
        window_s: LOWS_WINDOW_S as u32,
        definition: LowDefinition::Integral,
        lows: lows(&frametimes, LowDefinition::Integral),
    }];
    for &(window_s, definition) in low_windows {
        let times = displayed_frametimes(trailing(&long, f64::from(window_s)));
        lows_out.push(LowReadout {
            window_s,
            definition,
            lows: lows(&times, definition),
        });
    }

    let (n, app_sum) = second
        .iter()
        .filter_map(|f| f.ms_app_frametime)
        .fold((0u32, 0.0), |(n, s), v| (n + 1, s + v));

    FrameReadout {
        fps_displayed,
        fps_presented: presented_fps(second),
        rendered,
        rendered_source,
        fg_multiplier: fg_multiplier(fps_displayed, &rendered),
        fg_suspected: suspected,
        frametime_displayed_ms: fps_displayed.map(|f| 1000.0 / f),
        frametime_app_ms: (n > 0).then(|| app_sum / f64::from(n)),
        lows: lows_out,
        stutter: (!frametimes.is_empty()).then(|| stutter(main)),
        latency_pc_ms: mean_pc_latency(second),
        latency_display_ms: mean_display_latency(second),
        bottleneck: track_gpu.then(|| bottleneck(main, suspected)),
        swapchain,
    }
}
