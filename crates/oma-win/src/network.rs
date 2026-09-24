//! Per-adapter network throughput and link speed from GetIfTable2.

use std::collections::HashMap;
use std::time::Instant;

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use oma_core::rate::CounterRate;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
const IF_TYPE_IEEE80211: u32 = 71;
const FLAG_HARDWARE_INTERFACE: u8 = 0x01;
const FLAG_FILTER_INTERFACE: u8 = 0x02;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InterfaceRow {
    pub guid: String,
    pub alias: String,
    pub if_type: u32,
    pub flags: u8,
    pub up: bool,
    pub in_octets: u64,
    pub out_octets: u64,
    pub link_bps: u64,
}

/// Connected physical Ethernet/Wi-Fi adapters; skips virtual adapters, NDIS
/// filter (LWF) duplicates and disconnected interfaces.
pub(crate) fn is_monitored(row: &InterfaceRow) -> bool {
    matches!(row.if_type, IF_TYPE_ETHERNET_CSMACD | IF_TYPE_IEEE80211)
        && row.flags & FLAG_HARDWARE_INTERFACE != 0
        && row.flags & FLAG_FILTER_INTERFACE == 0
        && row.up
}

/// Sorted GUIDs of the monitored adapters; a change means "rediscover".
pub(crate) fn monitored_guids(rows: &[InterfaceRow]) -> Vec<String> {
    let mut guids: Vec<String> = rows
        .iter()
        .filter(|r| is_monitored(r))
        .map(|r| r.guid.clone())
        .collect();
    guids.sort();
    guids
}

pub(crate) fn wide_to_string(wide: &[u16]) -> String {
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

fn read_interfaces() -> Result<Vec<InterfaceRow>, ProviderError> {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: valid out-pointer; the table is freed below with FreeMibTable.
    let status = unsafe { GetIfTable2(&mut table) };
    if status != ERROR_SUCCESS || table.is_null() {
        return Err(ProviderError::Failed(format!(
            "GetIfTable2 failed: {status:?}"
        )));
    }
    // SAFETY: on success `table` points to `NumEntries` rows until freed.
    let rows = unsafe {
        let t = &*table;
        std::slice::from_raw_parts(t.Table.as_ptr(), t.NumEntries as usize)
    };
    let result = rows
        .iter()
        .map(|r| InterfaceRow {
            guid: format!("{:?}", r.InterfaceGuid).to_ascii_lowercase(),
            alias: wide_to_string(&r.Alias),
            if_type: r.Type,
            flags: r.InterfaceAndOperStatusFlags._bitfield,
            up: r.OperStatus == IfOperStatusUp,
            in_octets: r.InOctets,
            out_octets: r.OutOctets,
            link_bps: r.ReceiveLinkSpeed,
        })
        .collect();
    // SAFETY: `table` came from GetIfTable2 and is not used afterwards.
    unsafe { FreeMibTable(table as *const _) };
    Ok(result)
}

struct Adapter {
    guid: String,
    down: CounterRate,
    up: CounterRate,
}

pub struct NetworkProvider {
    epoch: Instant,
    adapters: Vec<Adapter>,
    known: Vec<String>,
}

impl Default for NetworkProvider {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            adapters: Vec::new(),
            known: Vec::new(),
        }
    }
}

impl NetworkProvider {
    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

impl Provider for NetworkProvider {
    fn name(&self) -> &'static str {
        "network"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        let mut rows: Vec<InterfaceRow> = read_interfaces()?
            .into_iter()
            .filter(is_monitored)
            .collect();
        rows.sort_by(|a, b| a.alias.cmp(&b.alias).then_with(|| a.guid.cmp(&b.guid)));

        let mut devices = Vec::new();
        let mut sensors = Vec::new();
        let mut adapters = Vec::new();
        for row in &rows {
            let id = format!("network/{}", row.guid);
            devices.push(Device {
                id: id.clone(),
                kind: DeviceKind::Network,
                name: row.alias.clone(),
                vendor: None,
                properties: Default::default(),
            });
            sensors.push(Sensor::new(
                &id,
                SensorKind::Throughput,
                "down",
                Unit::BytesPerSecond,
                Label::new("network.down"),
                Source::IpHelper,
            ));
            sensors.push(Sensor::new(
                &id,
                SensorKind::Throughput,
                "up",
                Unit::BytesPerSecond,
                Label::new("network.up"),
                Source::IpHelper,
            ));
            sensors.push(Sensor::new(
                &id,
                SensorKind::Throughput,
                "link-speed",
                Unit::BitsPerSecond,
                Label::new("network.linkSpeed"),
                Source::IpHelper,
            ));
            // Fresh baselines: the first poll after a discover yields None for
            // down/up (no elapsed interval to compute a rate over yet), not a
            // spike computed over the few milliseconds between discover and poll.
            adapters.push(Adapter {
                guid: row.guid.clone(),
                down: CounterRate::new(),
                up: CounterRate::new(),
            });
        }
        self.known = monitored_guids(&rows);
        self.adapters = adapters;
        Ok(Inventory { devices, sensors })
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let rows = read_interfaces()?;
        if monitored_guids(&rows) != self.known {
            return Err(ProviderError::Rediscover);
        }
        let t = self.now_ms();
        let by_guid: HashMap<&str, &InterfaceRow> =
            rows.iter().map(|r| (r.guid.as_str(), r)).collect();
        let mut values = Vec::with_capacity(self.adapters.len() * 3);
        for adapter in &mut self.adapters {
            let row = by_guid
                .get(adapter.guid.as_str())
                .ok_or(ProviderError::Rediscover)?;
            values.push(adapter.down.update(row.in_octets, t));
            values.push(adapter.up.update(row.out_octets, t));
            values.push(Some(row.link_bps as f64));
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(guid: &str, if_type: u32, flags: u8, up: bool) -> InterfaceRow {
        InterfaceRow {
            guid: guid.into(),
            alias: guid.to_uppercase(),
            if_type,
            flags,
            up,
            in_octets: 0,
            out_octets: 0,
            link_bps: 1_000_000_000,
        }
    }

    #[test]
    fn monitors_connected_physical_ethernet_and_wifi() {
        assert!(is_monitored(&row("a", 6, 0b01, true)));
        assert!(is_monitored(&row("b", 71, 0b01, true)));
    }

    #[test]
    fn skips_virtual_filter_loopback_and_disconnected_interfaces() {
        assert!(!is_monitored(&row("virtual", 6, 0b00, true)));
        assert!(!is_monitored(&row("filter", 6, 0b11, true)));
        assert!(!is_monitored(&row("loopback", 24, 0b01, true)));
        assert!(!is_monitored(&row("down", 6, 0b01, false)));
    }

    #[test]
    fn detects_adapter_set_changes() {
        let rows = vec![row("a", 6, 1, true), row("b", 71, 1, false)];
        assert_eq!(monitored_guids(&rows), vec!["a".to_string()]);
        let rows_after_wifi_connects = vec![row("a", 6, 1, true), row("b", 71, 1, true)];
        assert_eq!(
            monitored_guids(&rows_after_wifi_connects),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn wide_strings_stop_at_nul() {
        let mut w = [0u16; 8];
        w[..3].copy_from_slice(&[b'W' as u16, b'i' as u16, b'-' as u16]);
        assert_eq!(wide_to_string(&w), "Wi-");
    }
}
