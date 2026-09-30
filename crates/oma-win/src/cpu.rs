//! CPU load and effective clock from PDH "Processor Information" counters.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use windows::core::w;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

use crate::pdh::{Counter, PdhError, Query, PDH_CALC_NEGATIVE_DENOMINATOR};

const DEVICE_ID: &str = "cpu/0";
const UTILITY: &str = r"\Processor Information(*)\% Processor Utility";
const TIME: &str = r"\Processor Information(*)\% Processor Time";
const PERFORMANCE: &str = r"\Processor Information(_Total)\% Processor Performance";
const FREQUENCY: &str = r"\Processor Information(_Total)\Processor Frequency";
const TOTAL_INSTANCE: &str = "_Total";
/// A poll with a PDH problem is logged at most this often, per limiter.
const LOG_INTERVAL: Duration = Duration::from_secs(3_600);
/// Consecutive polls that may repeat the last valid value through
/// `PDH_CALC_NEGATIVE_DENOMINATOR`; past them the value is absent, so a
/// broken counter never shows a stale number for long.
pub(crate) const MAX_HELD_POLLS: u32 = 3;

/// A "Processor Information" instance such as "0,7" (group 0, processor 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct LogicalProcessor {
    pub group: u16,
    pub number: u16,
}

impl LogicalProcessor {
    pub(crate) fn instance(self) -> String {
        format!("{},{}", self.group, self.number)
    }
}

/// Parses "group,number"; `None` for "_Total" and per-group "N,_Total".
pub(crate) fn parse_instance(name: &str) -> Option<LogicalProcessor> {
    let (group, number) = name.split_once(',')?;
    Some(LogicalProcessor {
        group: group.trim().parse().ok()?,
        number: number.trim().parse().ok()?,
    })
}

fn add_load_counter<T, E>(mut add: impl FnMut(&str) -> Result<T, E>) -> Result<T, E> {
    add(UTILITY).or_else(|_| add(TIME))
}

/// Task Manager's estimated clock: nominal frequency × % performance.
pub(crate) fn effective_clock_mhz(nominal_mhz: f64, performance_pct: f64) -> f64 {
    nominal_mhz * performance_pct / 100.0
}

/// The effective clock from its two counters: `Ok(None)` when one of them
/// could not be added (`discover` already warned), `Err` with the PDH status
/// when PDH has no value for this poll. `% Processor Performance` (_Total)
/// does that now and then for one poll, with `PDH_CALC_NEGATIVE_DENOMINATOR`,
/// because its raw base goes backwards (seen live on the development machine).
pub(crate) fn effective_clock(
    frequency: Option<Result<f64, u32>>,
    performance: Option<Result<f64, u32>>,
) -> Result<Option<f64>, u32> {
    match (frequency, performance) {
        (Some(nominal), Some(performance)) => Ok(Some(effective_clock_mhz(nominal?, performance?))),
        _ => Ok(None),
    }
}

/// The last valid reading, repeated through at most [`MAX_HELD_POLLS`]
/// consecutive transient PDH failures.
#[derive(Debug)]
pub(crate) struct Hold<T> {
    last: Option<T>,
    held: u32,
}

impl<T> Default for Hold<T> {
    fn default() -> Self {
        Self {
            last: None,
            held: 0,
        }
    }
}

impl<T: Clone> Hold<T> {
    /// A valid reading: remembered, and the streak of held polls ends.
    pub(crate) fn fresh(&mut self, value: T) -> T {
        self.last = Some(value.clone());
        self.held = 0;
        value
    }

    /// A transient failure: the last valid reading while the streak is
    /// within the bound, then nothing.
    pub(crate) fn transient(&mut self) -> Option<T> {
        if self.held >= MAX_HELD_POLLS {
            self.last = None;
        }
        self.held = self.held.saturating_add(1);
        self.last.clone()
    }

    /// Any other outcome: nothing is repeated any more.
    pub(crate) fn clear(&mut self) {
        self.last = None;
        self.held = 0;
    }
}

/// The clock of this poll and the PDH status to log. Only
/// `PDH_CALC_NEGATIVE_DENOMINATOR` repeats the last valid clock (see
/// [`Hold`]); any other status and a missing counter give no value.
pub(crate) fn clock_value(
    reading: Result<Option<f64>, u32>,
    hold: &mut Hold<f64>,
) -> (Option<f64>, Option<u32>) {
    match reading {
        Ok(Some(mhz)) => (Some(hold.fresh(mhz)), None),
        Ok(None) => {
            hold.clear();
            (None, None)
        }
        Err(PDH_CALC_NEGATIVE_DENOMINATOR) => {
            (hold.transient(), Some(PDH_CALC_NEGATIVE_DENOMINATOR))
        }
        Err(status) => {
            hold.clear();
            (None, Some(status))
        }
    }
}

