//! GPU engine load and adapter memory from the PDH "GPU Engine" and
//! "GPU Adapter Memory" counters, aggregated the way Task Manager does.

use std::collections::{BTreeMap, BTreeSet};

use oma_core::model::Source;
use oma_core::provider::ProviderError;

use super::adapter::Adapter;
use super::field::GpuField;
use super::layer::{GpuLayer, Readings};
use crate::pdh::{Counter, PdhError, Query};

const ENGINE: &str = r"\GPU Engine(*)\Utilization Percentage";
const DEDICATED: &str = r"\GPU Adapter Memory(*)\Dedicated Usage";
const SHARED: &str = r"\GPU Adapter Memory(*)\Shared Usage";

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
    pid.parse::<u32>().ok()?;
    let (luid, rest) = parse_luid(rest)?;
    let (phys, engine) = rest.strip_prefix("_phys_")?.split_once("_eng_")?;
    phys.parse::<u32>().ok()?;
    Some(EngineInstance {
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

struct Counters {
    query: Query,
    engine: Option<Counter>,
    dedicated: Option<Counter>,
    shared: Option<Counter>,
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
        Ok(Self {
            query,
            engine,
            dedicated,
            shared,
        })
    }

    fn instances(&self, counter: Option<Counter>) -> Result<Vec<String>, PdhError> {
        counter.map_or(Ok(Vec::new()), |c| self.query.instances(c))
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
}

impl PdhLayer {
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
        let Some(counters) = self.counters.as_mut() else {
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
        Ok(self
            .adapters
            .iter()
            .map(|(luid, supported)| {
                adapter_readings(engines.as_deref(), &dedicated, &shared, *luid, supported)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RTX: u64 = 0x17DB6;
    const RADEON: u64 = 0x1A331;
    const BASIC_RENDER: u64 = 0x1A2C6;

    fn engine(luid: u64, engine: u32, engtype: &str) -> EngineInstance {
        EngineInstance {
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
            Some(engine(RTX, 0, "3D"))
        );
        assert_eq!(
            parse_engine("pid_6860_luid_0x00000000_0x0001A331_phys_0_eng_10_engtype_Video Codec 0"),
            Some(engine(RADEON, 10, "Video Codec 0"))
        );
        assert_eq!(
            parse_engine("pid_4_luid_0x00000000_0x00017DB6_phys_0_eng_14_engtype_Security_1"),
            Some(engine(RTX, 14, "Security_1"))
        );
        assert_eq!(
            parse_engine("pid_4_luid_0x00000001_0x00000002_phys_1_eng_3_engtype_Copy"),
            Some(engine(0x1_0000_0002, 3, "Copy"))
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
}
