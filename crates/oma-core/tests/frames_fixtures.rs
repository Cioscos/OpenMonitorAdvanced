//! Frame metrics against the real PresentMon captures in `testdata/presentmon/`.

use std::collections::HashMap;
use std::path::PathBuf;

use oma_core::frames::metrics::{bottleneck, displayed_fps, Bottleneck};
use oma_core::frames::{
    fg_multiplier, fg_suspected, pick_swapchain, rendered_fps, FrameKind, FrameSample, Rendered,
    RenderedSource,
};

/// Reads a fixture by column name. Absent column or `NA` gives `None`;
/// `PCLFrameId` 0 gives `None`; an absent `FrameType` gives `Unknown`.
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
            let kind = match raw("FrameType") {
                Some("Application") => FrameKind::App,
                _ => FrameKind::Unknown,
            };
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
