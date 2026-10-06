//! Gauge full scale (DB6).

use super::score::SCALE_POINTS;

/// First number of the 1-2-2.5-5 x 10^n series that is >= 1.1 x max(values, 1500).
pub fn full_scale(values: &[f64]) -> f64 {
    let target = 1.1 * values.iter().copied().fold(SCALE_POINTS, f64::max);
    let mut decade = 1.0;
    loop {
        for step in [1.0, 2.0, 2.5, 5.0] {
            if step * decade >= target {
                return step * decade;
            }
        }
        decade *= 10.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_series() {
        assert_eq!(full_scale(&[1500.0]), 2000.0);
        assert_eq!(full_scale(&[1900.0]), 2500.0);
        assert_eq!(full_scale(&[4500.0]), 5000.0);
        assert_eq!(full_scale(&[4600.0]), 10_000.0);
        assert_eq!(full_scale(&[120.0]), 2000.0);
        assert_eq!(full_scale(&[]), 2000.0);
        assert_eq!(full_scale(&[50_000.0]), 100_000.0);
        assert_eq!(full_scale(&[120.0, 1900.0, 3.0]), 2500.0);
    }
}
