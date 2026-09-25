//! Per-process GPU usage (decision D5), shared between the GPU provider, which publishes it
//! every tick, and the shell command `get_gpu_processes`, which reads it. Not sensors: no
//! ids, no history.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Rows returned per device, busiest first.
const MAX_ROWS: usize = 20;

/// Rows older than this are hidden (two and a half 1 s ticks): if the provider's worker
/// thread hangs (a vendor layer stuck in the driver, or a stuck PDH collect), it stops
/// calling `publish`, but without this check the last tick's rows would keep answering
/// `get_gpu_processes` with frozen, non-zero loads next to GPU sensors the engine has since
/// blanked after two missed 200 ms deadlines (spec §8).
const MAX_ROW_AGE: Duration = Duration::from_millis(2_500);

/// One process using one GPU during the last tick.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuProcess {
    pub pid: u32,
    /// Executable file name, e.g. "dwm.exe"; "Idle"/"System" for pids 0/4, "PID <n>" when
    /// the process ended before its name could be read.
    pub name: String,
    /// Busiest single engine of the process, 0..=100; `None` on the first tick after a
    /// (re)attach or for a process that just appeared (rate counter without two samples).
    pub load_percent: Option<f64>,
    /// Driver name of that engine ("3D", "VideoEncode", "Video Codec 0"...), only when the
    /// load is above 0.
    pub engine: Option<String>,
    pub dedicated_bytes: Option<u64>,
    pub shared_bytes: Option<u64>,
}

#[derive(Default)]
struct Inner {
    by_luid: HashMap<u64, Vec<GpuProcess>>,
    devices: HashMap<String, u64>,
    /// Set by `publish`; `None` before the first publish.
    last_publish: Option<Instant>,
}

/// Latest per-process GPU usage, keyed by adapter; cheap to clone (shared state).
#[derive(Clone, Default)]
pub struct GpuProcessTable(Arc<Mutex<Inner>>);

/// Busiest first: load (unknown last), then dedicated memory (unknown last), then pid.
fn busiest_first(a: &GpuProcess, b: &GpuProcess) -> Ordering {
    let load = |p: &GpuProcess| p.load_percent.unwrap_or(-1.0);
    load(b)
        .total_cmp(&load(a))
        .then_with(|| b.dedicated_bytes.cmp(&a.dedicated_bytes))
        .then_with(|| a.pid.cmp(&b.pid))
}

impl GpuProcessTable {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Processes of GPU `device_id`, busiest first, at most 20; empty for an unknown device
    /// or when the last publish is older than `MAX_ROW_AGE` (the provider's worker thread
    /// has hung).
    pub fn processes(&self, device_id: &str) -> Vec<GpuProcess> {
        self.processes_at(device_id, Instant::now())
    }

    /// `processes`, with the "now" used for the staleness check made explicit for testing.
    fn processes_at(&self, device_id: &str, now: Instant) -> Vec<GpuProcess> {
        let mut rows = {
            let inner = self.lock();
            let stale = inner
                .last_publish
                .is_none_or(|last| now.duration_since(last) >= MAX_ROW_AGE);
            if stale {
                return Vec::new();
            }
            let Some(luid) = inner.devices.get(device_id) else {
                return Vec::new();
            };
            inner.by_luid.get(luid).cloned().unwrap_or_default()
        };
        rows.sort_by(busiest_first);
        rows.truncate(MAX_ROWS);
        rows
    }

    /// Device id -> adapter LUID of the last discover (replaces the previous mapping).
    pub(crate) fn set_devices(&self, devices: Vec<(String, u64)>) {
        self.lock().devices = devices.into_iter().collect();
    }

    /// Replaces the whole table with the rows of the last tick and records the publish time.
    pub(crate) fn publish(&self, by_luid: HashMap<u64, Vec<GpuProcess>>) {
        let mut inner = self.lock();
        inner.by_luid = by_luid;
        inner.last_publish = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, load: Option<f64>, dedicated: Option<u64>) -> GpuProcess {
        GpuProcess {
            pid,
            name: format!("p{pid}.exe"),
            load_percent: load,
            engine: None,
            dedicated_bytes: dedicated,
            shared_bytes: None,
        }
    }

    const RTX: u64 = 0x18036;
    const RADEON: u64 = 0x1AAD7;

