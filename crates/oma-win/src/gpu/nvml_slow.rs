//! The slow NVML readings (energy since start, PCIe throughput) on their own thread.
//!
//! `nvmlDeviceGetPcieThroughput` blocks for about 30 ms and `nvmlDeviceGetTotalEnergyConsumption`
//! has a p95 around 9 ms, so the sampling tick never calls them: the `oma-gpu-slow` thread
//! reads them at 1 Hz while somebody keeps asking (`touch`), sleeps on a condition variable
//! otherwise, and publishes the latest values. The tick only reads that cache. No NVML call is
//! made while the mutex shared with the tick is held.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::adapter::PciAddress;
use super::field::GpuField;
use super::layer::Readings;

/// The fields served by this thread.
pub(super) const SLOW_FIELDS: [GpuField; 3] =
    [GpuField::EnergyTotal, GpuField::PcieRx, GpuField::PcieTx];

/// No `touch` for this long puts the thread to sleep.
const IDLE_AFTER: Duration = Duration::from_secs(3);
/// Cached values older than this are absent.
const FRESH_FOR: Duration = Duration::from_secs(3);
const PERIOD: Duration = Duration::from_secs(1);
/// NVML reports PCIe throughput in "KB/s" without defining KB; 1024 bytes is the spec 4.2
/// choice.
const KB: f64 = 1024.0;

/// Joules since the first reading of an NVML millijoule counter. A counter that goes
/// backwards (driver reload) becomes the new base; the total never decreases.
#[derive(Debug, Default)]
pub(super) struct EnergyMeter {
    last_mj: Option<u64>,
    total_mj: u64,
}

impl EnergyMeter {
    pub(super) fn update(&mut self, counter_mj: u64) -> f64 {
        if let Some(last) = self.last_mj {
            self.total_mj = self
                .total_mj
                .saturating_add(counter_mj.saturating_sub(last));
        }
        self.last_mj = Some(counter_mj);
        self.total_mj as f64 / 1000.0
    }
}

/// When the tick last asked for the slow values.
#[derive(Debug, Default)]
pub(super) struct Demand {
    last_touch: Option<Instant>,
}

impl Demand {
    pub(super) fn touch(&mut self, now: Instant) {
        self.last_touch = Some(now);
    }

    pub(super) fn idle(&self, now: Instant) -> bool {
        self.last_touch
            .is_none_or(|at| now.saturating_duration_since(at) >= IDLE_AFTER)
    }
}

/// Whether a value published at `at` can still be shown at `now`.
pub(super) fn fresh(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) <= FRESH_FOR
}

/// NVML device handle moved to the slow thread.
#[derive(Clone, Copy)]
pub(super) struct Handle(pub(super) *mut c_void);

// SAFETY: NVML is documented as thread-safe, and a device handle is a process-wide token that
// stays valid for the library's lifetime; nvml.dll is never unloaded nor shut down (D1).
unsafe impl Send for Handle {}

/// A GPU the thread reads, with the slow fields it supports.
#[derive(Clone)]
pub(super) struct Target {
    /// Identity of the GPU across re-enumerations (a handle may change, the address does not).
    pub(super) key: PciAddress,
    pub(super) handle: Handle,
    pub(super) fields: Vec<GpuField>,
}

/// Last published values of one GPU.
#[derive(Default)]
struct Latest {
    /// The last read said the GPU or driver is gone: the layer must rediscover.
    stale: bool,
    values: [Option<(f64, Instant)>; 3],
}

impl Latest {
    fn slot(field: GpuField) -> Option<usize> {
        SLOW_FIELDS.iter().position(|&f| f == field)
    }

    fn readings(&self, now: Instant) -> Readings {
        SLOW_FIELDS
            .iter()
            .zip(&self.values)
            .filter_map(|(&field, v)| {
                let (value, at) = (*v)?;
                fresh(at, now).then_some((field, value))
            })
            .collect()
    }
}

#[derive(Default)]
struct State {
    demand: Demand,
    targets: Vec<Target>,
    latest: HashMap<PciAddress, Latest>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Handle on the running `oma-gpu-slow` thread. The thread is deliberately never joined and
/// may outlive its layer (it lives as long as the process; tests leak one parked thread per
/// load + attach, which is harmless).
pub(super) struct SlowReader {
    shared: Arc<Shared>,
}

impl SlowReader {
    /// Starts the thread. `read(handle, field)` returns the raw NVML counter (millijoules for
    /// the energy, KB/s for the PCIe throughput) or `Err(stale)`, where `stale` means the GPU
    /// or the driver is gone.
    pub(super) fn start(
        read: impl Fn(*mut c_void, GpuField) -> Result<u64, bool> + Send + 'static,
    ) -> Option<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("oma-gpu-slow".into())
            .spawn(move || run(&worker, read))
            .map_err(|e| tracing::warn!(error = %e, "could not start the GPU slow thread"))
            .ok()?;
        Some(Self { shared })
    }

    pub(super) fn set_targets(&self, targets: Vec<Target>) {
        let mut state = self.shared.lock();
        state.targets = targets;
        // A rediscover is what the stale flag asked for.
        state.latest.values_mut().for_each(|l| l.stale = false);
    }

