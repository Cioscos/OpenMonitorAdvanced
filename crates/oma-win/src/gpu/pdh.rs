//! GPU engine load and adapter memory from the PDH "GPU Engine" and
//! "GPU Adapter Memory" counters, aggregated the way Task Manager does, plus
//! the per-process table from the same engine rows and "GPU Process Memory".

use std::collections::{BTreeMap, BTreeSet, HashMap};

use oma_core::model::Source;
use oma_core::provider::ProviderError;

use super::adapter::Adapter;
use super::field::GpuField;
use super::layer::{GpuLayer, Readings};
use super::processes::{GpuProcess, GpuProcessTable};
use super::procname::ProcessNames;
use crate::pdh::{Counter, PdhError, Query};

const ENGINE: &str = r"\GPU Engine(*)\Utilization Percentage";
const DEDICATED: &str = r"\GPU Adapter Memory(*)\Dedicated Usage";
const SHARED: &str = r"\GPU Adapter Memory(*)\Shared Usage";
const PROCESS_DEDICATED: &str = r"\GPU Process Memory(*)\Dedicated Usage";
const PROCESS_SHARED: &str = r"\GPU Process Memory(*)\Shared Usage";

const LOAD_FIELDS: [GpuField; 6] = [
    GpuField::LoadCore,
    GpuField::Load3d,
    GpuField::LoadCompute,
    GpuField::LoadCopy,
    GpuField::LoadVideoDecode,
    GpuField::LoadVideoEncode,
];

/// One "GPU Engine" instance: an engine (D3DKMT node) of an adapter, as seen
/// by one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EngineInstance {
    /// Process owning the instance (4 = System).
    pub pid: u32,
    pub luid: u64,
    /// Engine ordinal (`eng_N`), equal to the D3DKMT node ordinal.
    pub engine: u32,
    /// Driver-defined engine name, e.g. "3D", "Copy", "Video Codec 0".
    pub engtype: String,
}

fn hex_u32(text: &str) -> Option<u32> {
    u32::from_str_radix(text.strip_prefix("0x")?, 16).ok()
}

/// Parses `0x{high:08X}_0x{low:08X}` at the start of `text`; returns the
/// packed LUID and the text after the low part.
fn parse_luid(text: &str) -> Option<(u64, &str)> {
    let (high, rest) = text.split_once('_')?;
    let (low, rest) = rest.split_at(rest.find('_').unwrap_or(rest.len()));
    Some((((hex_u32(high)? as u64) << 32) | hex_u32(low)? as u64, rest))
}

/// `pid_15028_luid_0x00000000_0x00017DB6_phys_0_eng_0_engtype_3D`. The engine
/// type is everything after `_engtype_` and may contain spaces or underscores.
pub(crate) fn parse_engine(instance: &str) -> Option<EngineInstance> {
    let (head, engtype) = instance.split_once("_engtype_")?;
    let (pid, rest) = head.strip_prefix("pid_")?.split_once("_luid_")?;
    let pid = pid.parse::<u32>().ok()?;
    let (luid, rest) = parse_luid(rest)?;
    let (phys, engine) = rest.strip_prefix("_phys_")?.split_once("_eng_")?;
    phys.parse::<u32>().ok()?;
    Some(EngineInstance {
        pid,
        luid,
        engine: engine.parse().ok()?,
        engtype: engtype.to_owned(),
    })
}

/// `luid_0x00000000_0x00017DB6_phys_0` → the adapter LUID. Per-process
/// instances (`pid_…`) are rejected.
pub(crate) fn parse_adapter_memory(instance: &str) -> Option<u64> {
    let (luid, rest) = parse_luid(instance.strip_prefix("luid_")?)?;
    rest.strip_prefix("_phys_")?.parse::<u32>().ok()?;
    Some(luid)
}

/// `pid_26328_luid_0x00000000_0x00018036_phys_0` → (pid, LUID). Adapter-wide
/// instances (no `pid_`) are rejected.
pub(crate) fn parse_process_memory(instance: &str) -> Option<(u32, u64)> {
    let (pid, rest) = instance.strip_prefix("pid_")?.split_once("_luid_")?;
    let (luid, rest) = parse_luid(rest)?;
    rest.strip_prefix("_phys_")?.parse::<u32>().ok()?;
    Some((pid.parse().ok()?, luid))
}

/// Maps a driver engine name to its load field (case-insensitive). Engines
/// such as Security, Timer, VR, OFA or LegacyOverlay only count toward LoadCore.
pub(crate) fn classify(engtype: &str) -> Option<GpuField> {
    let name = engtype.to_ascii_lowercase();
    if name == "3d" || name == "high priority 3d" || name.starts_with("graphics") {
        Some(GpuField::Load3d)
    } else if name.starts_with("compute")
        || name == "high priority compute"
        || name.starts_with("cuda")
    {
        Some(GpuField::LoadCompute)
    } else if name.starts_with("copy") {
        Some(GpuField::LoadCopy)
    } else if name.contains("decode")
        || name.starts_with("video codec")
        || name.starts_with("video jpeg")
    {
        Some(GpuField::LoadVideoDecode)
    } else if name.contains("encode") {
        Some(GpuField::LoadVideoEncode)
    } else {
        None
    }
}

