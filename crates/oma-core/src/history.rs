//! In-memory ring buffer of recent samples (spec §4.2).

use std::collections::{HashMap, VecDeque};

use serde::Serialize;

/// A slice of history aligned on shared timestamps.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryWindow {
    pub timestamps_ms: Vec<u64>,
    /// One series per requested id, same length as `timestamps_ms`.
    pub series: Vec<Vec<Option<f64>>>,
}

#[derive(Debug)]
pub struct History {
    capacity: usize,
    timestamps: VecDeque<u64>,
    /// NaN marks a missing value; it saves memory compared to `Option<f64>`.
    series: Vec<VecDeque<f64>>,
    index: HashMap<String, usize>,
}

impl History {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "history capacity must be positive");
        Self {
            capacity,
            timestamps: VecDeque::with_capacity(capacity),
            series: Vec::new(),
            index: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.timestamps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.timestamps.is_empty()
    }

    /// Replaces the sensor list. Series of ids that still exist are kept;
    /// new ids start with missing values for the samples already stored.
    pub fn set_sensors(&mut self, ids: &[String]) {
        let old_index = std::mem::take(&mut self.index);
        let mut previous: HashMap<String, VecDeque<f64>> = old_index
            .into_iter()
            .map(|(id, i)| (id, std::mem::take(&mut self.series[i])))
            .collect();
        let len = self.timestamps.len();
        self.series = ids
            .iter()
            .map(|id| {
                previous
                    .remove(id)
                    .unwrap_or_else(|| std::iter::repeat_n(f64::NAN, len).collect())
            })
            .collect();
        self.index = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
    }

    /// Appends one sample per sensor, in `set_sensors` order.
    pub fn push(&mut self, timestamp_ms: u64, values: &[Option<f64>]) {
        assert_eq!(
            values.len(),
            self.series.len(),
            "values must match the sensor list"
        );
        if self.timestamps.len() == self.capacity {
            self.timestamps.pop_front();
            for s in &mut self.series {
                s.pop_front();
            }
        }
        self.timestamps.push_back(timestamp_ms);
        for (s, v) in self.series.iter_mut().zip(values) {
            s.push_back(v.unwrap_or(f64::NAN));
        }
    }

    /// Samples taken at or after `since_ms`. The scan goes backwards from the
    /// newest sample and also stops where time runs backwards (wall clock set
    /// back), so the result is always in chronological order.
    pub fn window(&self, ids: &[String], since_ms: u64) -> HistoryWindow {
        let mut count = 0;
        let mut later = u64::MAX;
        for &t in self.timestamps.iter().rev() {
            if t < since_ms || t > later {
                break;
            }
            later = t;
            count += 1;
        }
        let start = self.timestamps.len() - count;
        let series = ids
            .iter()
            .map(|id| match self.index.get(id) {
                Some(&i) => self.series[i]
                    .range(start..)
                    .map(|&v| (!v.is_nan()).then_some(v))
                    .collect(),
                None => vec![None; count],
            })
            .collect();
        HistoryWindow {
            timestamps_ms: self.timestamps.range(start..).copied().collect(),
            series,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn evicts_oldest_sample_when_full() {
        let mut h = History::new(2);
        h.set_sensors(&ids(&["a"]));
        h.push(1, &[Some(1.0)]);
        h.push(2, &[Some(2.0)]);
        h.push(3, &[Some(3.0)]);
        let w = h.window(&ids(&["a"]), 0);
        assert_eq!(w.timestamps_ms, vec![2, 3]);
        assert_eq!(w.series, vec![vec![Some(2.0), Some(3.0)]]);
    }

    #[test]
    fn window_returns_only_recent_samples() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a"]));
        for t in [1_000, 2_000, 3_000] {
            h.push(t, &[Some(t as f64)]);
        }
        let w = h.window(&ids(&["a"]), 2_000);
        assert_eq!(w.timestamps_ms, vec![2_000, 3_000]);
        assert_eq!(w.series[0], vec![Some(2_000.0), Some(3_000.0)]);
    }

    #[test]
    fn missing_values_round_trip_as_none() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a"]));
        h.push(1, &[None]);
        assert_eq!(h.window(&ids(&["a"]), 0).series[0], vec![None]);
    }

    #[test]
    fn set_sensors_keeps_existing_series_and_pads_new_ones() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a"]));
        h.push(1, &[Some(1.0)]);
        h.set_sensors(&ids(&["b", "a"]));
        h.push(2, &[Some(20.0), Some(2.0)]);
        let w = h.window(&ids(&["a", "b"]), 0);
        assert_eq!(w.series[0], vec![Some(1.0), Some(2.0)]);
        assert_eq!(w.series[1], vec![None, Some(20.0)]);
    }

    #[test]
    fn unknown_ids_yield_empty_values() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a"]));
        h.push(1, &[Some(1.0)]);
        assert_eq!(h.window(&ids(&["nope"]), 0).series[0], vec![None]);
    }

    #[test]
    fn clock_jumping_backwards_keeps_only_samples_after_the_jump() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a"]));
        for t in [10_000, 20_000, 5_000, 6_000] {
            h.push(t, &[Some(1.0)]);
        }
        assert_eq!(h.window(&ids(&["a"]), 0).timestamps_ms, vec![5_000, 6_000]);
    }

    #[test]
    fn len_counts_samples() {
        let mut h = History::new(10);
        assert!(h.is_empty());
        h.set_sensors(&ids(&["a"]));
        h.push(1, &[Some(1.0)]);
        assert_eq!(h.len(), 1);
    }
}