/// The loads of this poll, `_Total` first and then each processor, and the
/// PDH status to log. When the utility base wraps (every 2^32 / ~625 kHz, about
/// 1 h 54 min 32 s, on the development machine) PDH answers the whole array
/// with `PDH_CALC_NEGATIVE_DENOMINATOR` for one poll: each instance repeats
/// its value of the last valid poll (see [`Hold`]) instead of failing the
/// provider. Any other error still fails it; a changed processor set still
/// asks for a rediscovery.
pub(crate) fn load_values(
    utility: Result<Vec<(String, f64)>, PdhError>,
    processors: &[LogicalProcessor],
    hold: &mut Hold<Vec<Option<f64>>>,
) -> Result<(Vec<Option<f64>>, Option<u32>), ProviderError> {
    let utility = match utility {
        Ok(utility) => utility,
        Err(e) if e.status == PDH_CALC_NEGATIVE_DENOMINATOR => {
            let loads = hold
                .transient()
                .unwrap_or_else(|| vec![None; processors.len() + 1]);
            return Ok((loads, Some(e.status)));
        }
        Err(e) => {
            hold.clear();
            return Err(e.into());
        }
    };
    if !utility.is_empty() {
        let seen = utility
            .iter()
            .filter(|(name, _)| parse_instance(name).is_some())
            .count();
        if seen != processors.len() {
            hold.clear();
            return Err(ProviderError::Rediscover);
        }
    }
    let by_instance: HashMap<&str, f64> = utility.iter().map(|(n, v)| (n.as_str(), *v)).collect();
    let mut loads = Vec::with_capacity(processors.len() + 2);
    loads.push(by_instance.get(TOTAL_INSTANCE).copied().and_then(load_pct));
    loads.extend(processors.iter().map(|p| {
        by_instance
            .get(p.instance().as_str())
            .copied()
            .and_then(load_pct)
    }));
    Ok((hold.fresh(loads), None))
}

/// At most one line per [`LOG_INTERVAL`], with the count of the events it
/// kept quiet since the last one.
#[derive(Debug, Default)]
pub(crate) struct HourlyLog {
    last: Option<Instant>,
    suppressed: u64,
}

impl HourlyLog {
    /// `Some(suppressed)` when a line is due at `now`; otherwise the event is
    /// counted as suppressed.
    pub(crate) fn due(&mut self, now: Instant) -> Option<u64> {
        if self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= LOG_INTERVAL)
        {
            self.last = Some(now);
            Some(std::mem::take(&mut self.suppressed))
        } else {
            self.suppressed += 1;
            None
        }
    }
}

/// Processor Utility exceeds 100 % while boosting; Task Manager caps it and so do we.
pub(crate) fn load_pct(utility: f64) -> Option<f64> {
    (utility.is_finite() && utility >= 0.0).then(|| utility.min(100.0))
}