fn raise(readings: &mut Readings, field: GpuField, value: f64) {
    let slot = readings.entry(field).or_insert(value);
    *slot = slot.max(value);
}

/// Task Manager's aggregation for one adapter: each engine's utilization is
/// summed over processes; LoadCore is the busiest engine and each typed field
/// the busiest engine of that type, all clamped to 0..=100. NaN rows (a
/// process that just appeared) are ignored.
pub(crate) fn aggregate(rows: &[(EngineInstance, f64)], luid: u64) -> Readings {
    let mut engines: BTreeMap<u32, (&str, f64)> = BTreeMap::new();
    for (instance, value) in rows {
        if instance.luid != luid || !value.is_finite() {
            continue;
        }
        engines
            .entry(instance.engine)
            .or_insert((instance.engtype.as_str(), 0.0))
            .1 += value;
    }
    let mut readings = Readings::new();
    for (engtype, total) in engines.into_values() {
        let load = total.clamp(0.0, 100.0);
        raise(&mut readings, GpuField::LoadCore, load);
        if let Some(field) = classify(engtype) {
            raise(&mut readings, field, load);
        }
    }
    readings
}

/// Adapter-memory rows summed per LUID (linked adapters have one row per
/// physical adapter); non-adapter rows and NaN values are ignored.
pub(crate) fn memory_by_luid(rows: &[(String, f64)]) -> BTreeMap<u64, f64> {
    let mut totals = BTreeMap::new();
    for (instance, value) in rows {
        if let (Some(luid), true) = (parse_adapter_memory(instance), value.is_finite()) {
            *totals.entry(luid).or_insert(0.0) += value;
        }
    }
    totals
}

/// Fields available for `luid` given the instance names seen at attach.
///
/// Engine instances exist only for processes using the adapter, so an idle
/// GPU (e.g. an Optimus dGPU) has none at attach. LoadCore is therefore also
/// declared for any adapter PDH knows through "GPU Adapter Memory": the
/// wildcard engine counter picks up instances that appear later, and until
/// then the adapter reads 0. Typed engine fields still need an instance at attach.
pub(crate) fn supported_fields(
    engines: &[EngineInstance],
    dedicated: &BTreeSet<u64>,
    shared: &BTreeSet<u64>,
    luid: u64,
) -> BTreeSet<GpuField> {
    let mut fields = BTreeSet::new();
    for engine in engines.iter().filter(|e| e.luid == luid) {
        fields.insert(GpuField::LoadCore);
        fields.extend(classify(&engine.engtype));
    }
    if dedicated.contains(&luid) {
        fields.insert(GpuField::LoadCore);
        fields.insert(GpuField::MemoryDedicatedUsed);
    }
    if shared.contains(&luid) {
        fields.insert(GpuField::LoadCore);
        fields.insert(GpuField::MemorySharedUsed);
    }
    fields
}

/// One adapter's readings for a tick. `engines` is `None` when engine load is
/// not available this tick (first sample after attach, or PDH has no rate
/// yet); memory is a plain gauge and is reported regardless.
pub(crate) fn adapter_readings(
    engines: Option<&[(EngineInstance, f64)]>,
    dedicated: &BTreeMap<u64, f64>,
    shared: &BTreeMap<u64, f64>,
    luid: u64,
    supported: &BTreeSet<GpuField>,
) -> Readings {
    let mut readings = Readings::new();
    if let Some(rows) = engines {
        readings = aggregate(rows, luid);
        // A supported engine type with no process instance left is idle, not unknown.
        for field in LOAD_FIELDS {
            readings.entry(field).or_insert(0.0);
        }
    }
    if let Some(&bytes) = dedicated.get(&luid) {
        readings.insert(GpuField::MemoryDedicatedUsed, bytes);
    }
    if let Some(&bytes) = shared.get(&luid) {
        readings.insert(GpuField::MemorySharedUsed, bytes);
    }
    readings.retain(|field, _| supported.contains(field));
    readings
}

/// GPU use of one process on one adapter during a tick.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ProcessUsage {
    pub load: Option<f64>,
    pub engine: Option<String>,
    pub dedicated: Option<u64>,
    pub shared: Option<u64>,
}

/// Adds per-process memory rows (bytes) to `usage`; `phys_N` rows of the same
/// process and adapter are summed, NaN and negative rows ignored.
fn add_memory(
    usage: &mut BTreeMap<(u64, u32), ProcessUsage>,
    rows: &[(String, f64)],
    slot: fn(&mut ProcessUsage) -> &mut Option<u64>,
) {
    for (instance, value) in rows {
        let Some((pid, luid)) = parse_process_memory(instance) else {
            continue;
        };
        if !value.is_finite() || *value < 0.0 {
            continue;
        }
        let bytes = slot(usage.entry((luid, pid)).or_default());
        *bytes = Some(bytes.unwrap_or(0) + value.round() as u64);
    }
}

