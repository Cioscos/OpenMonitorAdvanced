//! Evaluation of overlay blocks: rolling statistics, thresholds, frame
//! generation state and `visibleIf`. Pure, shared by controller and renderer.

use std::collections::VecDeque;

use super::Rgba;
use super::{CompareOp, Source, Stat, StatOp, Threshold, ThresholdTarget, VisibleIf};

/// Samples of one source, kept for the longest window any block asks for.
#[derive(Debug, Clone)]
pub struct StatRing {
    max_window_s: f64,
    samples: VecDeque<(f64, f64)>,
}

impl StatRing {
    pub fn new(max_window_s: u32) -> Self {
        Self {
            max_window_s: f64::from(max_window_s),
            samples: VecDeque::new(),
        }
    }

    /// Adds a sample at `t_s` seconds. Absent and non-finite values are
    /// skipped, so the newest sample keeps the time of the newest datum.
    pub fn push(&mut self, t_s: f64, value: Option<f64>) {
        let Some(v) = value.filter(|v| v.is_finite() && t_s.is_finite()) else {
            return;
        };
        self.samples.push_back((t_s, v));
        let oldest = t_s - self.max_window_s;
        while self.samples.front().is_some_and(|&(t, _)| t < oldest) {
            self.samples.pop_front();
        }
    }

    /// The kept samples as `(t_s, value)`, oldest first (for the charts).
    pub fn samples(&self) -> impl DoubleEndedIterator<Item = (f64, f64)> + '_ {
        self.samples.iter().copied()
    }

    /// `current` is the last value; `min`, `avg` and `max` cover the samples
    /// of the last `stat.window` seconds before the newest one (inclusive).
    pub fn value(&self, stat: &Stat) -> Option<f64> {
        let &(newest, last) = self.samples.back()?;
        if stat.op == StatOp::Current {
            return Some(last);
        }
        let from = newest - f64::from(stat.window);
        let in_window = self
            .samples
            .iter()
            .rev()
            .take_while(|&&(t, _)| t >= from)
            .map(|&(_, v)| v);
        match stat.op {
            StatOp::Current => Some(last),
            StatOp::Min => in_window.reduce(f64::min),
            StatOp::Max => in_window.reduce(f64::max),
            StatOp::Avg => {
                let (sum, n) = in_window.fold((0.0, 0u32), |(s, n), v| (s + v, n + 1));
                (n > 0).then(|| sum / f64::from(n))
            }
        }
    }
}

fn compare(op: CompareOp, value: f64, limit: f64) -> bool {
    match op {
        CompareOp::Gt => value > limit,
        CompareOp::Ge => value >= limit,
        CompareOp::Lt => value < limit,
        CompareOp::Le => value <= limit,
    }
}

/// The colour of the first true rule for `target`, on the value as it is
/// before any rounding for display.
pub fn threshold_color(
    thresholds: &[Threshold],
    target: ThresholdTarget,
    value: Option<f64>,
) -> Option<Rgba> {
    let v = value.filter(|v| v.is_finite())?;
    thresholds
        .iter()
        .filter(|t| t.target == target)
        .find(|t| compare(t.op, v, t.value))
        .map(|t| t.color)
}

/// Frame generation is in use: multiplier above 1.2, or the "FG?" heuristic.
pub fn fg_active(multiplier: Option<f64>, fg_suspected: bool) -> bool {
    fg_suspected || multiplier.is_some_and(|m| m > 1.2)
}

