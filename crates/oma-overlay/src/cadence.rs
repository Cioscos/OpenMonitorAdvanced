//! When the overlay presents (spec §5.2): texts at most at `textHz`, charts
//! at most at `chartFps`, and nothing at all without changes.

use crate::state::Changes;

/// What a frame must redraw.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Due {
    pub text: bool,
    pub charts: bool,
}

impl Due {
    pub fn any(&self) -> bool {
        self.text || self.charts
    }
}

/// Throttles the redraws. Changes that are not due yet are remembered and
/// served at the next slot.
#[derive(Debug, Clone)]
pub struct Cadence {
    text_period_s: f64,
    chart_period_s: f64,
    last_text_s: Option<f64>,
    last_charts_s: Option<f64>,
    text_pending: bool,
    charts_pending: bool,
}

/// Tolerance on a slot, so a period that does not divide exactly in binary
/// (1/30 s, 1/60 s) is still due at its nominal time.
const SLOT_EPSILON_S: f64 = 1e-9;

fn slot_reached(last: Option<f64>, period: f64, now_s: f64) -> bool {
    last.is_none_or(|t| now_s - t >= period - SLOT_EPSILON_S)
}

fn period(hz: u32) -> f64 {
    1.0 / f64::from(hz.max(1))
}

impl Cadence {
    pub fn new(chart_fps: u32, text_hz: u32) -> Self {
        Self {
            text_period_s: period(text_hz),
            chart_period_s: period(chart_fps),
            last_text_s: None,
            last_charts_s: None,
            text_pending: false,
            charts_pending: false,
        }
    }

    /// Changes the rates (new drawing settings); pending changes stay.
    pub fn set_rates(&mut self, chart_fps: u32, text_hz: u32) {
        self.text_period_s = period(text_hz);
        self.chart_period_s = period(chart_fps);
    }

    /// Takes the changes since the last call and says what to redraw at
    /// `now_s`. A new layout, new settings or a new placement redraw
    /// everything at once.
    pub fn due(&mut self, now_s: f64, pending: &Changes) -> Due {
        let all = pending.layout || pending.settings || pending.placement;
        self.text_pending |= pending.text || all;
        self.charts_pending |= pending.charts || all;
        let text =
            self.text_pending && (all || slot_reached(self.last_text_s, self.text_period_s, now_s));
        let charts = self.charts_pending
            && (all || slot_reached(self.last_charts_s, self.chart_period_s, now_s));
        if text {
            self.text_pending = false;
            self.last_text_s = Some(now_s);
        }
        if charts {
            self.charts_pending = false;
            self.last_charts_s = Some(now_s);
        }
        Due { text, charts }
    }

    /// Seconds from `now_s` until a pending change becomes due; `None` when
    /// nothing is pending.
    pub fn next_wake(&self, now_s: f64) -> Option<f64> {
        let slot = |pending: bool, last: Option<f64>, period: f64| {
            pending.then(|| last.map_or(now_s, |t| t + period))
        };
        let text = slot(self.text_pending, self.last_text_s, self.text_period_s);
        let charts = slot(self.charts_pending, self.last_charts_s, self.chart_period_s);
        let at = match (text, charts) {
            (Some(a), Some(b)) => a.min(b),
            (a, b) => a.or(b)?,
        };
        Some((at - now_s).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text() -> Changes {
        Changes {
            text: true,
            ..Changes::default()
        }
    }

    fn charts() -> Changes {
        Changes {
            charts: true,
            ..Changes::default()
        }
    }

    const NONE: Changes = Changes {
        layout: false,
        text: false,
        charts: false,
        placement: false,
        settings: false,
    };

    #[test]
    fn text_due_at_text_hz() {
        let mut c = Cadence::new(30, 2);
        assert_eq!(
            c.due(0.0, &text()),
            Due {
                text: true,
                charts: false
            }
        );
        // Within the 0.5 s period the change waits.
        assert!(!c.due(0.2, &text()).text);
        assert!(!c.due(0.49, &NONE).text);
        // At the slot the deferred change is served, without a new one.
        assert!(c.due(0.5, &NONE).text);
        assert!(!c.due(0.6, &NONE).text);
        // 4 Hz after a rate change.
        c.set_rates(30, 4);
        assert!(!c.due(0.7, &text()).text);
        assert!(c.due(0.75, &NONE).text);
    }

    #[test]
    fn charts_due_at_chart_fps() {
        let mut c = Cadence::new(30, 2);
        assert!(c.due(0.0, &charts()).charts);
        assert!(!c.due(0.01, &charts()).charts);
        let d = c.due(1.0 / 30.0, &NONE);
        assert_eq!(
            d,
            Due {
                text: false,
                charts: true
            }
        );
        // 60 FPS halves the period.
        c.set_rates(60, 2);
        let t = 1.0 / 30.0;
        assert!(!c.due(t + 0.01, &charts()).charts);
        assert!(c.due(t + 1.0 / 60.0, &NONE).charts);
        // A new layout redraws everything at once, slot or not.
        let layout = Changes {
            layout: true,
            ..Changes::default()
        };
        assert_eq!(
            c.due(t + 1.0 / 60.0 + 0.001, &layout),
            Due {
                text: true,
                charts: true
            }
        );
    }

    #[test]
    fn nothing_due_without_changes() {
        let mut c = Cadence::new(60, 4);
        for i in 0..100 {
            assert!(!c.due(f64::from(i) * 0.1, &NONE).any());
        }
        assert_eq!(c.next_wake(10.0), None);
        // A served change leaves nothing pending.
        assert!(c.due(20.0, &text()).text);
        assert!(!c.due(30.0, &NONE).any());
        assert_eq!(c.next_wake(30.0), None);
    }

    #[test]
    fn next_wake_is_the_earliest_due() {
        let mut c = Cadence::new(30, 2);
        c.due(0.0, &text());
        c.due(0.0, &charts());
        // Both pending again: charts at 1/30 s, text at 0.5 s.
        c.due(
            0.01,
            &Changes {
                text: true,
                charts: true,
                ..Changes::default()
            },
        );
        let wake = c.next_wake(0.01).unwrap();
        assert!((wake - (1.0 / 30.0 - 0.01)).abs() < 1e-9, "{wake}");
        c.due(1.0 / 30.0, &NONE);
        let wake = c.next_wake(0.1).unwrap();
        assert!((wake - 0.4).abs() < 1e-9, "{wake}");
        // Past the slot: wake now.
        assert_eq!(c.next_wake(0.7), Some(0.0));
    }
}