/// Per (LUID, pid) usage for a tick (decision D5). The load of a process is its
/// busiest single engine (not a sum per engine type, which can exceed the
/// adapter's own LoadCore), clamped to 0..=100; `engine` names that engine only
/// when the load is above 0. `engines` is `None` when engine load is not
/// available this tick: every load is then unknown. A process whose engine rows
/// are all NaN (it just appeared) has an unknown load too.
pub(crate) fn process_usage(
    engines: Option<&[(EngineInstance, f64)]>,
    dedicated: &[(String, f64)],
    shared: &[(String, f64)],
) -> BTreeMap<(u64, u32), ProcessUsage> {
    let mut usage: BTreeMap<(u64, u32), ProcessUsage> = BTreeMap::new();
    for (instance, value) in engines.unwrap_or_default() {
        let entry = usage.entry((instance.luid, instance.pid)).or_default();
        if !value.is_finite() {
            continue;
        }
        let load = value.clamp(0.0, 100.0);
        if entry.load.is_none_or(|busiest| load > busiest) {
            entry.load = Some(load);
            entry.engine = (load > 0.0).then(|| instance.engtype.clone());
        }
    }
    add_memory(&mut usage, dedicated, |u| &mut u.dedicated);
    add_memory(&mut usage, shared, |u| &mut u.shared);
    usage
}

/// Table rows grouped by adapter LUID, named through `name`.
pub(crate) fn process_rows(
    usage: BTreeMap<(u64, u32), ProcessUsage>,
    name: impl Fn(u32) -> String,
) -> HashMap<u64, Vec<GpuProcess>> {
    let mut rows: HashMap<u64, Vec<GpuProcess>> = HashMap::new();
    for ((luid, pid), u) in usage {
        rows.entry(luid).or_default().push(GpuProcess {
            pid,
            name: name(pid),
            load_percent: u.load,
            engine: u.engine,
            dedicated_bytes: u.dedicated,
            shared_bytes: u.shared,
        });
    }
    rows
}

struct Counters {
    query: Query,
    engine: Option<Counter>,
    dedicated: Option<Counter>,
    shared: Option<Counter>,
    process_dedicated: Option<Counter>,
    process_shared: Option<Counter>,
}

impl Counters {
    fn open() -> Result<Self, PdhError> {
        let mut query = Query::open()?;
        let mut add = |path: &str| {
            query
                .add_english(path)
                .map_err(|e| tracing::warn!(error = %e, path, "GPU PDH counter unavailable"))
                .ok()
        };
        let engine = add(ENGINE);
        let dedicated = add(DEDICATED);
        let shared = add(SHARED);
        let process_dedicated = add(PROCESS_DEDICATED);
        let process_shared = add(PROCESS_SHARED);
        Ok(Self {
            query,
            engine,
            dedicated,
            shared,
            process_dedicated,
            process_shared,
        })
    }

    fn instances(&self, counter: Option<Counter>) -> Result<Vec<String>, PdhError> {
        counter.map_or(Ok(Vec::new()), |c| self.query.instances(c))
    }

    /// Rows of an optional per-process counter. A failed read counts as no rows,
    /// so the process table never costs the adapter readings.
    fn process_rows(&self, counter: Option<Counter>) -> Vec<(String, f64)> {
        let Some(counter) = counter else {
            return Vec::new();
        };
        self.query.array(counter).unwrap_or_else(|e| {
            tracing::debug!(error = %e, "GPU process memory counter read failed");
            Vec::new()
        })
    }

    fn memory(&self, counter: Option<Counter>) -> Result<BTreeMap<u64, f64>, PdhError> {
        Ok(counter
            .map(|c| self.query.array(c))
            .transpose()?
            .map(|rows| memory_by_luid(&rows))
            .unwrap_or_default())
    }
}

/// PDH layer. Owns its own query so a slow or failing GPU counter set never
/// affects the CPU provider's query.
#[derive(Default)]
pub(crate) struct PdhLayer {
    counters: Option<Counters>,
    /// (LUID, supported fields) per adapter of the last attach.
    adapters: Vec<(u64, BTreeSet<GpuField>)>,
    /// Set by `attach`, consumed by the next `sample` (CpuProvider's rule).
    fresh: bool,
    /// Where each tick's per-process rows are published.
    processes: GpuProcessTable,
    names: ProcessNames,
}

impl PdhLayer {
    pub(crate) fn new(processes: GpuProcessTable) -> Self {
        Self {
            processes,
            ..Self::default()
        }
    }

