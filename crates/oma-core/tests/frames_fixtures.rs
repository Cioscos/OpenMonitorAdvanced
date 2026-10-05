//! Frame metrics against the real PresentMon captures in `testdata/presentmon/`.

use std::collections::HashMap;
use std::path::PathBuf;

use oma_core::frames::metrics::{bottleneck, displayed_fps, stutter, Bottleneck, LowDefinition};
use oma_core::frames::{
    fg_multiplier, fg_suspected, pick_swapchain, read, rendered_fps, synthetic, FrameKind,
    FrameSample, FrameWindow, Rendered, RenderedSource, SyntheticProfile,
};

/// Reads a fixture by column name. Absent column or `NA` gives `None`;
/// `PCLFrameId` 0 gives `None`; `FrameType` follows SD3 (see [`frame_kind`]).
fn load(name: &str) -> Vec<FrameSample> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/presentmon")
        .join(format!("{name}.csv"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let mut lines = text.lines();
    let header: HashMap<&str, usize> = lines
        .next()
        .expect("header")
        .split(',')
        .enumerate()
        .map(|(i, c)| (c, i))
        .collect();
    lines
        .filter(|l| !l.is_empty())
        .map(|line| {
            let cols: Vec<&str> = line.split(',').collect();
            let raw = |c: &str| header.get(c).map(|&i| cols[i]);
            let num = |c: &str| raw(c).and_then(|v| v.parse::<f64>().ok());
            let qpc: f64 = num("TimeInQPC").expect("TimeInQPC");
            let swapchain = u64::from_str_radix(
                raw("SwapChainAddress")
                    .expect("SwapChainAddress")
                    .trim_start_matches("0x"),
                16,
            )
            .expect("swapchain");
            let kind = frame_kind(raw("FrameType"));
            FrameSample {
                t_s: qpc / 1e7,
                swapchain,
                kind,
                displayed: num("MsBetweenDisplayChange").is_some(),
                ms_between_presents: num("MsBetweenPresents").expect("MsBetweenPresents"),
                ms_between_display_change: num("MsBetweenDisplayChange"),
                ms_until_displayed: num("MsUntilDisplayed"),
                ms_app_frametime: num("MsBetweenAppStart"),
                ms_pc_latency: num("MsPCLatency"),
                ms_gpu_busy: num("MsGPUBusy"),
                pcl_frame_id: raw("PCLFrameId")
                    .and_then(|v| v.parse::<u64>().ok())
                    .filter(|&id| id != 0),
            }
        })
        .collect()
}

/// SD3: the service's `FrameType` mapping, mirrored for the fixtures.
fn frame_kind(text: Option<&str>) -> FrameKind {
    match text {
        None | Some("" | "NA" | "Unknown") => FrameKind::Unknown,
        Some("Application") => FrameKind::App,
        Some("Intel XeSS-FG") => FrameKind::GeneratedIntelXefg,
        Some("AMD AFMF") => FrameKind::GeneratedAmdAfmf,
        Some(_) => FrameKind::GeneratedOther,
    }
}

#[test]
fn frame_kind_follows_sd3() {
    for (text, expected) in [
        (Some("Application"), FrameKind::App),
        (Some("Intel XeSS-FG"), FrameKind::GeneratedIntelXefg),
        (Some("AMD AFMF"), FrameKind::GeneratedAmdAfmf),
        (Some("NVIDIA DLSS-FG"), FrameKind::GeneratedOther),
        (Some("Unknown"), FrameKind::Unknown),
        (Some("NA"), FrameKind::Unknown),
        (Some(""), FrameKind::Unknown),
        (None, FrameKind::Unknown),
    ] {
        assert_eq!(frame_kind(text), expected, "{text:?}");
    }
}

fn near(actual: f64, expected: f64, tol: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{what}: {actual} not within {tol} of {expected}"
    );
}

