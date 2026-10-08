//! The SLC cache cliff of an SSD under N2's sequential writes (DC8), found from the write
//! speed sampled every 500 ms. Pure: the engine feeds it samples.

/// The samples (milliseconds into the phase) whose median is the base speed.
const BASE_FROM_MS: u64 = 2_000;
const BASE_TO_MS: u64 = 5_000;
/// A sample under this part of the base is low.
const LOW_RATIO: f64 = 0.6;
/// Consecutive low samples (3 s at 2 Hz) that make a cliff.
const LOW_RUN: u32 = 6;

#[derive(Default)]
pub struct SlcDetector {
    samples: Vec<f64>,
    base_samples: Vec<f64>,
    base: Option<f64>,
    /// The bytes written at the previous sample.
    prev_written: u64,
    low_run: u32,
    /// The bytes written when the current run of low samples began.
    run_start: u64,
    cliff: bool,
}

fn median(values: &[f64]) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    match v.len() {
        0 => 0.0,
        n if n % 2 == 1 => v[n / 2],
        n => (v[n / 2 - 1] + v[n / 2]) / 2.0,
    }
}

impl SlcDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// One sample: the write speed over the last 500 ms, the bytes written so far and the
    /// milliseconds since the phase began. Gives the bytes written at the start of the
    /// cliff, once, when the sixth low sample in a row arrives.
    pub fn sample(&mut self, bps: f64, written: u64, t_ms: u64) -> Option<u64> {
        self.samples.push(bps);
        let before = std::mem::replace(&mut self.prev_written, written);
        if self.cliff {
            return None;
        }
        if t_ms <= BASE_TO_MS {
            if t_ms >= BASE_FROM_MS {
                self.base_samples.push(bps);
            }
            return None;
        }
        if self.base.is_none() && !self.base_samples.is_empty() {
            self.base = Some(median(&self.base_samples));
        }
        let base = self.base.filter(|&b| b > 0.0)?;
        if bps >= LOW_RATIO * base {
            self.low_run = 0;
            return None;
        }
        if self.low_run == 0 {
            self.run_start = before;
        }
        self.low_run += 1;
        if self.low_run < LOW_RUN {
            return None;
        }
        self.cliff = true;
        Some(self.run_start)
    }

    /// The speed after the cliff: the median of the last fifth of the samples. `None`
    /// without a cliff.
    pub fn steady(&self) -> Option<f64> {
        if !self.cliff {
            return None;
        }
        let n = (self.samples.len() / 5).max(1);
        Some(median(&self.samples[self.samples.len() - n..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: f64 = 1e6;

    /// Feeds `speeds` (MB/s, one per 500 ms) and gives the cliff (if any) with the detector.
    fn feed(speeds: &[f64]) -> (Option<u64>, SlcDetector) {
        let mut d = SlcDetector::new();
        let (mut written, mut cliff) = (0u64, None);
        for (i, &s) in speeds.iter().enumerate() {
            written += (s * MB * 0.5) as u64;
            let t = (i as u64 + 1) * 500;
            if let Some(at) = d.sample(s * MB, written, t) {
                assert!(cliff.is_none(), "reported twice");
                cliff = Some(at);
            }
        }
        (cliff, d)
    }

    fn repeat(speed: f64, seconds: u32) -> Vec<f64> {
        vec![speed; (seconds * 2) as usize]
    }

    #[test]
    fn flat_speed_has_no_cliff() {
        let (cliff, d) = feed(&repeat(3000.0, 60));
        assert_eq!(cliff, None);
        assert_eq!(d.steady(), None);
    }

    #[test]
    fn single_drop_is_a_cliff_at_its_first_sample() {
        let mut v = repeat(3000.0, 20);
        v.extend(repeat(800.0, 20));
        let (cliff, d) = feed(&v);
        let at = cliff.expect("a cliff");
        let expected = (3000.0 * MB * 20.0) as u64;
        assert!(at.abs_diff(expected) <= (3000.0 * MB * 0.5) as u64, "{at}");
        assert_eq!(d.steady(), Some(800.0 * MB));
    }

    #[test]
    fn short_dip_is_not_a_cliff() {
        let mut v = repeat(3000.0, 20);
        v.extend(repeat(800.0, 2));
        v.extend(repeat(3000.0, 20));
        assert_eq!(feed(&v).0, None);
    }

    #[test]
    fn steady_is_the_median_of_the_last_fifth() {
        let mut v = repeat(3000.0, 20);
        v.extend(repeat(800.0, 20));
        // 100 samples: the last 20 are ten of 700 and ten of 900.
        v.extend(vec![700.0; 10]);
        v.extend(vec![900.0; 10]);
        assert_eq!(v.len(), 100);
        let (cliff, d) = feed(&v);
        assert!(cliff.is_some());
        assert_eq!(d.steady(), Some(800.0 * MB));
    }
}