/// Whether a block shows. No condition shows; an absent source hides.
pub fn is_visible(
    cond: Option<&VisibleIf>,
    lookup: &dyn Fn(&Source, &Stat) -> Option<f64>,
    fg_active: bool,
) -> bool {
    match cond {
        None => true,
        Some(VisibleIf::Fg(_)) => fg_active,
        Some(VisibleIf::Compare(c)) => lookup(&c.source, &c.stat)
            .filter(|v| v.is_finite())
            .is_some_and(|v| compare(c.op, v, c.value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::{Comparison, FgCondition, FgState};

    fn st(op: StatOp, window: u32) -> Stat {
        Stat {
            op,
            window,
            ..Stat::default()
        }
    }

    fn th(op: CompareOp, value: f64, rgb: (u8, u8, u8), target: ThresholdTarget) -> Threshold {
        Threshold {
            op,
            value,
            color: Rgba::rgb(rgb.0, rgb.1, rgb.2),
            target,
        }
    }

    #[test]
    fn stat_ring_min_avg_max_over_window() {
        let mut r = StatRing::new(10);
        for (t, v) in [(0.0, 100.0), (5.0, 10.0), (8.0, 20.0), (10.0, 30.0)] {
            r.push(t, Some(v));
        }
        // Last 5 s relative to t = 10: samples at 5, 8, 10.
        assert_eq!(r.value(&st(StatOp::Min, 5)), Some(10.0));
        assert_eq!(r.value(&st(StatOp::Max, 5)), Some(30.0));
        assert_eq!(r.value(&st(StatOp::Avg, 5)), Some(20.0));
        // Last 10 s: the sample at 0 is on the edge and counts.
        assert_eq!(r.value(&st(StatOp::Max, 10)), Some(100.0));
        // Beyond the ring's window nothing older is kept.
        r.push(11.0, Some(1.0));
        assert_eq!(r.value(&st(StatOp::Max, 600)), Some(30.0));
    }

    #[test]
    fn stat_ring_samples_oldest_first() {
        let mut r = StatRing::new(5);
        assert_eq!(r.samples().count(), 0);
        for (t, v) in [(0.0, 1.0), (3.0, 2.0), (6.0, 3.0)] {
            r.push(t, Some(v));
        }
        // The sample at 0 is older than the 5 s window and is gone.
        let s: Vec<_> = r.samples().collect();
        assert_eq!(s, vec![(3.0, 2.0), (6.0, 3.0)]);
    }

    #[test]
    fn stat_current_is_last_value() {
        let mut r = StatRing::new(10);
        assert_eq!(r.value(&Stat::default()), None);
        r.push(0.0, Some(1.0));
        r.push(1.0, Some(7.0));
        assert_eq!(r.value(&Stat::default()), Some(7.0));
    }

    #[test]
    fn absent_values_do_not_enter_the_ring() {
        let mut r = StatRing::new(10);
        r.push(0.0, Some(4.0));
        r.push(1.0, None);
        r.push(2.0, Some(f64::NAN));
        r.push(3.0, Some(f64::INFINITY));
        assert_eq!(r.value(&Stat::default()), Some(4.0));
        // The window is relative to the newest datum, which is the one at t = 0.
        assert_eq!(r.value(&st(StatOp::Avg, 1)), Some(4.0));
    }

    #[test]
    fn first_true_threshold_wins_per_target() {
        let t = [
            th(CompareOp::Gt, 90.0, (255, 0, 0), ThresholdTarget::Value),
            th(CompareOp::Gt, 50.0, (255, 255, 0), ThresholdTarget::Value),
            th(CompareOp::Gt, 50.0, (0, 0, 255), ThresholdTarget::Panel),
        ];
        let c = |target, v| threshold_color(&t, target, Some(v));
        assert_eq!(c(ThresholdTarget::Value, 95.0), Some(Rgba::rgb(255, 0, 0)));
        assert_eq!(
            c(ThresholdTarget::Value, 60.0),
            Some(Rgba::rgb(255, 255, 0))
        );
        assert_eq!(c(ThresholdTarget::Value, 10.0), None);
        assert_eq!(c(ThresholdTarget::Panel, 95.0), Some(Rgba::rgb(0, 0, 255)));
        assert_eq!(c(ThresholdTarget::Graph, 95.0), None);
        assert_eq!(threshold_color(&t, ThresholdTarget::Value, None), None);
    }

    #[test]
    fn threshold_compares_value_before_rounding() {
        let t = [th(
            CompareOp::Gt,
            79.95,
            (255, 0, 0),
            ThresholdTarget::Value,
        )];
        // Shown as "80", but the comparison sees 79.96.
        assert_eq!(
            threshold_color(&t, ThresholdTarget::Value, Some(79.96)),
            Some(Rgba::rgb(255, 0, 0))
        );
    }

    #[test]
    fn fg_active_rule() {
        assert!(!fg_active(Some(1.2), false));
        assert!(fg_active(Some(1.21), false));
        assert!(fg_active(None, true));
        assert!(!fg_active(None, false));
    }

    #[test]
    fn visible_if_absent_source_is_false() {
        let cond = VisibleIf::Compare(Comparison {
            source: Source::Sensor("a/b/c".into()),
            stat: Stat::default(),
            op: CompareOp::Gt,
            value: 1.0,
        });
        assert!(is_visible(None, &|_, _| None, false));
        assert!(!is_visible(Some(&cond), &|_, _| None, false));
        assert!(is_visible(Some(&cond), &|_, _| Some(2.0), false));
        assert!(!is_visible(Some(&cond), &|_, _| Some(1.0), false));
    }

    #[test]
    fn visible_if_fg_active() {
        let cond = VisibleIf::Fg(FgCondition {
            fg: FgState::Active,
        });
        assert!(is_visible(Some(&cond), &|_, _| None, true));
        assert!(!is_visible(Some(&cond), &|_, _| None, false));
    }
}
