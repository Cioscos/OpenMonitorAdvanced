//! Swapchain choice: the swapchain that presents the most frames wins.

use crate::frames::{FrameSample, FPS_WINDOW_S};

/// The swapchain with the most displayed frames in the last second (relative
/// to the newest `t_s`); ties go to the swapchain with the most recent frame.
pub fn pick_swapchain(frames: &[FrameSample]) -> Option<u64> {
    let newest = frames
        .iter()
        .map(|f| f.t_s)
        .fold(f64::NEG_INFINITY, f64::max);
    if !newest.is_finite() {
        return None;
    }
    // (swapchain, displayed frames, newest frame time)
    let mut tally: Vec<(u64, u32, f64)> = Vec::new();
    for f in frames.iter().filter(|f| f.t_s >= newest - FPS_WINDOW_S) {
        let at = match tally.iter().position(|e| e.0 == f.swapchain) {
            Some(i) => i,
            None => {
                tally.push((f.swapchain, 0, f64::NEG_INFINITY));
                tally.len() - 1
            }
        };
        tally[at].1 += u32::from(f.displayed);
        tally[at].2 = tally[at].2.max(f.t_s);
    }
    tally
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(a.2.total_cmp(&b.2)))
        .map(|e| e.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::FrameKind;

    fn sc(t_s: f64, swapchain: u64, displayed: bool) -> FrameSample {
        FrameSample {
            t_s,
            swapchain,
            kind: FrameKind::App,
            displayed,
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
    fn picks_the_swapchain_with_most_displayed_frames() {
        let mut frames = Vec::new();
        for i in 0..20 {
            let t = f64::from(i) * 0.04;
            frames.push(sc(t, 0xA, true));
            if i % 2 == 0 {
                frames.push(sc(t + 0.01, 0xB, true));
            }
            // Presented-but-not-displayed frames do not count.
            frames.push(sc(t + 0.02, 0xC, false));
            frames.push(sc(t + 0.021, 0xC, false));
        }
        assert_eq!(pick_swapchain(&frames), Some(0xA));
        assert_eq!(pick_swapchain(&[]), None);
    }

    #[test]
    fn ignores_frames_older_than_one_second() {
        let mut frames: Vec<FrameSample> = (0..50)
            .map(|i| sc(f64::from(i) * 0.01, 0xA, true))
            .collect();
        frames.extend((0..5).map(|i| sc(5.0 + f64::from(i) * 0.01, 0xB, true)));
        assert_eq!(pick_swapchain(&frames), Some(0xB));
    }

    #[test]
    fn tie_goes_to_the_most_recent() {
        let frames = vec![
            sc(1.0, 0xA, true),
            sc(1.1, 0xB, true),
            sc(1.2, 0xA, true),
            sc(1.3, 0xB, true),
        ];
        assert_eq!(pick_swapchain(&frames), Some(0xB));
        let frames = vec![
            sc(1.0, 0xB, true),
            sc(1.1, 0xA, true),
            sc(1.2, 0xB, true),
            sc(1.3, 0xA, true),
        ];
        assert_eq!(pick_swapchain(&frames), Some(0xA));
    }
}
