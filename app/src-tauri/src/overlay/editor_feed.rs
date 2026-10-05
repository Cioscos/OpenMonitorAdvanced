//! The editor's frame data (DD14, DD15): synthetic frames for the canvas and
//! the preview while no game is the target, and the payload of the
//! `overlay-editor-data` event. Pure.

use oma_core::frames::{synthetic, FrameWindow, SyntheticProfile, LOWS_WINDOW_S};
use oma_ipc::overlay::{FrameMetrics, WireFrameTime};
use serde::Serialize;

/// The made-up game of the editor (DD15).
pub const EDITOR_SYNTHETIC: SyntheticProfile = SyntheticProfile {
    base_fps: 72.0,
    fg_factor: 2,
    jitter_ms: 0.8,
    stutter_every: Some(90),
    pcl: true,
    gpu_busy_ratio: Some(0.9),
};

/// The most a single [`SyntheticFeed::advance`] generates.
const MAX_CATCH_UP_S: f64 = 2.0;
const SEED: u64 = 0x0E_D170;

/// Synthetic frames one second at a time, up to the caller's clock, in a
/// window of their own (10 s).
pub struct SyntheticFeed {
    window: FrameWindow,
    generated_to_s: f64,
    seed: u64,
    /// The last PCL frame id given, so that ids keep growing across seconds.
    pcl_id: u64,
}

impl SyntheticFeed {
    pub fn new() -> Self {
        Self {
            window: FrameWindow::new(LOWS_WINDOW_S),
            generated_to_s: f64::NEG_INFINITY,
            seed: SEED,
            pcl_id: 0,
        }
    }

    /// Generates the whole seconds missing up to `now_s`; after a long pause
    /// (or the first time) only the last 2 s.
    pub fn advance(&mut self, now_s: f64) {
        if now_s - self.generated_to_s > MAX_CATCH_UP_S {
            self.generated_to_s = now_s - MAX_CATCH_UP_S;
        }
        while self.generated_to_s + 1.0 <= now_s {
            let (start, mut end, mut last_id) = (self.generated_to_s, 1.0_f64, self.pcl_id);
            for mut f in synthetic(self.seed, &EDITOR_SYNTHETIC, 1.0) {
                // The second ends with its last app frame, which may run
                // past 1 s: the next one starts there, without overlap.
                if let Some(ms) = f.ms_app_frametime {
                    end = end.max(f.t_s + ms / 1_000.0);
                }
                f.t_s += start;
                f.pcl_frame_id = f.pcl_frame_id.map(|id| {
                    last_id = self.pcl_id + id;
                    last_id
                });
                self.window.push(f);
            }
            self.pcl_id = last_id;
            self.seed = self.seed.wrapping_add(1);
            self.generated_to_s = start + end;
        }
    }

    pub fn window(&self) -> &FrameWindow {
        &self.window
    }
}

impl Default for SyntheticFeed {
    fn default() -> Self {
        Self::new()
    }
}

/// The `overlay-editor-data` payload: the latest metrics and the frame
/// times new since the previous one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorData {
    pub metrics: FrameMetrics,
    pub frame_times: Vec<WireFrameTime>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(feed: &SyntheticFeed) -> Vec<f64> {
        feed.window()
            .last(f64::INFINITY)
            .iter()
            .map(|f| f.t_s)
            .collect()
    }

    #[test]
    fn synthetic_feed_is_deterministic_and_bounded() {
        let (mut a, mut b) = (SyntheticFeed::new(), SyntheticFeed::new());
        for now in [100.0, 100.5, 101.2, 103.0] {
            a.advance(now);
            b.advance(now);
        }
        let frames = a.window().last(f64::INFINITY);
        assert_eq!(frames, b.window().last(f64::INFINITY));
        assert!(!frames.is_empty());
        // Frames of a second may run a few ms past it.
        assert!(frames.iter().all(|f| f.t_s < 103.05));
        let t: Vec<f64> = frames.iter().map(|f| f.t_s).collect();
        assert!(t.windows(2).all(|w| w[1] > w[0]), "seconds do not overlap");
        // PCL ids keep growing across the generated seconds (the rendered
        // FPS reads them over the whole window).
        let ids: Vec<u64> = frames.iter().filter_map(|f| f.pcl_frame_id).collect();
        assert!(ids.windows(2).all(|w| w[1] > w[0]), "{ids:?}");

        // After a 30 s pause, at most 2 s of new frames.
        let before = times(&a).last().copied().unwrap();
        a.advance(133.0);
        let new: Vec<f64> = times(&a).into_iter().filter(|&t| t > before).collect();
        assert!(!new.is_empty());
        let span = new.last().unwrap() - new.first().unwrap();
        assert!(span < 2.05, "{span}");
        assert!(*new.first().unwrap() >= 131.0 - 1e-9);
    }

    #[test]
    fn first_advance_fills_at_most_two_seconds() {
        let mut feed = SyntheticFeed::new();
        feed.advance(5_000.0);
        let t = times(&feed);
        assert!(t.first().unwrap() >= &4_998.0 && t.last().unwrap() < &5_000.05);
        // About 72 app frames a second, each followed by a generated one;
        // a second may run a few ms long, so one or two of them.
        assert!((140..=300).contains(&t.len()), "{}", t.len());
    }
}
