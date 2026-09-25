//! Running min/max/average per sensor since the app started (spec §4.2).
//!
//! They live in the core, not in the UI: the WebView is destroyed when the
//! window closes (spec §2.2), while the statistics must cover the whole
//! session, tray time included.

use std::collections::HashMap;

use serde::Serialize;

/// Statistics of one sensor over the samples seen since start or last reset.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorStats {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    /// Number of valid samples behind `avg`.
    pub count: u64,
}

#[derive(Debug, Clone, Copy)]
struct Acc {
    min: f64,
    max: f64,
    sum: f64,
    count: u64,
}

impl Acc {
    const EMPTY: Self = Self {
        min: f64::INFINITY,
        max: f64::NEG_INFINITY,
        sum: 0.0,
        count: 0,
    };

    fn add(&mut self, value: f64) {
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.sum += value;
        self.count += 1;
    }

    fn stats(&self) -> Option<SensorStats> {
        (self.count > 0).then(|| SensorStats {
            min: self.min,
            max: self.max,
            avg: self.sum / self.count as f64,
            count: self.count,
        })
    }
}

/// One accumulator per sensor, in the order given to `set_sensors`.
#[derive(Debug, Default)]
pub struct Stats {
    index: HashMap<String, usize>,
    acc: Vec<Acc>,
}

impl Stats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the sensor list, like `History::set_sensors`: accumulators of
    /// ids that are still present are kept, new ids start empty.
    pub fn set_sensors(&mut self, ids: &[String]) {
        let previous: HashMap<String, Acc> = std::mem::take(&mut self.index)
            .into_iter()
            .map(|(id, i)| (id, self.acc[i]))
            .collect();
        self.acc = ids
            .iter()
            .map(|id| previous.get(id).copied().unwrap_or(Acc::EMPTY))
            .collect();
        self.index = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
    }

    /// Adds one sample per sensor, in `set_sensors` order. Missing and
    /// non-finite values are skipped; values beyond the sensor list are
    /// ignored and sensors without a value simply get no sample.
    pub fn push(&mut self, values: &[Option<f64>]) {
        for (acc, value) in self.acc.iter_mut().zip(values) {
            if let Some(v) = value.filter(|v| v.is_finite()) {
                acc.add(v);
            }
        }
    }

    /// Statistics per requested id; `None` for an unknown id or a sensor
    /// without valid samples yet.
    pub fn get(&self, ids: &[String]) -> Vec<Option<SensorStats>> {
        ids.iter()
            .map(|id| self.index.get(id).and_then(|&i| self.acc[i].stats()))
            .collect()
    }

    /// Restarts the statistics of the given sensors; unknown ids are ignored.
    pub fn reset(&mut self, ids: &[String]) {
        for id in ids {
            if let Some(&i) = self.index.get(id) {
                self.acc[i] = Acc::EMPTY;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn stats(min: f64, max: f64, avg: f64, count: u64) -> Option<SensorStats> {
        Some(SensorStats {
            min,
            max,
            avg,
            count,
        })
    }

    #[test]
    fn tracks_min_max_avg_and_count() {
        let mut s = Stats::new();
        s.set_sensors(&ids(&["a", "b"]));
        s.push(&[Some(10.0), Some(-1.0)]);
        s.push(&[Some(30.0), Some(-3.0)]);
        s.push(&[Some(20.0), Some(-2.0)]);
        assert_eq!(
            s.get(&ids(&["a", "b"])),
            vec![stats(10.0, 30.0, 20.0, 3), stats(-3.0, -1.0, -2.0, 3)]
        );
    }

    #[test]
    fn unknown_ids_and_sensors_without_samples_have_no_stats() {
        let mut s = Stats::new();
        assert_eq!(s.get(&ids(&["a"])), vec![None]);
        s.set_sensors(&ids(&["a"]));
        assert_eq!(s.get(&ids(&["a", "nope"])), vec![None, None]);
    }

    #[test]
    fn missing_and_non_finite_values_are_skipped() {
        let mut s = Stats::new();
        s.set_sensors(&ids(&["a"]));
        for v in [None, Some(f64::NAN), Some(4.0), Some(f64::INFINITY)] {
            s.push(&[v]);
        }
        s.push(&[Some(f64::NEG_INFINITY)]);
        assert_eq!(s.get(&ids(&["a"])), vec![stats(4.0, 4.0, 4.0, 1)]);
    }

    #[test]
    fn set_sensors_keeps_persisting_ids_and_starts_new_ones_empty() {
        let mut s = Stats::new();
        s.set_sensors(&ids(&["a", "gone"]));
        s.push(&[Some(1.0), Some(5.0)]);
        s.set_sensors(&ids(&["new", "a"]));
        s.push(&[Some(7.0), Some(3.0)]);
        assert_eq!(
            s.get(&ids(&["a", "new", "gone"])),
            vec![stats(1.0, 3.0, 2.0, 2), stats(7.0, 7.0, 7.0, 1), None]
        );
    }

    #[test]
    fn a_removed_id_that_comes_back_starts_empty() {
        let mut s = Stats::new();
        s.set_sensors(&ids(&["a"]));
        s.push(&[Some(1.0)]);
        s.set_sensors(&[]);
        s.set_sensors(&ids(&["a"]));
        assert_eq!(s.get(&ids(&["a"])), vec![None]);
    }

    #[test]
    fn reset_clears_only_the_named_sensors() {
        let mut s = Stats::new();
        s.set_sensors(&ids(&["a", "b"]));
        s.push(&[Some(1.0), Some(2.0)]);
        s.reset(&ids(&["a", "unknown"]));
        assert_eq!(
            s.get(&ids(&["a", "b"])),
            vec![None, stats(2.0, 2.0, 2.0, 1)]
        );
        s.push(&[Some(9.0), None]);
        assert_eq!(s.get(&ids(&["a"])), vec![stats(9.0, 9.0, 9.0, 1)]);
    }

    #[test]
    fn a_value_count_mismatch_never_panics() {
        let mut s = Stats::new();
        s.set_sensors(&ids(&["a", "b"]));
        s.push(&[Some(1.0)]);
        s.push(&[Some(2.0), Some(3.0), Some(99.0)]);
        s.push(&[]);
        assert_eq!(
            s.get(&ids(&["a", "b"])),
            vec![stats(1.0, 2.0, 1.5, 2), stats(3.0, 3.0, 3.0, 1)]
        );
    }

    #[test]
    fn serializes_with_the_ts_contract_keys() {
        let value = serde_json::to_value(SensorStats {
            min: 1.0,
            max: 3.0,
            avg: 2.0,
            count: 2,
        })
        .expect("serialize");
        assert_eq!(
            value,
            serde_json::json!({ "min": 1.0, "max": 3.0, "avg": 2.0, "count": 2 })
        );
    }
}
