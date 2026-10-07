//! Share of the total CPU used by programs other than ours (M8a2), so the
//! CPU benchmark can flag a measurement taken while the machine was busy.

use std::collections::HashSet;
use std::io;

use crate::pdh::{Counter, Query};

/// One `\Process(*)` instance: its CPU (percent of ONE logical CPU) and, when
/// the `ID Process` array lines up with the CPU one, its PID.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: String,
    pub cpu: f64,
    pub pid: Option<u32>,
}

/// Pairs the CPU array with the `ID Process` array of the same collection. Both
/// list the instances in the same order; when the lengths or the names differ
/// no row gets a PID, and only the name rule of [`others_share`] applies.
pub fn with_pids(cpu: Vec<(String, f64)>, ids: &[(String, f64)]) -> Vec<Row> {
    let aligned = cpu.len() == ids.len() && cpu.iter().zip(ids).all(|(c, i)| c.0 == i.0);
    cpu.into_iter()
        .enumerate()
        .map(|(n, (name, cpu))| Row {
            pid: aligned
                .then(|| ids[n].1)
                .filter(|id| id.is_finite() && *id >= 0.0)
                .map(|id| id as u32),
            name,
            cpu,
        })
        .collect()
}

/// Share of total CPU (0..=1) used by every process except `_Total`, `Idle`,
/// ours by name (`oma-*`, with or without a `#n` suffix) and `ours` by PID (the
/// app's process tree, so its WebView2 is not "another program"). The total is
/// divided by 100 x `logical`.
pub fn others_share(rows: &[Row], ours: &HashSet<u32>, logical: u32) -> f64 {
    let total: f64 = rows
        .iter()
        .filter(|r| {
            let base = r
                .name
                .split('#')
                .next()
                .unwrap_or(&r.name)
                .to_ascii_lowercase();
            r.cpu.is_finite()
                && base != "_total"
                && base != "idle"
                && !base.starts_with("oma-")
                && !r.pid.is_some_and(|pid| ours.contains(&pid))
        })
        .map(|r| r.cpu)
        .sum();
    (total / (100.0 * f64::from(logical.max(1)))).clamp(0.0, 1.0)
}

pub struct OtherCpu {
    query: Query,
    cpu: Counter,
    pid: Counter,
}

impl OtherCpu {
    pub fn open() -> io::Result<Self> {
        let mut query = Query::open().map_err(io::Error::other)?;
        let cpu = query
            .add_english(r"\Process(*)\% Processor Time")
            .map_err(io::Error::other)?;
        let pid = query
            .add_english(r"\Process(*)\ID Process")
            .map_err(io::Error::other)?;
        Ok(Self { query, cpu, pid })
    }

    /// `None` until two collections exist, or when PDH has no data. `ours` is
    /// left out by PID; an unreadable PID array leaves only the name rule.
    pub fn sample(&mut self, logical: u32, ours: &HashSet<u32>) -> Option<f64> {
        self.query.collect().ok()?;
        let cpu = self.query.array(self.cpu).ok()?;
        if cpu.is_empty() {
            return None;
        }
        let ids = self.query.array(self.pid).unwrap_or_default();
        Some(others_share(&with_pids(cpu, &ids), ours, logical))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(items: &[(&str, f64)]) -> Vec<(String, f64)> {
        items.iter().map(|(n, v)| ((*n).to_owned(), *v)).collect()
    }

    fn no_pids(cpu: Vec<(String, f64)>) -> Vec<Row> {
        with_pids(cpu, &[])
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
        assert!((others_share(&no_pids(r), &HashSet::new(), 8) - 0.1).abs() < 1e-9);
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
        let none = HashSet::new();
        assert!((others_share(&no_pids(r), &none, 4) - 0.1).abs() < 1e-9);
        // Clamped to 1, and a zero core count never divides by zero.
        assert_eq!(
            others_share(&no_pids(rows(&[("x", 5000.0)])), &none, 4),
            1.0
        );
        assert_eq!(others_share(&no_pids(rows(&[("x", 5.0)])), &none, 0), 0.05);
    }

    #[test]
    fn others_share_leaves_out_our_process_tree_by_pid() {
        // Our WebView2 shares its name with other apps' WebView2: only the PID tells them apart.
        let cpu = rows(&[
            ("msedgewebview2", 80.0),
            ("msedgewebview2", 40.0),
            ("game", 40.0),
        ]);
        let ids = rows(&[
            ("msedgewebview2", 101.0),
            ("msedgewebview2", 202.0),
            ("game", 303.0),
        ]);
        let ours = HashSet::from([101]);
        let r = with_pids(cpu, &ids);
        // (40 + 40) / (100 * 8)
        assert!((others_share(&r, &ours, 8) - 0.1).abs() < 1e-9);
    }

    #[test]
    fn with_pids_gives_no_pid_when_the_two_arrays_do_not_line_up() {
        let cpu = rows(&[("a", 10.0), ("b", 10.0)]);
        let r = with_pids(cpu.clone(), &rows(&[("a", 1.0)]));
        assert!(r.iter().all(|r| r.pid.is_none()));
        let r = with_pids(cpu.clone(), &rows(&[("a", 1.0), ("c", 2.0)]));
        assert!(r.iter().all(|r| r.pid.is_none()));
        let r = with_pids(cpu, &rows(&[("a", 1.0), ("b", f64::NAN)]));
        assert_eq!(r.iter().map(|r| r.pid).collect::<Vec<_>>(), [Some(1), None]);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn other_cpu_reads_this_machine() {
        let mut cpu = OtherCpu::open().unwrap();
        assert_eq!(
            cpu.sample(8, &HashSet::new()),
            None,
            "first sample has no rate yet"
        );
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let share = cpu.sample(8, &HashSet::new()).expect("second sample");
        println!("other CPU share: {share:.4}");
        // The two arrays of one collection line up, so every instance has its PID.
        let rows = with_pids(
            cpu.query.array(cpu.cpu).unwrap(),
            &cpu.query.array(cpu.pid).unwrap(),
        );
        assert!(rows.len() > 10 && rows.iter().all(|r| r.pid.is_some()));
        let mine = rows.iter().find(|r| r.pid == Some(std::process::id()));
        assert!(mine.is_some(), "this test process is in the array");
        assert!((0.0..=1.0).contains(&share));
    }
}
