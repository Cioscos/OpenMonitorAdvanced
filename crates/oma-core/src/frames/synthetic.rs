//! Deterministic synthetic frames for tests and for the demo backend.

use crate::frames::{FrameKind, FrameSample};

#[derive(Debug, Clone, PartialEq)]
pub struct SyntheticProfile {
    pub base_fps: f64,
    pub fg_factor: u8,
    pub jitter_ms: f64,
    pub stutter_every: Option<u32>,
    pub pcl: bool,
    pub gpu_busy_ratio: Option<f64>,
}

/// Why a [`SyntheticProfile`] was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SyntheticError {
    #[error("base FPS must be finite and within 1..=1000")]
    BaseFps,
    #[error("frame generation factor must be within 1..=4")]
    FgFactor,
    #[error("jitter must be finite and not negative")]
    Jitter,
    #[error("GPU busy ratio must be finite and not negative")]
    GpuBusyRatio,
}

impl SyntheticProfile {
    /// Checks the inputs `synthetic` can safely turn into frames.
    pub fn validate(&self) -> Result<(), SyntheticError> {
        if !(self.base_fps.is_finite() && (1.0..=1000.0).contains(&self.base_fps)) {
            return Err(SyntheticError::BaseFps);
        }
        if !(1..=4).contains(&self.fg_factor) {
            return Err(SyntheticError::FgFactor);
        }
        if !(self.jitter_ms.is_finite() && self.jitter_ms >= 0.0) {
            return Err(SyntheticError::Jitter);
        }
        if self
            .gpu_busy_ratio
            .is_some_and(|r| !(r.is_finite() && r >= 0.0))
        {
            return Err(SyntheticError::GpuBusyRatio);
        }
        Ok(())
    }
}

/// The longest capture `synthetic` produces.
const MAX_DURATION_S: f64 = 600.0;

/// xorshift64* generator.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [-1, 1).
    fn signed_unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

const GENERATED_PRESENT_MS: f64 = 0.25;