fn cpu_name() -> String {
    let mut buffer = [0u16; 256];
    let mut bytes = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: `buffer` and `bytes` describe writable memory of that size.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0"),
            w!("ProcessorNameString"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status != ERROR_SUCCESS {
        return "CPU".to_owned();
    }
    let chars = (bytes as usize / 2).saturating_sub(1); // drop the terminating NUL
    let name = String::from_utf16_lossy(&buffer[..chars]).trim().to_owned();
    if name.is_empty() {
        "CPU".to_owned()
    } else {
        name
    }
}

struct Counters {
    query: Query,
    utility: Counter,
    performance: Option<Counter>,
    frequency: Option<Counter>,
}

#[derive(Default)]
pub struct CpuProvider {
    counters: Option<Counters>,
    processors: Vec<LogicalProcessor>,
    /// Set by `discover`; consumed by the next `poll`. See `take_fresh`.
    fresh: bool,
    /// The last valid loads and clock, repeated through a transient PDH
    /// calculation error (see `load_values`, `clock_value`).
    load_hold: Hold<Vec<Option<f64>>>,
    clock_hold: Hold<f64>,
    load_log: HourlyLog,
    clock_log: HourlyLog,
}

/// `true` only for the first call after a discover: PDH rate counters were
/// just added, so the collect a few milliseconds later has too short an
/// interval to yield a meaningful rate (noisy load/clock), mirroring the
/// network provider's first-sample rule. Resets the flag as a side effect.
fn take_fresh(fresh: &mut bool) -> bool {
    std::mem::replace(fresh, false)
}

impl CpuProvider {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Provider for CpuProvider {
    fn name(&self) -> &'static str {
        "cpu"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        let mut query = Query::open()?;
        let utility = add_load_counter(|path| query.add_english(path))?;
        let performance = query.add_english(PERFORMANCE).map_err(|e| {
            tracing::warn!(error = %e, "failed to add processor performance counter; effective clock will be unavailable");
        }).ok();
        let frequency = query.add_english(FREQUENCY).map_err(|e| {
            tracing::warn!(error = %e, "failed to add processor frequency counter; effective clock will be unavailable");
        }).ok();
        query.collect()?;
        let mut processors: Vec<_> = query
            .instances(utility)?
            .iter()
            .filter_map(|n| parse_instance(n))
            .collect();
        processors.sort_unstable();

        let mut sensors = vec![Sensor::new(
            DEVICE_ID,
            SensorKind::Load,
            "total",
            Unit::Percent,
            Label::new("cpu.load.total"),
            Source::Pdh,
        )];
        sensors.extend(processors.iter().enumerate().map(|(index, p)| {
            Sensor::new(
                DEVICE_ID,
                SensorKind::Load,
                &format!("thread-{}-{}", p.group, p.number),
                Unit::Percent,
                Label::with_arg("cpu.load.thread", index.to_string()),
                Source::Pdh,
            )
        }));
        sensors.push(Sensor::new(
            DEVICE_ID,
            SensorKind::Clock,
            "effective",
            Unit::Megahertz,
            Label::new("cpu.clock.effective"),
            Source::Pdh,
        ));

        self.counters = Some(Counters {
            query,
            utility,
            performance,
            frequency,
        });
        self.processors = processors;
        self.fresh = true;
        self.load_hold.clear();
        self.clock_hold.clear();
        Ok(Inventory {
            devices: vec![Device {
                id: DEVICE_ID.to_owned(),
                kind: DeviceKind::Cpu,
                name: cpu_name(),
                vendor: None,
                properties: Default::default(),
            }],
            sensors,
        })
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let fresh = take_fresh(&mut self.fresh);
        let counters = self.counters.as_mut().ok_or(ProviderError::Rediscover)?;
        counters.query.collect()?;
        if fresh {
            // Prime the PDH rate counters but report no value yet.
            return Ok(vec![None; self.processors.len() + 2]);
        }
        let (mut values, load_status) = load_values(
            counters.query.array(counters.utility),
            &self.processors,
            &mut self.load_hold,
        )?;
        if let Some(status) = load_status {
            if let Some(suppressed) = self.load_log.due(Instant::now()) {
                tracing::info!(
                    status = format_args!("{status:#010x}"),
                    held = values.iter().any(Option::is_some),
                    suppressed,
                    "PDH has no CPU load for this poll"
                );
            }
        }
        let (clock, clock_status) = clock_value(
            effective_clock(
                counters.frequency.map(|c| counters.query.value(c)),
                counters.performance.map(|c| counters.query.value(c)),
            ),
            &mut self.clock_hold,
        );
        if let Some(status) = clock_status {
            if let Some(suppressed) = self.clock_log.due(Instant::now()) {
                tracing::info!(
                    status = format_args!("{status:#010x}"),
                    held = clock.is_some(),
                    suppressed,
                    "PDH has no effective clock for this poll"
                );
            }
        }
        values.push(clock);
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdh::{PdhError, PDH_CALC_NEGATIVE_DENOMINATOR, PDH_INVALID_DATA};

    fn lp(group: u16, number: u16) -> LogicalProcessor {
        LogicalProcessor { group, number }
    }

    #[test]
    fn parses_group_and_number() {
        assert_eq!(parse_instance("0,7"), Some(lp(0, 7)));
        assert_eq!(parse_instance("1,63"), Some(lp(1, 63)));
    }

    #[test]
    fn ignores_total_instances() {
        assert_eq!(parse_instance("_Total"), None);
        assert_eq!(parse_instance("0,_Total"), None);
    }

    #[test]
    fn processors_sort_by_group_then_number() {
        let mut v: Vec<_> = ["1,0", "0,63", "0,2"]
            .iter()
            .filter_map(|n| parse_instance(n))
            .collect();
        v.sort_unstable();
        assert_eq!(v, vec![lp(0, 2), lp(0, 63), lp(1, 0)]);
    }

    #[test]
    fn instance_name_round_trips() {
        assert_eq!(lp(1, 5).instance(), "1,5");
    }

    #[test]
    fn utility_falls_back_to_processor_time() {
        let mut paths = Vec::new();
        let value = add_load_counter(|path| {
            paths.push(path.to_owned());
            if path == UTILITY {
                Err("missing utility")
            } else {
                Ok(42)
            }
        });
        assert_eq!(value, Ok(42));
        assert_eq!(paths, vec![UTILITY, TIME]);
        assert!(add_load_counter::<(), _>(|_| Err("missing both")).is_err());
    }

    #[test]
    fn effective_clock_scales_nominal_frequency() {
        let mhz = effective_clock_mhz(4201.0, 104.35);
        assert!((mhz - 4383.74).abs() < 0.01, "{mhz}");
    }

    #[test]
    fn clock_is_missing_without_its_counters_and_reports_a_pdh_status() {
        assert_eq!(
            effective_clock(Some(Ok(4201.0)), Some(Ok(100.0))),
            Ok(Some(4201.0))
        );
        // A counter that could not be added was already reported by `discover`.
        assert_eq!(effective_clock(None, Some(Ok(100.0))), Ok(None));
        assert_eq!(effective_clock(Some(Ok(4201.0)), None), Ok(None));
        // `% Processor Performance` (_Total) whose raw base went backwards (seen live).
        assert_eq!(
            effective_clock(Some(Ok(4201.0)), Some(Err(PDH_CALC_NEGATIVE_DENOMINATOR))),
            Err(PDH_CALC_NEGATIVE_DENOMINATOR)
        );
        assert_eq!(
            effective_clock(Some(Err(PDH_INVALID_DATA)), Some(Ok(100.0))),
            Err(PDH_INVALID_DATA)
        );
    }

    #[test]
    fn hold_repeats_the_last_valid_value_for_at_most_three_polls() {
        let mut hold = Hold::default();
        // Nothing valid yet: nothing to repeat.
        assert_eq!(hold.transient(), None);
        assert_eq!(hold.fresh(1.0), 1.0);
        for _ in 0..MAX_HELD_POLLS {
            assert_eq!(hold.transient(), Some(1.0));
        }
        assert_eq!(hold.transient(), None, "past the bound");
        assert_eq!(hold.transient(), None);
        // A valid value ends the streak.
        assert_eq!(hold.fresh(2.0), 2.0);
        assert_eq!(hold.transient(), Some(2.0));
        hold.clear();
        assert_eq!(hold.transient(), None, "cleared");
    }

    #[test]
    fn clock_repeats_through_transient_calc_errors_only() {
        let mut hold = Hold::default();
        let neg = Err(PDH_CALC_NEGATIVE_DENOMINATOR);
        type Reading = Result<Option<f64>, u32>;
        type Clock = (Option<f64>, Option<u32>);
        let polls: Vec<(Reading, Clock)> = vec![
            // The first polls have no valid value yet.
            (neg, (None, Some(PDH_CALC_NEGATIVE_DENOMINATOR))),
            (Ok(Some(4500.0)), (Some(4500.0), None)),
            (neg, (Some(4500.0), Some(PDH_CALC_NEGATIVE_DENOMINATOR))),
            (neg, (Some(4500.0), Some(PDH_CALC_NEGATIVE_DENOMINATOR))),
            (neg, (Some(4500.0), Some(PDH_CALC_NEGATIVE_DENOMINATOR))),
            // A fourth failure in a row: the counter is broken, not glitching.
            (neg, (None, Some(PDH_CALC_NEGATIVE_DENOMINATOR))),
            (Ok(Some(4400.0)), (Some(4400.0), None)),
            // Any other status keeps today's behaviour and ends the hold.
            (Err(PDH_INVALID_DATA), (None, Some(PDH_INVALID_DATA))),
            (neg, (None, Some(PDH_CALC_NEGATIVE_DENOMINATOR))),
            (Ok(Some(4300.0)), (Some(4300.0), None)),
            // So does a missing counter.
            (Ok(None), (None, None)),
            (neg, (None, Some(PDH_CALC_NEGATIVE_DENOMINATOR))),
        ];
        for (i, (reading, expected)) in polls.into_iter().enumerate() {
            assert_eq!(clock_value(reading, &mut hold), expected, "poll {i}");
        }
    }

    fn utility(items: &[(&str, f64)]) -> Result<Vec<(String, f64)>, PdhError> {
        Ok(items.iter().map(|(n, v)| ((*n).to_owned(), *v)).collect())
    }

    fn array_error(status: u32) -> Result<Vec<(String, f64)>, PdhError> {
        Err(PdhError {
            call: "PdhGetFormattedCounterArrayW",
            status,
        })
    }

    #[test]
    fn utility_base_wrap_repeats_the_loads_instead_of_failing() {
        // The diagnostic log's `provider degraded provider="cpu" err=PdhGetFormattedCounterArrayW
        // failed with PDH status 0x800007d6` every 1 h 54 min 32 s: the 32-bit base of
        // `% Processor Utility` (about 625,400 a second) wraps and PDH answers the whole array
        // with PDH_CALC_NEGATIVE_DENOMINATOR for one poll.
        let processors = [lp(0, 0), lp(0, 1)];
        let mut hold = Hold::default();
        let mut poll =
            |result| load_values(result, &processors, &mut hold).map_err(|e| e.to_string());
        let before = vec![Some(20.0), Some(10.0), Some(30.0)];
        assert_eq!(
            poll(utility(&[
                ("_Total", 20.0),
                ("0,0", 10.0),
                ("0,1", 30.0),
                ("0,_Total", 20.0)
            ])),
            Ok((before.clone(), None))
        );
        for _ in 0..MAX_HELD_POLLS {
            assert_eq!(
                poll(array_error(PDH_CALC_NEGATIVE_DENOMINATOR)),
                Ok((before.clone(), Some(PDH_CALC_NEGATIVE_DENOMINATOR)))
            );
        }
        // A persistent failure ends up visible as absent values, still without failing.
        assert_eq!(
            poll(array_error(PDH_CALC_NEGATIVE_DENOMINATOR)),
            Ok((vec![None; 3], Some(PDH_CALC_NEGATIVE_DENOMINATOR)))
        );
        // Each instance repeats its own last value, an invalid one stays absent.
        let after = vec![Some(25.0), None, Some(40.0)];
        assert_eq!(
            poll(utility(&[
                ("_Total", 25.0),
                ("0,0", f64::NAN),
                ("0,1", 40.0)
            ])),
            Ok((after.clone(), None))
        );
        assert_eq!(
            poll(array_error(PDH_CALC_NEGATIVE_DENOMINATOR)),
            Ok((after, Some(PDH_CALC_NEGATIVE_DENOMINATOR)))
        );
    }

    #[test]
    fn other_array_errors_still_fail_and_end_the_hold() {
        let processors = [lp(0, 0)];
        let mut hold = Hold::default();
        let mut poll =
            |result| load_values(result, &processors, &mut hold).map_err(|e| e.to_string());
        assert_eq!(
            poll(utility(&[("_Total", 5.0), ("0,0", 5.0)])),
            Ok((vec![Some(5.0), Some(5.0)], None))
        );
        let failed = poll(array_error(0xC000_0BB8));
        assert!(
            failed.as_ref().is_err_and(|e| e.contains("0xc0000bb8")),
            "{failed:?}"
        );
        assert_eq!(
            poll(array_error(PDH_CALC_NEGATIVE_DENOMINATOR)),
            Ok((vec![None; 2], Some(PDH_CALC_NEGATIVE_DENOMINATOR)))
        );
        // A different processor count still asks for a rediscovery.
        assert!(poll(utility(&[("_Total", 5.0), ("0,0", 5.0), ("0,1", 5.0)])).is_err());
    }

    #[test]
    fn hourly_log_counts_what_it_suppressed() {
        let start = Instant::now();
        let mut log = HourlyLog::default();
        assert_eq!(log.due(start), Some(0));
        assert_eq!(log.due(start + Duration::from_secs(1)), None);
        assert_eq!(log.due(start + Duration::from_secs(3_599)), None);
        assert_eq!(log.due(start + LOG_INTERVAL), Some(2));
        assert_eq!(log.due(start + LOG_INTERVAL + Duration::from_secs(1)), None);
        assert_eq!(log.due(start + LOG_INTERVAL * 2), Some(1));
    }

    #[test]
    fn load_is_capped_at_100() {
        assert_eq!(load_pct(104.0), Some(100.0));
        assert_eq!(load_pct(-1.0), None);
        assert_eq!(load_pct(f64::NAN), None);
    }

    #[test]
    fn fresh_flag_is_consumed_by_the_first_poll_only() {
        let mut fresh = true;
        assert!(take_fresh(&mut fresh));
        assert!(!take_fresh(&mut fresh));
        assert!(!take_fresh(&mut fresh));
    }
}
