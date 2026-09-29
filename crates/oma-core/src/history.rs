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
    /// A value count that does not match the sensor list is logged once.
    mismatch_logged: bool,
}

impl History {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "history capacity must be positive");
        Self {
            capacity,
            timestamps: VecDeque::with_capacity(capacity),
            series: Vec::new(),
            index: HashMap::new(),
            mismatch_logged: false,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Changes how many samples are kept. A smaller capacity drops the oldest
    /// samples (timestamps and every series together); a larger one keeps
    /// them all and simply leaves room for more.
    pub fn set_capacity(&mut self, capacity: usize) {
        assert!(capacity > 0, "history capacity must be positive");
        self.capacity = capacity;
        let excess = self.timestamps.len().saturating_sub(capacity);
        if excess > 0 {
            self.timestamps.drain(..excess);
            for s in &mut self.series {
                s.drain(..excess);
            }
        }
        // Give the memory of dropped samples back; growing needs no reserve,
        // `push` extends the buffers as samples arrive.
        self.timestamps.shrink_to(capacity);
        for s in &mut self.series {
            s.shrink_to(capacity);
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

    /// Appends one sample per sensor, in `set_sensors` order. A value count
    /// that does not match the sensor list is a bug upstream, but it must not
    /// stop sampling: missing values are stored as missing, extra ones are
    /// dropped, and the mismatch is logged once.
    pub fn push(&mut self, timestamp_ms: u64, values: &[Option<f64>]) {
        if values.len() != self.series.len() && !self.mismatch_logged {
            self.mismatch_logged = true;
            tracing::error!(
                expected = self.series.len(),
                got = values.len(),
                "history values do not match the sensor list; padding or truncating"
            );
        }
        if self.timestamps.len() == self.capacity {
            self.timestamps.pop_front();
            for s in &mut self.series {
                s.pop_front();
            }
        }
        self.timestamps.push_back(timestamp_ms);
        for (i, s) in self.series.iter_mut().enumerate() {
            s.push_back(values.get(i).copied().flatten().unwrap_or(f64::NAN));
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

    /// Like `window`, but at most `max_points` rows: long windows are reduced
    /// to a min/max envelope so a 1 h chart stays light (decision D3).
    /// Samples are split into `max_points / 2` consecutive balanced buckets
    /// whose sizes differ by at most one sample; each bucket yields two
    /// rows, (first timestamp, per-series minimum) and (last timestamp,
    /// per-series maximum). These are envelope bounds, not actual extremum
    /// times. Any missing value makes that series yield `None` twice for the
    /// bucket; peaks survive in fully valid buckets. With `max_points < 2`, or when
    /// the samples already fit, the result is exactly `window`.
    pub fn window_decimated(
        &self,
        ids: &[String],
        since_ms: u64,
        max_points: usize,
    ) -> HistoryWindow {
        decimate(self.window(ids, since_ms), max_points)
    }
}

fn decimate(raw: HistoryWindow, max_points: usize) -> HistoryWindow {
    let len = raw.timestamps_ms.len();
    if max_points < 2 || len <= max_points {
        return raw;
    }
    let buckets = max_points / 2;
    let bounds = |b: usize| (b * len / buckets, (b + 1) * len / buckets);
    let mut timestamps_ms = Vec::with_capacity(buckets * 2);
    for b in 0..buckets {
        let (start, end) = bounds(b);
        timestamps_ms.push(raw.timestamps_ms[start]);
        timestamps_ms.push(raw.timestamps_ms[end - 1]);
    }
    let series = raw
        .series
        .iter()
        .map(|values| {
            let mut out = Vec::with_capacity(buckets * 2);
            for b in 0..buckets {
                let (start, end) = bounds(b);
                if values[start..end].iter().any(Option::is_none) {
                    out.extend([None, None]);
                    continue;
                }
                let mut min: Option<f64> = None;
                let mut max: Option<f64> = None;
                for &v in values[start..end].iter().flatten() {
                    min = Some(min.map_or(v, |m| m.min(v)));
                    max = Some(max.map_or(v, |m| m.max(v)));
                }
                out.push(min);
                out.push(max);
            }
            out
        })
        .collect();
    HistoryWindow {
        timestamps_ms,
        series,
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
    fn shrinking_capacity_keeps_newest() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a", "b"]));
        for i in 1..=10u64 {
            h.push(i * 1_000, &[Some(i as f64), Some(-(i as f64))]);
        }
        h.set_capacity(4);
        assert_eq!(h.len(), 4);
        let w = h.window(&ids(&["a", "b"]), 0);
        assert_eq!(w.timestamps_ms, vec![7_000, 8_000, 9_000, 10_000]);
        assert_eq!(
            w.series[0],
            vec![Some(7.0), Some(8.0), Some(9.0), Some(10.0)]
        );
        assert_eq!(
            w.series[1],
            vec![Some(-7.0), Some(-8.0), Some(-9.0), Some(-10.0)]
        );
        // The ring keeps rolling at the new size.
        h.push(11_000, &[Some(11.0), Some(-11.0)]);
        assert_eq!(h.len(), 4);
        assert_eq!(
            h.window(&ids(&["a"]), 0).timestamps_ms,
            vec![8_000, 9_000, 10_000, 11_000]
        );
    }

    #[test]
    fn growing_capacity_keeps_everything() {
        let mut h = History::new(4);
        h.set_sensors(&ids(&["a"]));
        for i in 1..=4u64 {
            h.push(i, &[Some(i as f64)]);
        }
        h.set_capacity(8);
        assert_eq!(h.len(), 4);
        for i in 5..=8u64 {
            h.push(i, &[Some(i as f64)]);
        }
        let w = h.window(&ids(&["a"]), 0);
        assert_eq!(w.timestamps_ms, (1..=8).collect::<Vec<u64>>());
        assert_eq!(
            w.series[0],
            (1..=8).map(|i| Some(i as f64)).collect::<Vec<_>>()
        );
        h.push(9, &[Some(9.0)]);
        assert_eq!(h.len(), 8);
        assert_eq!(h.window(&ids(&["a"]), 0).timestamps_ms[0], 2);
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
    fn push_with_too_few_values_stores_the_rest_as_missing() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a", "b"]));
        h.push(1, &[Some(1.0)]);
        let w = h.window(&ids(&["a", "b"]), 0);
        assert_eq!(w.series, vec![vec![Some(1.0)], vec![None]]);
    }

    #[test]
    fn push_with_too_many_values_drops_the_extra_ones() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a"]));
        h.push(1, &[Some(1.0), Some(2.0)]);
        h.push(2, &[Some(3.0)]);
        let w = h.window(&ids(&["a"]), 0);
        assert_eq!(w.timestamps_ms, vec![1, 2]);
        assert_eq!(w.series, vec![vec![Some(1.0), Some(3.0)]]);
    }

    fn filled(values: &[Option<f64>]) -> History {
        let mut h = History::new(100);
        h.set_sensors(&ids(&["a"]));
        for (i, v) in values.iter().enumerate() {
            h.push((i as u64 + 1) * 1_000, &[*v]);
        }
        h
    }

    #[test]
    fn decimation_is_the_raw_window_when_samples_fit() {
        let h = filled(&[Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
        let a = ids(&["a"]);
        assert_eq!(h.window_decimated(&a, 0, 4), h.window(&a, 0));
        assert_eq!(h.window_decimated(&a, 0, 1), h.window(&a, 0));
        assert_eq!(h.window_decimated(&a, 0, 0), h.window(&a, 0));
    }

    #[test]
    fn decimation_emits_min_then_max_per_bucket() {
        let v = [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0, 5.0, 3.0].map(Some);
        let w = filled(&v).window_decimated(&ids(&["a"]), 0, 4);
        // Two buckets of five samples: [3 1 4 1 5] and [9 2 6 5 3].
        assert_eq!(w.timestamps_ms, vec![1_000, 5_000, 6_000, 10_000]);
        assert_eq!(
            w.series,
            vec![vec![Some(1.0), Some(5.0), Some(2.0), Some(9.0)]]
        );
    }

    #[test]
    fn balanced_buckets_distribute_the_remainder() {
        let v: Vec<Option<f64>> = (1..=11).map(|i| Some(i as f64)).collect();
        let w = filled(&v).window_decimated(&ids(&["a"]), 0, 5);
        // 5 / 2 = 2 balanced buckets: five samples, then six.
        assert_eq!(w.timestamps_ms, vec![1_000, 5_000, 6_000, 11_000]);
        assert_eq!(
            w.series,
            vec![vec![Some(1.0), Some(5.0), Some(6.0), Some(11.0)]]
        );
    }

    #[test]
    fn a_bucket_without_values_emits_none_twice() {
        let v = [None, None, None, Some(2.0), Some(4.0), Some(7.0)];
        let w = filled(&v).window_decimated(&ids(&["a", "unknown"]), 0, 4);
        assert_eq!(w.timestamps_ms, vec![1_000, 3_000, 4_000, 6_000]);
        assert_eq!(w.series[0], vec![None, None, Some(2.0), Some(7.0)]);
        assert_eq!(w.series[1], vec![None; 4]);
    }

    #[test]
    fn a_mixed_bucket_preserves_the_gap_conservatively() {
        let v = [Some(1.0), None, Some(3.0), Some(4.0), Some(5.0), Some(6.0)];
        let w = filled(&v).window_decimated(&ids(&["a"]), 0, 4);
        assert_eq!(w.series[0], vec![None, None, Some(4.0), Some(6.0)]);
    }

    #[test]
    fn near_one_hour_has_no_oversized_final_bucket() {
        let mut h = History::new(3_600);
        h.set_sensors(&ids(&["a"]));
        for i in 1..=3_599 {
            h.push(i * 1_000, &[Some(i as f64)]);
        }
        let w = h.window_decimated(&ids(&["a"]), 0, 900);
        assert_eq!(w.timestamps_ms.len(), 900);
        assert_eq!(w.timestamps_ms.first(), Some(&1_000));
        assert_eq!(w.timestamps_ms.last(), Some(&3_599_000));
        for pair in w.timestamps_ms.chunks_exact(2) {
            assert!((6_000..=7_000).contains(&(pair[1] - pair[0])));
        }
    }

    #[test]
    fn decimation_applies_after_the_since_filter() {
        let v: Vec<Option<f64>> = (1..=10).map(|i| Some(i as f64)).collect();
        let w = filled(&v).window_decimated(&ids(&["a"]), 5_000, 2);
        assert_eq!(w.timestamps_ms, vec![5_000, 10_000]);
        assert_eq!(w.series, vec![vec![Some(5.0), Some(10.0)]]);
    }

    #[test]
    fn decimated_output_never_exceeds_max_points() {
        for n in 0..40 {
            let v: Vec<Option<f64>> = (0..n).map(|i| Some(i as f64)).collect();
            let h = filled(&v);
            for max_points in 2..45 {
                let w = h.window_decimated(&ids(&["a"]), 0, max_points);
                assert!(
                    w.timestamps_ms.len() <= max_points,
                    "n={n} max={max_points}"
                );
                assert_eq!(w.series[0].len(), w.timestamps_ms.len());
                assert!(w.timestamps_ms.windows(2).all(|p| p[0] <= p[1]));
            }
        }
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