    /// One tick: adapter readings, and the per-process table published as a side effect.
    fn read(&mut self, fresh: bool) -> Result<Vec<Readings>, ProviderError> {
        let Some(counters) = self.counters.as_mut() else {
            self.processes.publish(HashMap::new());
            return Ok(vec![Readings::new(); self.adapters.len()]);
        };
        counters.query.collect()?;
        let engines: Option<Vec<(EngineInstance, f64)>> = match counters.engine {
            Some(counter) if !fresh => {
                let rows: Vec<_> = counters
                    .query
                    .array(counter)?
                    .into_iter()
                    .filter_map(|(name, value)| parse_engine(&name).map(|e| (e, value)))
                    .collect();
                (!rows.is_empty()).then_some(rows)
            }
            _ => None,
        };
        let dedicated = counters.memory(counters.dedicated)?;
        let shared = counters.memory(counters.shared)?;
        let usage = process_usage(
            engines.as_deref(),
            &counters.process_rows(counters.process_dedicated),
            &counters.process_rows(counters.process_shared),
        );
        self.names
            .update(&usage.keys().map(|&(_, pid)| pid).collect());
        let names = &self.names;
        self.processes
            .publish(process_rows(usage, |pid| names.name(pid)));
        Ok(self
            .adapters
            .iter()
            .map(|(luid, supported)| {
                adapter_readings(engines.as_deref(), &dedicated, &shared, *luid, supported)
            })
            .collect())
    }

    fn open(adapters: &[Adapter]) -> Result<(Counters, Vec<BTreeSet<GpuField>>), PdhError> {
        let mut counters = Counters::open()?;
        counters.query.collect()?;
        let engines: Vec<EngineInstance> = counters
            .instances(counters.engine)?
            .iter()
            .filter_map(|name| parse_engine(name))
            .collect();
        let luids = |names: Vec<String>| -> BTreeSet<u64> {
            names
                .iter()
                .filter_map(|name| parse_adapter_memory(name))
                .collect()
        };
        let dedicated = luids(counters.instances(counters.dedicated)?);
        let shared = luids(counters.instances(counters.shared)?);
        let supported = adapters
            .iter()
            .map(|a| supported_fields(&engines, &dedicated, &shared, a.luid))
            .collect();
        Ok((counters, supported))
    }
}

impl GpuLayer for PdhLayer {
    fn source(&self) -> Source {
        Source::Pdh
    }

    fn attach(&mut self, adapters: &[Adapter]) -> Vec<BTreeSet<GpuField>> {
        self.counters = None;
        self.fresh = true;
        let supported = match Self::open(adapters) {
            Ok((counters, supported)) => {
                self.counters = Some(counters);
                supported
            }
            Err(e) => {
                tracing::warn!(error = %e, "GPU PDH counters unavailable");
                vec![BTreeSet::new(); adapters.len()]
            }
        };
        self.adapters = adapters
            .iter()
            .map(|a| a.luid)
            .zip(supported.iter().cloned())
            .collect();
        supported
    }

