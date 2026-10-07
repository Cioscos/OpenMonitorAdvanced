//! Load levels of the GPU modes (plan DG10). Pure: the engine asks for the level at an
//! elapsed time and pauses after each submission to reach it.

use crate::rng::Xoshiro256ss;

/// `ramp` (S7): 20 to 100% in steps of 5%, 17 levels, each for 1/17 of the phase.
pub fn ramp_level(elapsed_ms: u64, phase_ms: u64) -> u8 {
    const LEVELS: u64 = 17;
    let step = if phase_ms == 0 {
        LEVELS - 1
    } else {
        (u128::from(elapsed_ms) * u128::from(LEVELS) / u128::from(phase_ms))
            .min(u128::from(LEVELS - 1)) as u64
    };
    20 + 5 * step as u8
}

/// `alternate` (S8): periods at 100% and 15% in turn, each 10 to 500 ms long, drawn from
/// the phase seed.
#[derive(Debug, Clone)]
pub struct Alternate {
    rng: Xoshiro256ss,
    level: u8,
    /// Elapsed time at which the current period ends.
    end_ms: u64,
}

impl Alternate {
    pub fn new(seed: u64) -> Self {
        let mut rng = Xoshiro256ss::new(seed);
        let end_ms = Self::period(&mut rng);
        Self {
            rng,
            level: 100,
            end_ms,
        }
    }

    fn period(rng: &mut Xoshiro256ss) -> u64 {
        10 + rng.next_u64() % 491
    }

    /// The level at `elapsed_ms`; the calls must come with non-decreasing times.
    pub fn level_at(&mut self, elapsed_ms: u64) -> u8 {
        while elapsed_ms >= self.end_ms {
            self.level = if self.level == 100 { 15 } else { 100 };
            self.end_ms += Self::period(&mut self.rng);
        }
        self.level
    }
}

/// `pause_resume` (S9): 60 s of full load, then 12 s idle, over and over.
pub fn pause_resume_active(elapsed_ms: u64) -> bool {
    elapsed_ms % 72_000 < 60_000
}

/// Pause after a finished submission of `submit_ms` so the GPU works `level`% of the time.
pub fn idle_after_ms(level: u8, submit_ms: f64) -> f64 {
    let level = f64::from(level.clamp(1, 100));
    submit_ms * (100.0 - level) / level
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_has_17_levels_from_20_to_100() {
        let phase = 17_000;
        let levels: Vec<u8> = (0..phase)
            .step_by(1000)
            .map(|t| ramp_level(t, phase))
            .collect();
        assert_eq!(levels, (0..17).map(|i| 20 + 5 * i).collect::<Vec<u8>>());
        assert_eq!(ramp_level(999, phase), 20);
        assert_eq!(ramp_level(phase - 1, phase), 100);
        // Past the end, and with an empty phase, the last level holds.
        assert_eq!(ramp_level(phase * 2, phase), 100);
        assert_eq!(ramp_level(0, 0), 100);
    }

    #[test]
    fn alternate_periods_are_10_to_500_ms_and_seeded() {
        let periods = |seed| {
            let mut alt = Alternate::new(seed);
            let mut out = Vec::new();
            let (mut level, mut start) = (alt.level_at(0), 0);
            assert_eq!(level, 100);
            for t in 1..60_000 {
                let now = alt.level_at(t);
                if now != level {
                    assert!(matches!(now, 15 | 100));
                    out.push(t - start);
                    (level, start) = (now, t);
                }
            }
            out
        };
        let a = periods(7);
        assert!(a.len() > 100);
        assert!(a.iter().all(|p| (10..=500).contains(p)), "{a:?}");
        assert_eq!(a, periods(7));
        assert_ne!(a, periods(8));
        // Big jumps skip whole periods and still land on a valid level.
        let mut alt = Alternate::new(7);
        assert!(matches!(alt.level_at(1_000_000), 15 | 100));
    }

    #[test]
    fn pause_resume_is_60_on_12_off() {
        assert!(pause_resume_active(0));
        assert!(pause_resume_active(59_999));
        assert!(!pause_resume_active(60_000));
        assert!(!pause_resume_active(71_999));
        assert!(pause_resume_active(72_000));
        assert!(!pause_resume_active(72_000 + 60_000));
    }

    #[test]
    fn idle_keeps_the_duty_cycle() {
        assert_eq!(idle_after_ms(25, 40.0), 120.0);
        assert_eq!(idle_after_ms(100, 40.0), 0.0);
        assert_eq!(idle_after_ms(50, 20.0), 20.0);
        // 0% would never resume: it counts as 1%.
        assert_eq!(idle_after_ms(0, 1.0), 99.0);
    }
}