#[test]
fn displayed_fps_matches_the_readme() {
    for (name, expected) in [
        ("nofg", 74.2),
        ("nofg-pcl", 97.0),
        ("cpubound", 156.4),
        ("dlssfg", 130.1),
        ("dlssfg-pcl", 130.5),
        ("fsrfg", 125.8),
        ("fsrfg-pcl", 106.3),
        ("smooth", 157.0),
        ("smooth-pcl", 157.0),
    ] {
        near(displayed_fps(&load(name)).unwrap(), expected, 0.5, name);
    }
}

#[test]
fn reflex_rendered_fps_matches_the_readme() {
    for (name, expected) in [
        ("nofg-pcl", 96.9),
        ("dlssfg-pcl", 65.3),
        ("fsrfg-pcl", 53.1),
        ("smooth-pcl", 78.5),
    ] {
        match rendered_fps(&load(name)) {
            Rendered::Fps { fps, source } => {
                assert_eq!(source, RenderedSource::Reflex, "{name}");
                near(fps, expected, 1.0, name);
            }
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn heuristic_and_unavailable_without_pcl() {
    for name in ["dlssfg", "smooth"] {
        assert_eq!(rendered_fps(&load(name)), Rendered::FgSuspected, "{name}");
        assert!(fg_suspected(&load(name)), "{name}");
    }
    for name in ["nofg", "cpubound", "fsrfg"] {
        assert_eq!(rendered_fps(&load(name)), Rendered::Unavailable, "{name}");
    }
}

#[test]
fn multiplier_is_about_two_with_fg() {
    for name in ["dlssfg-pcl", "fsrfg-pcl", "smooth-pcl"] {
        let frames = load(name);
        let m = fg_multiplier(displayed_fps(&frames), &rendered_fps(&frames)).unwrap();
        assert!((1.9..=2.1).contains(&m), "{name}: {m}");
    }
}

#[test]
fn bottleneck_matches_the_readme() {
    for name in ["nofg", "nofg-pcl"] {
        assert_eq!(bottleneck(&load(name), false), Bottleneck::Gpu, "{name}");
    }
    assert_eq!(bottleneck(&load("cpubound"), false), Bottleneck::Cpu);
    assert_eq!(bottleneck(&load("dlssfg"), true), Bottleneck::Unknown);
}

#[test]
fn swapchain_is_the_single_address_in_the_file() {
    for name in ["nofg", "dlssfg-pcl", "smooth"] {
        let frames = load(name);
        assert_eq!(pick_swapchain(&frames), Some(frames[0].swapchain), "{name}");
    }
}

const ALL_FIXTURES: [&str; 9] = [
    "nofg",
    "nofg-pcl",
    "cpubound",
    "dlssfg",
    "dlssfg-pcl",
    "fsrfg",
    "fsrfg-pcl",
    "smooth",
    "smooth-pcl",
];

/// The pre-M7c stutter algorithm: a fresh sort of the preceding 2 s per frame.
fn stutter_reference(frames: &[FrameSample]) -> (u32, f64) {
    let shown: Vec<(f64, f64)> = frames
        .iter()
        .filter(|f| f.displayed)
        .filter_map(|f| f.ms_between_display_change.map(|ms| (f.t_s, ms)))
        .collect();
    let total: f64 = shown.iter().map(|&(_, ms)| ms).sum();
    let (mut count, mut stutter_ms, mut start) = (0u32, 0.0, 0);
    for (i, &(t, ft)) in shown.iter().enumerate() {
        while shown[start].0 < t - 2.0 {
            start += 1;
        }
        if i - start < 10 {
            continue;
        }
        let mut v: Vec<f64> = shown[start..i].iter().map(|&(_, ms)| ms).collect();
        v.sort_by(|a, b| a.total_cmp(b));
        let n = v.len();
        let median = if n % 2 == 1 {
            v[n / 2]
        } else {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        };
        if ft > 2.5 * median && ft - median > 8.0 {
            count += 1;
            stutter_ms += ft;
        }
    }
    let pct = if total > 0.0 {
        stutter_ms / total * 100.0
    } else {
        0.0
    };
    (count, pct)
}

#[test]
fn stutter_incremental_matches_reference() {
    let mut inputs: Vec<(String, Vec<FrameSample>)> = ALL_FIXTURES
        .iter()
        .map(|n| ((*n).to_owned(), load(n)))
        .collect();
    let profile = SyntheticProfile {
        base_fps: 60.0,
        fg_factor: 2,
        jitter_ms: 3.0,
        stutter_every: Some(37),
        pcl: false,
        gpu_busy_ratio: Some(0.9),
    };
    inputs.push(("synthetic".to_owned(), synthetic(7, &profile, 30.0)));
    for (name, frames) in inputs {
        let (count, pct) = stutter_reference(&frames);
        let got = stutter(&frames);
        assert_eq!(got.count, count, "{name}");
        assert!((got.time_percent - pct).abs() < 1e-9, "{name}");
    }
}

fn window_of(frames: &[FrameSample], max_age_s: f64) -> FrameWindow {
    let mut w = FrameWindow::new(max_age_s);
    for f in frames {
        w.push(*f);
    }
    w
}

/// The README figures are means over the whole 10 s capture, while the
/// readout looks at the last second; so the readout must equal the metrics of
/// the last second exactly and stay near the README (a loose sanity bound).
#[test]
fn readout_matches_the_fixture_table() {
    for (name, shown, pcl) in [
        ("nofg-pcl", 97.0, true),
        ("dlssfg-pcl", 130.5, true),
        ("fsrfg-pcl", 106.3, true),
        ("smooth-pcl", 157.0, true),
        ("nofg", 74.2, false),
        ("cpubound", 156.4, false),
        ("dlssfg", 130.1, false),
        ("fsrfg", 125.8, false),
        ("smooth", 157.0, false),
    ] {
        let frames = load(name);
        let newest = frames.last().unwrap().t_s;
        let second: Vec<FrameSample> = frames
            .iter()
            .copied()
            .filter(|f| f.t_s >= newest - 1.0)
            .collect();
        let r = read(&window_of(&frames, 10.0), &[], true);
        let fps = r.fps_displayed.unwrap();
        near(fps, displayed_fps(&second).unwrap(), 1e-9, name);
        near(fps, shown, shown * 0.25, name);
        if let Rendered::Fps { fps, source } = rendered_fps(&second) {
            match r.rendered {
                Rendered::Fps {
                    fps: got,
                    source: got_source,
                } => {
                    near(got, fps, 1e-9, name);
                    assert_eq!(got_source, source, "{name}");
                }
                other => panic!("{name}: {other:?}"),
            }
        }
        if pcl {
            assert!(matches!(r.rendered, Rendered::Fps { .. }), "{name}");
            assert_eq!(r.rendered_source, Some("Reflex"), "{name}");
        }
        assert!(r.swapchain.is_some(), "{name}");
        assert!(r.stutter.is_some(), "{name}");
    }
    let fg = read(&window_of(&load("dlssfg"), 10.0), &[], false);
    assert!(fg.fg_suspected);
    assert_eq!(fg.rendered_source, Some("FG?"));
}

#[test]
fn readout_lows_for_requested_windows() {
    let w = window_of(&load("nofg"), 30.0);
    let base = read(&w, &[], false);
    assert_eq!(base.lows.len(), 1);
    assert_eq!(
        (base.lows[0].window_s, base.lows[0].definition),
        (10, LowDefinition::Integral)
    );
    let more = read(&w, &[(30, LowDefinition::Percentile)], false);
    assert_eq!(more.lows.len(), 2);
    assert_eq!(more.lows[1].window_s, 30);
    assert_eq!(more.lows[1].definition, LowDefinition::Percentile);
    assert!(more.lows[1].lows.is_some());
    assert_eq!(more.lows[0], base.lows[0]);
}

#[test]
fn readout_without_track_gpu_has_no_bottleneck() {
    let w = window_of(&load("nofg"), 10.0);
    assert_eq!(read(&w, &[], false).bottleneck, None);
    assert!(read(&w, &[], true).bottleneck.is_some());
}