    fn sample(&mut self) -> Result<Vec<Readings>, ProviderError> {
        // Utilization Percentage is a rate counter: the collect right after
        // attach spans a few milliseconds and yields noise, so like
        // CpuProvider the first sample only primes it (memory is still read).
        let fresh = std::mem::replace(&mut self.fresh, false);
        let result = self.read(fresh);
        if result.is_err() {
            // No stale rows while the counters fail.
            self.processes.publish(HashMap::new());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RTX: u64 = 0x17DB6;
    const RADEON: u64 = 0x1A331;
    const BASIC_RENDER: u64 = 0x1A2C6;

    fn engine(luid: u64, engine: u32, engtype: &str) -> EngineInstance {
        process_engine(0, luid, engine, engtype)
    }

    fn process_engine(pid: u32, luid: u64, engine: u32, engtype: &str) -> EngineInstance {
        EngineInstance {
            pid,
            luid,
            engine,
            engtype: engtype.to_owned(),
        }
    }

    /// Engine instances observed on this machine (one process).
    fn this_machine() -> Vec<EngineInstance> {
        let rtx = [
            "3D",
            "LegacyOverlay",
            "VideoDecode",
            "Copy",
            "Copy",
            "Security",
            "VideoEncode",
            "VideoEncode",
            "OFA_0",
            "VR",
            "Copy",
            "Copy",
            "Copy",
            "Copy",
            "Security_1",
        ];
        let radeon = [
            "3D",
            "Copy",
            "Compute 0",
            "Compute 1",
            "Timer 0",
            "Security 1",
            "High Priority Compute",
            "High Priority 3D",
            "Video JPEG 0",
            "Video Decode 1",
            "Video Codec 0",
        ];
        let mut all = Vec::new();
        for (i, t) in rtx.iter().enumerate() {
            all.push(engine(RTX, i as u32, t));
        }
        for (i, t) in radeon.iter().enumerate() {
            all.push(engine(RADEON, i as u32, t));
        }
        all.push(engine(BASIC_RENDER, 0, "3D"));
        all
    }

    #[test]
    fn parses_engine_instances() {
        assert_eq!(
            parse_engine("pid_15028_luid_0x00000000_0x00017DB6_phys_0_eng_0_engtype_3D"),
            Some(process_engine(15028, RTX, 0, "3D"))
        );
        assert_eq!(
            parse_engine("pid_6860_luid_0x00000000_0x0001A331_phys_0_eng_10_engtype_Video Codec 0"),
            Some(process_engine(6860, RADEON, 10, "Video Codec 0"))
        );
        assert_eq!(
            parse_engine("pid_4_luid_0x00000000_0x00017DB6_phys_0_eng_14_engtype_Security_1"),
            Some(process_engine(4, RTX, 14, "Security_1"))
        );
        assert_eq!(
            parse_engine("pid_4_luid_0x00000001_0x00000002_phys_1_eng_3_engtype_Copy"),
            Some(process_engine(4, 0x1_0000_0002, 3, "Copy"))
        );
    }

    #[test]
    fn rejects_malformed_engine_instances() {
        for name in [
            "_Total",
            "luid_0x00000000_0x00017DB6_phys_0",
            "pid_x_luid_0x00000000_0x00017DB6_phys_0_eng_0_engtype_3D",
            "pid_1_luid_0x00000000_0x00017DB6_phys_0_eng_x_engtype_3D",
            "pid_1_luid_00000000_00017DB6_phys_0_eng_0_engtype_3D",
            "pid_1_luid_0x00000000_0x00017DB6_eng_0_engtype_3D",
        ] {
            assert_eq!(parse_engine(name), None, "{name}");
        }
    }

    #[test]
    fn parses_adapter_memory_instances() {
        assert_eq!(
            parse_adapter_memory("luid_0x00000000_0x0001A331_phys_0"),
            Some(RADEON)
        );
        assert_eq!(
            parse_adapter_memory("luid_0x00000000_0x00017DB6_phys_0"),
            Some(RTX)
        );
        assert_eq!(
            parse_adapter_memory("pid_6860_luid_0x00000000_0x0001A331_phys_0"),
            None
        );
        assert_eq!(parse_adapter_memory("luid_0x00000000_0x0001A331"), None);
    }

    #[test]
    fn classifies_engine_types_of_this_machine() {
        use GpuField::*;
        let cases = [
            ("3D", Some(Load3d)),
            ("LegacyOverlay", None),
            ("VideoDecode", Some(LoadVideoDecode)),
            ("Copy", Some(LoadCopy)),
            ("Security", None),
            ("VideoEncode", Some(LoadVideoEncode)),
            ("OFA_0", None),
            ("VR", None),
            ("Security_1", None),
            ("Compute 0", Some(LoadCompute)),
            ("Compute 1", Some(LoadCompute)),
            ("Timer 0", None),
            ("Security 1", None),
            ("High Priority Compute", Some(LoadCompute)),
            ("High Priority 3D", Some(Load3d)),
            ("Video JPEG 0", Some(LoadVideoDecode)),
            ("Video Decode 1", Some(LoadVideoDecode)),
            ("Video Codec 0", Some(LoadVideoDecode)),
        ];
        for (engtype, expected) in cases {
            assert_eq!(classify(engtype), expected, "{engtype}");
        }
    }

    #[test]
    fn classification_is_case_insensitive_and_covers_other_vendors() {
        assert_eq!(classify("GRAPHICS_1"), Some(GpuField::Load3d));
        assert_eq!(classify("Cuda"), Some(GpuField::LoadCompute));
        assert_eq!(classify("copy 2"), Some(GpuField::LoadCopy));
        assert_eq!(classify("videoencode"), Some(GpuField::LoadVideoEncode));
        assert_eq!(classify("VideoProcessing"), None);
    }

    #[test]
    fn aggregates_like_task_manager() {
        let rows = vec![
            // Two processes on the 3D engine: summed.
            (engine(RTX, 0, "3D"), 30.0),
            (engine(RTX, 0, "3D"), 25.0),
            (engine(RTX, 2, "VideoDecode"), 18.0),
            // Two copy engines: the busiest one, not the sum.
            (engine(RTX, 3, "Copy"), 1.5),
            (engine(RTX, 4, "Copy"), 0.5),
            (engine(RTX, 5, "Security"), 0.0),
            // Other adapter and NaN rows are ignored.
            (engine(RADEON, 0, "3D"), 90.0),
            (engine(RTX, 6, "VideoEncode"), f64::NAN),
        ];
        let r = aggregate(&rows, RTX);
        assert_eq!(r[&GpuField::LoadCore], 55.0);
        assert_eq!(r[&GpuField::Load3d], 55.0);
        assert_eq!(r[&GpuField::LoadVideoDecode], 18.0);
        assert_eq!(r[&GpuField::LoadCopy], 1.5);
        assert!(!r.contains_key(&GpuField::LoadVideoEncode));
        assert!(!r.contains_key(&GpuField::LoadCompute));
    }

    #[test]
    fn load_is_clamped_to_100() {
        let rows = vec![
            (engine(RADEON, 2, "Compute 0"), 70.0),
            (engine(RADEON, 2, "Compute 0"), 60.0),
        ];
        let r = aggregate(&rows, RADEON);
        assert_eq!(r[&GpuField::LoadCore], 100.0);
        assert_eq!(r[&GpuField::LoadCompute], 100.0);
        assert!(aggregate(&rows, RTX).is_empty());
    }

    #[test]
    fn supported_fields_match_this_machine() {
        use GpuField::*;
        let engines = this_machine();
        let memory = BTreeSet::from([RTX, RADEON, BASIC_RENDER]);
        assert_eq!(
            supported_fields(&engines, &memory, &memory, RTX),
            BTreeSet::from([
                LoadCore,
                Load3d,
                LoadCopy,
                LoadVideoDecode,
                LoadVideoEncode,
                MemoryDedicatedUsed,
                MemorySharedUsed,
            ])
        );
        assert_eq!(
            supported_fields(&engines, &memory, &memory, RADEON),
            BTreeSet::from([
                LoadCore,
                Load3d,
                LoadCompute,
                LoadCopy,
                LoadVideoDecode,
                MemoryDedicatedUsed,
                MemorySharedUsed,
            ])
        );
        assert!(supported_fields(&engines, &BTreeSet::new(), &BTreeSet::new(), 0x42).is_empty());
    }

    #[test]
    fn memory_is_summed_per_adapter() {
        let rows = vec![
            ("luid_0x00000000_0x00017DB6_phys_0".to_owned(), 1_000.0),
            ("luid_0x00000000_0x00017DB6_phys_1".to_owned(), 500.0),
            ("luid_0x00000000_0x0001A331_phys_0".to_owned(), f64::NAN),
            ("pid_4_luid_0x00000000_0x0001A331_phys_0".to_owned(), 7.0),
        ];
        assert_eq!(memory_by_luid(&rows), BTreeMap::from([(RTX, 1_500.0)]));
    }

    #[test]
    fn first_sample_after_attach_has_no_engine_load() {
        let supported = supported_fields(
            &this_machine(),
            &BTreeSet::from([RTX]),
            &BTreeSet::from([RTX]),
            RTX,
        );
        let dedicated = BTreeMap::from([(RTX, 1_563.0 * 1024.0 * 1024.0)]);
        let shared = BTreeMap::from([(RTX, 69.0 * 1024.0 * 1024.0)]);
        let r = adapter_readings(None, &dedicated, &shared, RTX, &supported);
        assert_eq!(
            r,
            Readings::from([
                (GpuField::MemoryDedicatedUsed, 1_563.0 * 1024.0 * 1024.0),
                (GpuField::MemorySharedUsed, 69.0 * 1024.0 * 1024.0),
            ])
        );
    }

    #[test]
    fn idle_engines_read_zero_once_primed() {
        let supported = supported_fields(&this_machine(), &BTreeSet::new(), &BTreeSet::new(), RTX);
        // Only another adapter's rows this tick: every supported RTX load is 0.
        let rows = vec![(engine(RADEON, 0, "3D"), 12.0)];
        let r = adapter_readings(
            Some(&rows),
            &BTreeMap::new(),
            &BTreeMap::new(),
            RTX,
            &supported,
        );
        assert_eq!(r.len(), 5);
        assert!(r.values().all(|&v| v == 0.0));
        assert!(!r.contains_key(&GpuField::LoadCompute));
    }

    #[test]
    fn idle_adapter_at_attach_still_gets_core_load() {
        use GpuField::*;
        // Only the RTX has engine instances at attach; the Radeon only has memory ones.
        let engines: Vec<_> = this_machine()
            .into_iter()
            .filter(|e| e.luid == RTX)
            .collect();
        let memory = BTreeSet::from([RTX, RADEON]);
        let supported = supported_fields(&engines, &memory, &BTreeSet::new(), RADEON);
        assert_eq!(supported, BTreeSet::from([LoadCore, MemoryDedicatedUsed]));
        assert_eq!(
            supported_fields(&engines, &BTreeSet::new(), &memory, RADEON),
            BTreeSet::from([LoadCore, MemorySharedUsed])
        );
        let dedicated = BTreeMap::from([(RADEON, 512.0)]);

        // First sample after attach: no load yet.
        let r = adapter_readings(None, &dedicated, &BTreeMap::new(), RADEON, &supported);
        assert_eq!(r, Readings::from([(MemoryDedicatedUsed, 512.0)]));

        // Primed, still idle (only RTX rows): core load reads 0.
        let rows = vec![(engine(RTX, 0, "3D"), 12.0)];
        let r = adapter_readings(
            Some(&rows),
            &dedicated,
            &BTreeMap::new(),
            RADEON,
            &supported,
        );
        assert_eq!(
            r,
            Readings::from([(LoadCore, 0.0), (MemoryDedicatedUsed, 512.0)])
        );

        // A process starts using the Radeon later: its instances show up in the
        // wildcard counter without re-adding it, and LoadCore follows them.
        let rows = vec![
            (engine(RTX, 0, "3D"), 12.0),
            (engine(RADEON, 0, "3D"), 35.0),
            (engine(RADEON, 1, "Copy"), f64::NAN),
        ];
        let r = adapter_readings(
            Some(&rows),
            &dedicated,
            &BTreeMap::new(),
            RADEON,
            &supported,
        );
        assert_eq!(
            r,
            Readings::from([(LoadCore, 35.0), (MemoryDedicatedUsed, 512.0)])
        );
    }

    #[test]
    fn readings_keep_only_supported_fields() {
        let supported = BTreeSet::from([GpuField::LoadCore]);
        let rows = vec![(engine(RTX, 0, "3D"), 40.0)];
        let dedicated = BTreeMap::from([(RTX, 1.0)]);
        let r = adapter_readings(Some(&rows), &dedicated, &BTreeMap::new(), RTX, &supported);
        assert_eq!(r, Readings::from([(GpuField::LoadCore, 40.0)]));
    }

    #[test]
    fn parses_process_memory_instances() {
        assert_eq!(
            parse_process_memory("pid_26328_luid_0x00000000_0x00018036_phys_0"),
            Some((26328, 0x18036))
        );
        assert_eq!(
            parse_process_memory("pid_4_luid_0x00000001_0x00000002_phys_1"),
            Some((4, 0x1_0000_0002))
        );
        for name in [
            "luid_0x00000000_0x00018036_phys_0",
            "pid_x_luid_0x00000000_0x00018036_phys_0",
            "pid_1_luid_0x00000000_0x00018036",
            "pid_1_luid_0x00000000_0x00018036_phys_0_eng_0_engtype_3D",
        ] {
            assert_eq!(parse_process_memory(name), None, "{name}");
        }
    }

    const FFMPEG: u32 = 18796;
    const DWM: u32 = 2096;

    /// hevc_nvenc ffmpeg on the RTX at one tick (spike): 3D 73.7, two NVENC engines ~50.
    fn nvenc_tick() -> Vec<(EngineInstance, f64)> {
        vec![
            (process_engine(FFMPEG, RTX, 0, "3D"), 73.7),
            (process_engine(FFMPEG, RTX, 6, "VideoEncode"), 50.1),
            (process_engine(FFMPEG, RTX, 7, "VideoEncode"), 49.0),
            (process_engine(FFMPEG, RTX, 3, "Copy"), 0.0),
            (process_engine(DWM, RTX, 0, "3D"), 0.0),
            (process_engine(DWM, RTX, 3, "Copy"), 0.0),
            (process_engine(DWM, RADEON, 0, "3D"), 2.5),
        ]
    }

    #[test]
    fn process_load_is_the_busiest_single_engine() {
        let usage = process_usage(Some(&nvenc_tick()), &[], &[]);
        let ffmpeg = &usage[&(RTX, FFMPEG)];
        // Not 99.1 (the two VideoEncode engines summed): consistent with LoadCore.
        assert_eq!(ffmpeg.load, Some(73.7));
        assert_eq!(ffmpeg.engine.as_deref(), Some("3D"));
        let dwm = &usage[&(RTX, DWM)];
        assert_eq!(dwm.load, Some(0.0));
        assert_eq!(dwm.engine, None, "no engine label at 0 %");
        // The same process on another adapter is a separate row.
        assert_eq!(usage[&(RADEON, DWM)].load, Some(2.5));
        assert_eq!(usage.len(), 3);
    }

    #[test]
    fn process_load_is_clamped_and_nan_is_unknown() {
        let rows = vec![
            (process_engine(1, RTX, 0, "3D"), 130.0),
            (process_engine(2, RTX, 0, "3D"), f64::NAN),
            (process_engine(2, RTX, 1, "Copy"), f64::NAN),
            (process_engine(3, RTX, 0, "3D"), f64::NAN),
            (process_engine(3, RTX, 1, "Copy"), 4.0),
        ];
        let usage = process_usage(Some(&rows), &[], &[]);
        assert_eq!(usage[&(RTX, 1)].load, Some(100.0));
        assert_eq!(usage[&(RTX, 2)].load, None, "just appeared: no rate yet");
        assert_eq!(usage[&(RTX, 3)].load, Some(4.0));
        assert_eq!(usage[&(RTX, 3)].engine.as_deref(), Some("Copy"));
    }

    #[test]
    fn process_memory_is_summed_per_process_and_adapter() {
        let dedicated = vec![
            (
                "pid_2096_luid_0x00000000_0x00017DB6_phys_0".to_owned(),
                1_900_000_000.0,
            ),
            (
                "pid_2096_luid_0x00000000_0x00017DB6_phys_1".to_owned(),
                100_000_000.0,
            ),
            (
                "pid_2096_luid_0x00000000_0x0001A331_phys_0".to_owned(),
                11_600_000.0,
            ),
            (
                "pid_7_luid_0x00000000_0x00017DB6_phys_0".to_owned(),
                f64::NAN,
            ),
            ("luid_0x00000000_0x00017DB6_phys_0".to_owned(), 5.0),
        ];
        let shared = vec![(
            "pid_2096_luid_0x00000000_0x00017DB6_phys_0".to_owned(),
            69_000_000.4,
        )];
        // First tick after attach: no engine rows, memory only.
        let usage = process_usage(None, &dedicated, &shared);
        assert_eq!(
            usage[&(RTX, DWM)],
            ProcessUsage {
                load: None,
                engine: None,
                dedicated: Some(2_000_000_000),
                shared: Some(69_000_000),
            }
        );
        assert_eq!(usage[&(RADEON, DWM)].dedicated, Some(11_600_000));
        assert_eq!(usage[&(RADEON, DWM)].shared, None);
        assert!(!usage.contains_key(&(RTX, 7)), "a NaN-only row is no row");
        assert_eq!(usage.len(), 2);
    }

    #[test]
    fn process_rows_are_grouped_by_adapter_and_named() {
        let usage = process_usage(Some(&nvenc_tick()), &[], &[]);
        let rows = process_rows(usage, |pid| match pid {
            DWM => "dwm.exe".to_owned(),
            _ => "ffmpeg.exe".to_owned(),
        });
        let mut rtx: Vec<_> = rows[&RTX]
            .iter()
            .map(|p| (p.pid, p.name.as_str(), p.load_percent))
            .collect();
        rtx.sort_by_key(|r| r.0);
        assert_eq!(
            rtx,
            [
                (DWM, "dwm.exe", Some(0.0)),
                (FFMPEG, "ffmpeg.exe", Some(73.7))
            ]
        );
        assert_eq!(rows[&RADEON].len(), 1);
    }

    #[test]
    fn layer_without_counters_publishes_an_empty_table() {
        let table = GpuProcessTable::new();
        table.set_devices(vec![("gpu/x".to_owned(), RTX)]);
        table.publish(process_rows(
            process_usage(Some(&nvenc_tick()), &[], &[]),
            |_| String::new(),
        ));
        assert!(!table.processes("gpu/x").is_empty());
        let mut layer = PdhLayer::new(table.clone());
        assert_eq!(layer.sample(), Ok(vec![]));
        assert!(table.processes("gpu/x").is_empty());
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn reads_engine_load_and_memory_on_this_machine() {
        use GpuField::*;
        let adapters = super::super::enumerate::enumerate().expect("enumerate");
        let rtx = adapters
            .iter()
            .position(|a| a.vendor_id == 0x10DE)
            .expect("NVIDIA adapter");
        let radeon = adapters
            .iter()
            .position(|a| a.vendor_id == 0x1002)
            .expect("AMD adapter");
        let mut layer = PdhLayer::default();
        assert_eq!(layer.source(), Source::Pdh);
        let supported = layer.attach(&adapters);
        assert_eq!(supported.len(), adapters.len());
        for field in [
            LoadCore,
            Load3d,
            LoadCopy,
            LoadVideoDecode,
            LoadVideoEncode,
            MemoryDedicatedUsed,
            MemorySharedUsed,
        ] {
            assert!(supported[rtx].contains(&field), "RTX lacks {field:?}");
        }
        // Declared even when the iGPU is idle at attach.
        for field in [LoadCore, MemoryDedicatedUsed, MemorySharedUsed] {
            assert!(supported[radeon].contains(&field), "Radeon lacks {field:?}");
        }

        let first = layer.sample().expect("first sample");
        assert!(
            first
                .iter()
                .all(|r| LOAD_FIELDS.iter().all(|f| !r.contains_key(f))),
            "{first:?}"
        );
        let used = first[rtx][&MemoryDedicatedUsed];
        assert!(
            used > 0.0 && used <= adapters[rtx].dedicated_bytes as f64,
            "dedicated used {used}"
        );

        std::thread::sleep(std::time::Duration::from_millis(1_100));
        let second = layer.sample().expect("second sample");
        let load = second[rtx][&LoadCore];
        assert!((0.0..=100.0).contains(&load), "load {load}");
        assert_eq!(
            second[rtx].keys().copied().collect::<BTreeSet<_>>(),
            supported[rtx]
        );
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn publishes_per_process_rows_on_this_machine() {
        let adapters = super::super::enumerate::enumerate().expect("enumerate");
        let rtx = adapters
            .iter()
            .find(|a| a.vendor_id == 0x10DE)
            .expect("NVIDIA adapter");
        let table = GpuProcessTable::new();
        table.set_devices(vec![("rtx".to_owned(), rtx.luid)]);
        let mut layer = PdhLayer::new(table.clone());
        layer.attach(&adapters);

        layer.sample().expect("first sample");
        let first = table.processes("rtx");
        assert!(!first.is_empty(), "the desktop always uses the dGPU");
        assert!(
            first.iter().all(|p| p.load_percent.is_none()),
            "no load on the first tick after attach"
        );
        let dwm = first
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case("dwm.exe"))
            .expect("dwm.exe is named although OpenProcess fails for it");
        assert!(dwm.dedicated_bytes.is_some_and(|b| b > 0), "{dwm:?}");

        std::thread::sleep(std::time::Duration::from_millis(1_100));
        layer.sample().expect("second sample");
        let second = table.processes("rtx");
        assert!(second.len() <= 20);
        for p in &second {
            println!(
                "{:>6} {:<28} load {:?} {:?} ded {:?} shr {:?}",
                p.pid, p.name, p.load_percent, p.engine, p.dedicated_bytes, p.shared_bytes
            );
            if let Some(load) = p.load_percent {
                assert!((0.0..=100.0).contains(&load), "{p:?}");
                assert_eq!(p.engine.is_some(), load > 0.0, "{p:?}");
            }
        }
        assert!(second.iter().any(|p| p.load_percent.is_some()));
    }
}