    /// Marks demand (waking the thread if it sleeps) and returns the fresh cached values of
    /// the GPU at `key`, or `None` when its last read found it lost. Never calls NVML.
    pub(super) fn poll(&self, key: PciAddress, now: Instant) -> Option<Readings> {
        let mut state = self.shared.lock();
        state.demand.touch(now);
        let readings = match state.latest.get(&key) {
            Some(latest) if latest.stale => None,
            Some(latest) => Some(latest.readings(now)),
            None => Some(Readings::new()),
        };
        drop(state);
        self.shared.wake.notify_one();
        readings
    }
}

/// One round of slow reads, with no lock held: `(gpu, field, value or stale)`.
fn read_all(
    meters: &mut HashMap<PciAddress, EnergyMeter>,
    targets: &[Target],
    read: &impl Fn(*mut c_void, GpuField) -> Result<u64, bool>,
) -> Vec<(PciAddress, GpuField, Result<f64, bool>)> {
    let mut out = Vec::new();
    for target in targets {
        for &field in &target.fields {
            let value = read(target.handle.0, field).map(|raw| match field {
                GpuField::EnergyTotal => meters.entry(target.key).or_default().update(raw),
                _ => raw as f64 * KB,
            });
            out.push((target.key, field, value));
        }
    }
    out
}

fn run(shared: &Shared, read: impl Fn(*mut c_void, GpuField) -> Result<u64, bool>) {
    let mut meters: HashMap<PciAddress, EnergyMeter> = HashMap::new();
    loop {
        let targets = {
            let mut state = shared.lock();
            while state.demand.idle(Instant::now()) {
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            state.targets.clone()
        };
        // The (slow) NVML calls happen with no lock held.
        let published = read_all(&mut meters, &targets, &read);
        {
            let mut state = shared.lock();
            let at = Instant::now();
            for (key, field, value) in published {
                let latest = state.latest.entry(key).or_default();
                match value {
                    Ok(value) => {
                        if let Some(slot) = Latest::slot(field) {
                            latest.values[slot] = Some((value, at));
                        }
                    }
                    Err(true) => latest.stale = true,
                    Err(false) => {}
                }
            }
        }
        std::thread::sleep(PERIOD);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn energy_counts_from_the_first_reading() {
        let mut meter = EnergyMeter::default();
        assert_eq!(meter.update(5_000_000), 0.0);
        assert_eq!(meter.update(5_002_500), 2.5);
        assert_eq!(meter.update(5_002_500), 2.5);
    }

    #[test]
    fn energy_survives_a_counter_reset() {
        let mut meter = EnergyMeter::default();
        meter.update(1_000);
        assert_eq!(meter.update(3_000), 2.0);
        // Driver reload: the counter restarts lower; the total holds and counts on from there.
        assert_eq!(meter.update(500), 2.0);
        assert_eq!(meter.update(1_500), 3.0);
    }

    #[test]
    fn demand_goes_idle_after_three_seconds() {
        let t0 = Instant::now();
        let mut demand = Demand::default();
        assert!(demand.idle(t0), "never touched");
        demand.touch(t0);
        assert!(!demand.idle(t0 + Duration::from_millis(2_999)));
        assert!(demand.idle(t0 + Duration::from_secs(3)));
        demand.touch(t0 + Duration::from_secs(3));
        assert!(!demand.idle(t0 + Duration::from_secs(4)));
    }

    #[test]
    fn stale_slow_values_are_absent() {
        let t0 = Instant::now();
        assert!(fresh(t0, t0 + Duration::from_secs(3)));
        assert!(!fresh(t0, t0 + Duration::from_millis(3_001)));
        let mut latest = Latest::default();
        latest.values[0] = Some((12.0, t0));
        latest.values[1] = Some((1024.0, t0 + Duration::from_secs(2)));
        let readings = latest.readings(t0 + Duration::from_millis(4_000));
        assert_eq!(readings, Readings::from([(GpuField::PcieRx, 1024.0)]));
    }

    fn target(key: u32, handle: usize) -> Target {
        Target {
            key: PciAddress {
                bus: key,
                device: 0,
                function: 0,
            },
            handle: Handle(handle as *mut c_void),
            fields: vec![GpuField::EnergyTotal],
        }
    }

    #[test]
    fn a_new_handle_for_the_same_gpu_keeps_the_energy_total() {
        let read = |handle: *mut c_void, _| Ok(handle as u64 * 1_000);
        let mut meters = HashMap::new();
        let first = read_all(&mut meters, &[target(1, 5)], &read);
        assert_eq!(first[0].2, Ok(0.0));
        // The same GPU re-enumerated with another handle and a counter 3 J higher.
        let second = read_all(&mut meters, &[target(1, 8)], &read);
        assert_eq!(second[0].2, Ok(3.0));
    }

    #[test]
    fn a_lost_gpu_makes_the_poll_stale_until_the_next_attach() {
        let t0 = Instant::now();
        let t = target(1, 5);
        let reader = SlowReader {
            shared: Arc::new(Shared {
                state: Mutex::new(State::default()),
                wake: Condvar::new(),
            }),
        };
        assert_eq!(reader.poll(t.key, t0), Some(Readings::new()));
        reader.shared.lock().latest.entry(t.key).or_default().stale = true;
        assert_eq!(reader.poll(t.key, t0), None);
        reader.set_targets(vec![t.clone()]);
        assert_eq!(reader.poll(t.key, t0), Some(Readings::new()));
    }
}
