//! Converts cumulative counters (bytes, energy...) into per-second rates.

#[derive(Debug, Clone, Default)]
pub struct CounterRate {
    last: Option<(u64, u64)>,
}

impl CounterRate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a cumulative `value` sampled at monotonic time `t_ms` and returns
    /// units per second. Returns `None` for the first sample, when no time has
    /// elapsed, or when the counter went backwards (device reset): the new
    /// value becomes the baseline.
    pub fn update(&mut self, value: u64, t_ms: u64) -> Option<f64> {
        let (prev_value, prev_t) = self.last.replace((value, t_ms))?;
        if t_ms <= prev_t || value < prev_value {
            return None;
        }
        Some((value - prev_value) as f64 * 1000.0 / (t_ms - prev_t) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_has_no_rate() {
        let mut r = CounterRate::new();
        assert_eq!(r.update(1_000, 0), None);
    }

    #[test]
    fn rate_is_per_second() {
        let mut r = CounterRate::new();
        r.update(1_000, 0);
        assert_eq!(r.update(2_000, 500), Some(2_000.0));
    }

    #[test]
    fn zero_elapsed_time_yields_none() {
        let mut r = CounterRate::new();
        r.update(1_000, 100);
        assert_eq!(r.update(1_500, 100), None);
    }

    #[test]
    fn counter_reset_restarts_from_new_baseline() {
        let mut r = CounterRate::new();
        r.update(10_000, 0);
        assert_eq!(r.update(50, 1_000), None);
        assert_eq!(r.update(1_050, 2_000), Some(1_000.0));
    }
}
