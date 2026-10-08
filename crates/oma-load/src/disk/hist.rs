//! Latency histogram: 8 buckets per power of 2 from 1 us to 2^40 us, exact mean (DC3).

const SUB: usize = 8;
const POWERS: usize = 40;
const BUCKETS: usize = POWERS * SUB;

#[derive(Debug, Clone)]
pub struct Histogram {
    buckets: Box<[u64; BUCKETS]>,
    count: u64,
    sum_us: u128,
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

impl Histogram {
    pub fn new() -> Self {
        Self {
            buckets: Box::new([0; BUCKETS]),
            count: 0,
            sum_us: 0,
        }
    }

    pub fn record(&mut self, us: u64) {
        self.buckets[bucket_of(us)] += 1;
        self.count += 1;
        self.sum_us += u128::from(us);
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    /// Exact mean in microseconds; 0 when empty.
    pub fn mean_us(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum_us as f64 / self.count as f64
        }
    }

    /// The middle of the bucket that holds the 99th percentile; 0 when empty.
    pub fn p99_us(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let target = (self.count * 99).div_ceil(100);
        let mut seen = 0;
        for (i, n) in self.buckets.iter().enumerate() {
            seen += n;
            if seen >= target {
                return bucket_mid(i);
            }
        }
        bucket_mid(BUCKETS - 1)
    }

    pub fn merge(&mut self, other: &Histogram) {
        for (a, b) in self.buckets.iter_mut().zip(other.buckets.iter()) {
            *a += b;
        }
        self.count += other.count;
        self.sum_us += other.sum_us;
    }
}

fn bucket_of(us: u64) -> usize {
    if us == 0 {
        return 0;
    }
    let e = 63 - us.leading_zeros() as usize;
    if e >= POWERS {
        return BUCKETS - 1;
    }
    let off = us - (1u64 << e);
    let sub = ((u128::from(off) * SUB as u128) >> e) as usize;
    e * SUB + sub
}

fn bucket_mid(i: usize) -> f64 {
    let (e, sub) = (i / SUB, i % SUB);
    let base = (1u64 << e) as f64;
    base * (1.0 + (sub as f64 + 0.5) / SUB as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_mean_is_exact_and_p99_is_within_a_bucket() {
        let mut h = Histogram::new();
        for us in 1..=1000u64 {
            h.record(us);
        }
        assert_eq!(h.count(), 1000);
        assert_eq!(h.mean_us(), 500.5);
        // True p99 is 990; a bucket is 12.5 % wide.
        let p = h.p99_us();
        assert!((p - 990.0).abs() <= 990.0 * 0.125, "p99 {p}");
        let mut other = Histogram::new();
        other.record(1_000_000);
        h.merge(&other);
        assert_eq!(h.count(), 1001);
        assert!((h.mean_us() - (500_500.0 + 1_000_000.0) / 1001.0).abs() < 1e-6);
        assert_eq!(Histogram::new().mean_us(), 0.0);
        assert_eq!(Histogram::new().p99_us(), 0.0);
    }

    #[test]
    fn huge_and_zero_values_do_not_panic() {
        let mut h = Histogram::new();
        h.record(0);
        h.record(u64::MAX);
        assert_eq!(h.count(), 2);
        assert!(h.p99_us() > 0.0);
    }
}