    fn table() -> GpuProcessTable {
        let table = GpuProcessTable::new();
        table.set_devices(vec![
            ("gpu/pci-0000:01:00.0".to_owned(), RTX),
            ("gpu/pci-0000:11:00.0".to_owned(), RADEON),
        ]);
        table
    }

    #[test]
    fn rows_are_sorted_by_load_then_dedicated_memory() {
        let table = table();
        table.publish(HashMap::from([(
            RTX,
            vec![
                row(10, None, Some(900)),
                row(11, Some(5.0), Some(1)),
                row(12, Some(40.0), None),
                row(13, Some(5.0), Some(2_000)),
                row(14, None, None),
                row(15, None, Some(900)),
            ],
        )]));
        let pids: Vec<u32> = table
            .processes("gpu/pci-0000:01:00.0")
            .iter()
            .map(|p| p.pid)
            .collect();
        assert_eq!(pids, [12, 13, 11, 10, 15, 14]);
    }

    #[test]
    fn at_most_twenty_rows_are_returned() {
        let table = table();
        let rows = (0..30)
            .map(|pid| row(pid, Some(f64::from(pid)), None))
            .collect();
        table.publish(HashMap::from([(RTX, rows)]));
        let top = table.processes("gpu/pci-0000:01:00.0");
        assert_eq!(top.len(), 20);
        assert_eq!(top[0].pid, 29);
        assert_eq!(top[19].pid, 10);
    }

    #[test]
    fn unknown_device_or_adapter_gives_an_empty_list() {
        let table = table();
        table.publish(HashMap::from([(RTX, vec![row(1, Some(1.0), None)])]));
        assert!(table.processes("gpu/pci-0000:11:00.0").is_empty());
        assert!(table.processes("gpu/unknown").is_empty());
        assert!(GpuProcessTable::new()
            .processes("gpu/pci-0000:01:00.0")
            .is_empty());
    }

    #[test]
    fn clones_share_the_table_and_publish_replaces_it() {
        let table = table();
        let reader = table.clone();
        table.publish(HashMap::from([(RTX, vec![row(1, Some(1.0), None)])]));
        assert_eq!(reader.processes("gpu/pci-0000:01:00.0").len(), 1);
        table.publish(HashMap::new());
        assert!(reader.processes("gpu/pci-0000:01:00.0").is_empty());
        // A rediscover that drops a device also drops its rows from the answers.
        table.publish(HashMap::from([(RADEON, vec![row(2, None, Some(5))])]));
        table.set_devices(vec![("gpu/pci-0000:01:00.0".to_owned(), RTX)]);
        assert!(reader.processes("gpu/pci-0000:11:00.0").is_empty());
    }

    #[test]
    fn stale_rows_are_hidden_once_the_provider_hangs() {
        let table = table();
        table.publish(HashMap::from([(RTX, vec![row(1, Some(50.0), None)])]));
        // The exact publish instant is not observable from the test; using it as a
        // baseline with a small margin avoids flakiness from the call above.
        let published_around = Instant::now();
        assert!(
            !table
                .processes_at("gpu/pci-0000:01:00.0", published_around)
                .is_empty(),
            "fresh rows are visible"
        );
        assert!(
            !table
                .processes_at(
                    "gpu/pci-0000:01:00.0",
                    published_around + MAX_ROW_AGE - Duration::from_millis(50)
                )
                .is_empty(),
            "just under the limit: still visible"
        );
        assert!(
            table
                .processes_at(
                    "gpu/pci-0000:01:00.0",
                    published_around + MAX_ROW_AGE + Duration::from_millis(50)
                )
                .is_empty(),
            "a hung provider's stale rows must not be shown"
        );
    }

    #[test]
    fn serializes_with_the_ts_contract_keys() {
        let process = GpuProcess {
            pid: 2096,
            name: "dwm.exe".to_owned(),
            load_percent: Some(3.5),
            engine: Some("3D".to_owned()),
            dedicated_bytes: Some(2_000_000_000),
            shared_bytes: None,
        };
        assert_eq!(
            serde_json::to_value(&process).unwrap(),
            serde_json::json!({
                "pid": 2096,
                "name": "dwm.exe",
                "loadPercent": 3.5,
                "engine": "3D",
                "dedicatedBytes": 2_000_000_000u64,
                "sharedBytes": null
            })
        );
    }
}
