//! Share of the total CPU used by programs other than ours (M8a2), so the
//! CPU benchmark can flag a measurement taken while the machine was busy.

use std::io;

use crate::pdh::{Counter, Query};

/// Share of total CPU (0..=1) used by every process except `_Total`, `Idle`
/// and ours (`oma-*`, with or without a `#n` suffix). Each row is percent of
/// ONE logical CPU, so the total is divided by 100 x `logical`.
pub fn others_share(rows: &[(String, f64)], logical: u32) -> f64 {
    let total: f64 = rows
        .iter()
        .filter(|(name, value)| {
            let base = name.split('#').next().unwrap_or(name).to_ascii_lowercase();
            value.is_finite() && base != "_total" && base != "idle" && !base.starts_with("oma-")
        })
        .map(|(_, value)| value)
        .sum();
    (total / (100.0 * f64::from(logical.max(1)))).clamp(0.0, 1.0)
}

pub struct OtherCpu {
    query: Query,
    counter: Counter,
}

impl OtherCpu {
    pub fn open() -> io::Result<Self> {
        let mut query = Query::open().map_err(io::Error::other)?;
        let counter = query
            .add_english(r"\Process(*)\% Processor Time")
            .map_err(io::Error::other)?;
        Ok(Self { query, counter })
    }

    /// `None` until two collections exist, or when PDH has no data.
    pub fn sample(&mut self, logical: u32) -> Option<f64> {
        self.query.collect().ok()?;
        let rows = self.query.array(self.counter).ok()?;
        (!rows.is_empty()).then(|| others_share(&rows, logical))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(items: &[(&str, f64)]) -> Vec<(String, f64)> {
        items.iter().map(|(n, v)| ((*n).to_owned(), *v)).collect()
    }

    #[test]
    fn others_share_excludes_idle_total_and_our_processes() {
        let r = rows(&[
            ("_Total", 400.0),
            ("Idle", 300.0),
            ("oma-load", 800.0),
            ("OMA-App", 50.0),
            ("game", 50.0),
            ("chrome", 30.0),
        ]);
        // (50 + 30) / (100 * 8)
        assert!((others_share(&r, 8) - 0.1).abs() < 1e-9);
    }

    #[test]
    fn others_share_with_instance_suffixes() {
        let r = rows(&[
            ("oma-load#1", 700.0),
            ("oma-overlay#12", 10.0),
            ("chrome#1", 20.0),
            ("chrome#2", 20.0),
            ("bad", f64::NAN),
        ]);
        assert!((others_share(&r, 4) - 0.1).abs() < 1e-9);
        // Clamped to 1, and a zero core count never divides by zero.
        assert_eq!(others_share(&rows(&[("x", 5000.0)]), 4), 1.0);
        assert_eq!(others_share(&rows(&[("x", 5.0)]), 0), 0.05);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn other_cpu_reads_this_machine() {
        let mut cpu = OtherCpu::open().unwrap();
        assert_eq!(cpu.sample(8), None, "first sample has no rate yet");
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let share = cpu.sample(8).expect("second sample");
        println!("other CPU share: {share:.4}");
        assert!((0.0..=1.0).contains(&share));
    }
}