/// Frames for `duration_s` seconds of a made-up game. Each app frame is
/// followed by `fg_factor - 1` generated frames (labelled `GeneratedOther`,
/// or, with `pcl`, plain `App` frames without an id, like DLSS FG captures).
/// An invalid profile, or a duration that is not finite, negative or over
/// 600 s, gives no frames.
pub fn synthetic(seed: u64, profile: &SyntheticProfile, duration_s: f64) -> Vec<FrameSample> {
    if profile.validate().is_err()
        || !duration_s.is_finite()
        || !(0.0..=MAX_DURATION_S).contains(&duration_s)
    {
        return Vec::new();
    }
    let mut rng = Rng(if seed == 0 {
        0x9E37_79B9_7F4A_7C15
    } else {
        seed
    });
    let fg = profile.fg_factor;
    let base_ms = 1000.0 / profile.base_fps;
    let mut out = Vec::new();
    let (mut t, mut app_index, mut pcl_id) = (0.0_f64, 0u32, 1u64);
    while t < duration_s {
        let slow = profile
            .stutter_every
            .is_some_and(|n| n > 0 && app_index % n == n - 1);
        let app_ms = if slow { base_ms * 4.0 } else { base_ms };
        let shown_ms = app_ms / f64::from(fg);
        for k in 0..fg {
            let is_app = k == 0;
            let jitter = rng.signed_unit() * profile.jitter_ms;
            let present_ms = if !is_app {
                GENERATED_PRESENT_MS
            } else if fg > 1 {
                (app_ms - f64::from(fg - 1) * GENERATED_PRESENT_MS).max(GENERATED_PRESENT_MS)
            } else {
                app_ms
            };
            let kind = if is_app || profile.pcl {
                FrameKind::App
            } else {
                FrameKind::GeneratedOther
            };
            out.push(FrameSample {
                t_s: t + f64::from(k) * shown_ms / 1000.0,
                swapchain: 1,
                kind,
                displayed: true,
                ms_between_presents: present_ms,
                ms_between_display_change: Some((shown_ms + jitter).max(0.01)),
                ms_until_displayed: Some(5.0),
                ms_app_frametime: is_app.then_some(app_ms),
                ms_pc_latency: (is_app && profile.pcl).then_some(25.0),
                ms_gpu_busy: profile
                    .gpu_busy_ratio
                    .filter(|_| is_app)
                    .map(|r| r * app_ms),
                pcl_frame_id: (is_app && profile.pcl).then_some(pcl_id),
            });
        }
        pcl_id += 1;
        app_index += 1;
        t += app_ms / 1000.0;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::generation::{rendered_fps, Rendered, RenderedSource};
    use crate::frames::metrics::{displayed_fps, stutter};

    fn profile() -> SyntheticProfile {
        SyntheticProfile {
            base_fps: 60.0,
            fg_factor: 1,
            jitter_ms: 0.0,
            stutter_every: None,
            pcl: false,
            gpu_busy_ratio: None,
        }
    }

    #[test]
    fn same_seed_same_frames() {
        let p = SyntheticProfile {
            jitter_ms: 1.0,
            ..profile()
        };
        assert_eq!(synthetic(7, &p, 2.0), synthetic(7, &p, 2.0));
    }

    #[test]
    fn different_seed_different_jitter() {
        let p = SyntheticProfile {
            jitter_ms: 1.0,
            ..profile()
        };
        assert_ne!(synthetic(1, &p, 2.0), synthetic(2, &p, 2.0));
    }

    #[test]
    fn fg_factor_two_doubles_displayed_over_rendered() {
        let p = SyntheticProfile {
            fg_factor: 2,
            pcl: true,
            ..profile()
        };
        let frames = synthetic(3, &p, 5.0);
        let shown = displayed_fps(&frames).unwrap();
        assert!((118.0..=122.0).contains(&shown), "displayed {shown}");
        match rendered_fps(&frames) {
            Rendered::Fps { fps, source } => {
                assert_eq!(source, RenderedSource::Reflex);
                assert!((59.0..=61.0).contains(&fps), "rendered {fps}");
            }
            other => panic!("expected Fps, got {other:?}"),
        }
    }

    #[test]
    fn generated_frames_are_marked_without_pcl() {
        let p = SyntheticProfile {
            fg_factor: 2,
            ..profile()
        };
        let frames = synthetic(3, &p, 2.0);
        assert!(frames.iter().any(|f| f.kind.is_generated()));
        assert!(frames.iter().all(|f| f.pcl_frame_id.is_none()));
    }

    #[test]
    fn gpu_busy_ratio_applies_to_app_frames() {
        let p = SyntheticProfile {
            gpu_busy_ratio: Some(0.95),
            ..profile()
        };
        let frames = synthetic(1, &p, 1.0);
        let f = &frames[0];
        assert!((f.ms_gpu_busy.unwrap() - 0.95 * f.ms_app_frametime.unwrap()).abs() < 1e-9);
    }

    #[test]
    fn stutter_every_shows_up_in_stutter_count() {
        let p = SyntheticProfile {
            stutter_every: Some(60),
            ..profile()
        };
        let calm = synthetic(1, &profile(), 10.0);
        let rough = synthetic(1, &p, 10.0);
        assert_eq!(stutter(&calm).count, 0);
        assert!(stutter(&rough).count >= 5);
    }
}

#[cfg(test)]
mod guard_tests {
    use super::*;

    fn ok() -> SyntheticProfile {
        SyntheticProfile {
            base_fps: 60.0,
            fg_factor: 1,
            jitter_ms: 0.0,
            stutter_every: None,
            pcl: false,
            gpu_busy_ratio: None,
        }
    }

    #[test]
    fn synthetic_rejects_out_of_range_inputs() {
        assert!(ok().validate().is_ok());
        assert!(!synthetic(1, &ok(), 1.0).is_empty());
        for base_fps in [0.0, 1001.0, f64::NAN, f64::INFINITY, -5.0] {
            let p = SyntheticProfile { base_fps, ..ok() };
            assert!(p.validate().is_err(), "{base_fps}");
            assert!(synthetic(1, &p, 1.0).is_empty(), "{base_fps}");
        }
        for fg_factor in [0, 5] {
            let p = SyntheticProfile { fg_factor, ..ok() };
            assert!(synthetic(1, &p, 1.0).is_empty(), "fg {fg_factor}");
        }
        for jitter_ms in [f64::NAN, -1.0, f64::INFINITY] {
            let p = SyntheticProfile { jitter_ms, ..ok() };
            assert!(synthetic(1, &p, 1.0).is_empty(), "jitter {jitter_ms}");
        }
        let p = SyntheticProfile {
            gpu_busy_ratio: Some(f64::NAN),
            ..ok()
        };
        assert!(synthetic(1, &p, 1.0).is_empty());
        for duration in [-1.0, 601.0, f64::NAN, f64::INFINITY] {
            assert!(synthetic(1, &ok(), duration).is_empty(), "{duration}");
        }
    }
}
