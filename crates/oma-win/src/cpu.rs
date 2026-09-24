//! CPU load and effective clock from PDH "Processor Information" counters.

use std::collections::HashMap;

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use windows::core::w;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

use crate::pdh::{Counter, Query};

const DEVICE_ID: &str = "cpu/0";
const UTILITY: &str = r"\Processor Information(*)\% Processor Utility";
const TIME: &str = r"\Processor Information(*)\% Processor Time";
const PERFORMANCE: &str = r"\Processor Information(_Total)\% Processor Performance";
const FREQUENCY: &str = r"\Processor Information(_Total)\Processor Frequency";
const TOTAL_INSTANCE: &str = "_Total";

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
        let performance = query.add_english(PERFORMANCE).ok();
        let frequency = query.add_english(FREQUENCY).ok();
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
        let counters = self.counters.as_mut().ok_or(ProviderError::Rediscover)?;
        counters.query.collect()?;
        let utility = counters.query.array(counters.utility)?;
        if !utility.is_empty() {
            let seen = utility
                .iter()
                .filter(|(name, _)| parse_instance(name).is_some())
                .count();
            if seen != self.processors.len() {
                return Err(ProviderError::Rediscover);
            }
        }
        let by_instance: HashMap<&str, f64> =
            utility.iter().map(|(n, v)| (n.as_str(), *v)).collect();

        let mut values = Vec::with_capacity(self.processors.len() + 2);
        values.push(by_instance.get(TOTAL_INSTANCE).copied().and_then(load_pct));
        values.extend(self.processors.iter().map(|p| {
            by_instance
                .get(p.instance().as_str())
                .copied()
                .and_then(load_pct)
        }));
        let clock = match (
            counters.frequency.and_then(|c| counters.query.value(c)),
            counters.performance.and_then(|c| counters.query.value(c)),
        ) {
            (Some(nominal), Some(performance)) => Some(effective_clock_mhz(nominal, performance)),
            _ => None,
        };
        values.push(clock);
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn load_is_capped_at_100() {
        assert_eq!(load_pct(104.0), Some(100.0));
        assert_eq!(load_pct(-1.0), None);
        assert_eq!(load_pct(f64::NAN), None);
    }
}
