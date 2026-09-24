//! Physical disk throughput and activity (PDH) plus volume usage.

use std::collections::HashMap;

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

use crate::pdh::{Counter, Query};
use crate::storage_identity::{disk_identity, volume_identity};

const READ: &str = r"\PhysicalDisk(*)\Disk Read Bytes/sec";
const WRITE: &str = r"\PhysicalDisk(*)\Disk Write Bytes/sec";
const IDLE: &str = r"\PhysicalDisk(*)\% Idle Time";

/// A "PhysicalDisk" instance such as "2 C: D:" (disk 2 holding C: and D:).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiskInstance {
    pub instance: String,
    pub index: u32,
    pub volumes: Vec<String>,
}

/// `None` for "_Total".
pub(crate) fn parse_disk_instance(name: &str) -> Option<DiskInstance> {
    let mut parts = name.split_whitespace();
    let index = parts.next()?.parse().ok()?;
    let volumes = parts
        .filter(|p| p.len() == 2 && p.ends_with(':') && p.as_bytes()[0].is_ascii_alphabetic())
        .map(|p| p.to_ascii_uppercase())
        .collect();
    Some(DiskInstance {
        instance: name.to_owned(),
        index,
        volumes,
    })
}

pub(crate) fn disk_instances(names: &[String]) -> Vec<DiskInstance> {
    let mut disks: Vec<_> = names
        .iter()
        .filter_map(|n| parse_disk_instance(n))
        .collect();
    disks.sort_by_key(|d| d.index);
    disks
}

pub(crate) fn disks_changed(known: &[DiskInstance], names: &[String]) -> bool {
    disk_instances(names) != known
}

/// True when a volume's identity no longer matches the one recorded at discovery
/// (including when the volume no longer resolves at all): the letter was reused
/// by different hardware or a recreated partition, so history must not carry over.
pub(crate) fn volume_identity_changed(recorded: &str, current: Option<&str>) -> bool {
    current != Some(recorded)
}

pub(crate) fn disk_name(disk: &DiskInstance) -> String {
    if disk.volumes.is_empty() {
        format!("Disk {}", disk.index)
    } else {
        format!("Disk {} ({})", disk.index, disk.volumes.join(", "))
    }
}

pub(crate) fn active_pct(idle: f64) -> Option<f64> {
    idle.is_finite().then(|| (100.0 - idle).clamp(0.0, 100.0))
}

pub(crate) fn used_pct(total: u64, free: u64) -> Option<f64> {
    (total > 0).then(|| total.saturating_sub(free) as f64 * 100.0 / total as f64)
}

/// `(total, free)` bytes of a volume such as "C:"; `None` if unavailable.
fn volume_space(volume: &str) -> Option<(u64, u64)> {
    let root = HSTRING::from(format!("{volume}\\"));
    let (mut total, mut free) = (0u64, 0u64);
    // SAFETY: valid root path and out-pointers.
    unsafe { GetDiskFreeSpaceExW(&root, None, Some(&mut total), Some(&mut free)) }.ok()?;
    Some((total, free))
}

struct Counters {
    query: Query,
    read: Counter,
    write: Counter,
    idle: Counter,
}

#[derive(Default)]
pub struct StorageProvider {
    counters: Option<Counters>,
    disks: Vec<DiskInstance>,
    disk_ids: HashMap<u32, String>,
    volume_ids: HashMap<String, String>,
}

impl Provider for StorageProvider {
    fn name(&self) -> &'static str {
        "storage"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        let mut query = Query::open()?;
        let read = query.add_english(READ)?;
        let write = query.add_english(WRITE)?;
        let idle = query.add_english(IDLE)?;
        query.collect()?;
        let disks = disk_instances(&query.instances(read)?);

        // Resolve stable identities only during discovery; never persist PDH indices.
        let mut disk_ids: HashMap<u32, String> = disks
            .iter()
            .filter_map(|d| disk_identity(d.index).map(|id| (d.index, id)))
            .collect();
        let mut counts = HashMap::<String, usize>::new();
        for id in disk_ids.values() {
            *counts.entry(id.clone()).or_default() += 1;
        }
        disk_ids.retain(|_, id| counts[id] == 1); // Ambiguous serials must not merge disks.
        let volume_ids: HashMap<String, String> = disks
            .iter()
            .flat_map(|d| &d.volumes)
            .filter_map(|v| volume_identity(v).map(|id| (v.clone(), id)))
            .collect();
        let mut devices = Vec::new();
        let mut sensors = Vec::new();
        for disk in &disks {
            let Some(id) = disk_ids.get(&disk.index).cloned() else {
                tracing::warn!(
                    index = disk.index,
                    "disk has no unique persistent identity; omitted"
                );
                continue;
            };
            devices.push(Device {
                id: id.clone(),
                kind: DeviceKind::Storage,
                name: disk_name(disk),
                vendor: None,
                properties: Default::default(),
            });
            sensors.push(Sensor::new(
                &id,
                SensorKind::Throughput,
                "read",
                Unit::BytesPerSecond,
                Label::new("storage.read"),
                Source::Pdh,
            ));
            sensors.push(Sensor::new(
                &id,
                SensorKind::Throughput,
                "write",
                Unit::BytesPerSecond,
                Label::new("storage.write"),
                Source::Pdh,
            ));
            sensors.push(Sensor::new(
                &id,
                SensorKind::Load,
                "active",
                Unit::Percent,
                Label::new("storage.active"),
                Source::Pdh,
            ));
            for volume in &disk.volumes {
                let Some(volume_id) = volume_ids.get(volume) else {
                    continue;
                };
                sensors.push(Sensor::new(
                    &id,
                    SensorKind::Percent,
                    &format!("volume-{}", volume_id),
                    Unit::Percent,
                    Label::with_arg("storage.volumeUsed", volume.clone()),
                    Source::Win32,
                ));
                sensors.push(Sensor::new(
                    &id,
                    SensorKind::Data,
                    &format!("volume-{}-free", volume_id),
                    Unit::Bytes,
                    Label::with_arg("storage.volumeFree", volume.clone()),
                    Source::Win32,
                ));
            }
        }
        self.counters = Some(Counters {
            query,
            read,
            write,
            idle,
        });
        self.disks = disks;
        self.disk_ids = disk_ids;
        self.volume_ids = volume_ids;
        Ok(Inventory { devices, sensors })
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let counters = self.counters.as_mut().ok_or(ProviderError::Rediscover)?;
        counters.query.collect()?;
        let read = counters.query.array(counters.read)?;
        // Raw instances distinguish a missing disk from rate-counter warm-up.
        if disks_changed(&self.disks, &counters.query.instances(counters.read)?) {
            return Err(ProviderError::Rediscover);
        }
        let read: HashMap<String, f64> = read.into_iter().collect();
        let write: HashMap<String, f64> =
            counters.query.array(counters.write)?.into_iter().collect();
        let idle: HashMap<String, f64> = counters.query.array(counters.idle)?.into_iter().collect();
        let finite =
            |map: &HashMap<String, f64>, key: &str| map.get(key).copied().filter(|v| v.is_finite());

        let mut values = Vec::new();
        for disk in &self.disks {
            if !self.disk_ids.contains_key(&disk.index) {
                continue;
            }
            values.push(finite(&read, &disk.instance));
            values.push(finite(&write, &disk.instance));
            values.push(finite(&idle, &disk.instance).and_then(active_pct));
            for volume in &disk.volumes {
                let Some(recorded_id) = self.volume_ids.get(volume) else {
                    continue;
                };
                // The letter alone cannot tell a swapped disk or recreated partition
                // from the one seen at discovery; the volume GUID can.
                if volume_identity_changed(recorded_id, volume_identity(volume).as_deref()) {
                    return Err(ProviderError::Rediscover);
                }
                match volume_space(volume) {
                    Some((total, free)) => {
                        values.push(used_pct(total, free));
                        values.push(Some(free as f64));
                    }
                    None => values.extend([None, None]),
                }
            }
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_disk_with_one_volume() {
        let d = parse_disk_instance("2 C:").unwrap();
        assert_eq!(d.index, 2);
        assert_eq!(d.volumes, vec!["C:".to_string()]);
    }

    #[test]
    fn parses_disk_with_several_or_no_volumes() {
        assert_eq!(
            parse_disk_instance("0 C: D:").unwrap().volumes,
            vec!["C:", "D:"]
        );
        assert!(parse_disk_instance("1").unwrap().volumes.is_empty());
        assert_eq!(parse_disk_instance("3 e:").unwrap().volumes, vec!["E:"]);
    }

    #[test]
    fn total_instance_is_not_a_disk() {
        assert_eq!(parse_disk_instance("_Total"), None);
    }

    #[test]
    fn disks_are_sorted_by_index() {
        let disks = disk_instances(&["2 C:".into(), "_Total".into(), "0 D:".into()]);
        assert_eq!(
            disks.iter().map(|d| d.index).collect::<Vec<_>>(),
            vec![0, 2]
        );
    }

    #[test]
    fn detects_disk_set_changes() {
        let known = disk_instances(&["0 C:".into()]);
        assert!(!disks_changed(&known, &["0 C:".into(), "_Total".into()]));
        assert!(disks_changed(
            &known,
            &["0 C:".into(), "1 E:".into(), "_Total".into()]
        ));
    }

    #[test]
    fn active_time_is_the_complement_of_idle() {
        assert!((active_pct(99.9).unwrap() - 0.1).abs() < 1e-9);
        assert_eq!(active_pct(120.0), Some(0.0));
        assert_eq!(active_pct(f64::NAN), None);
    }

    #[test]
    fn volume_usage() {
        assert_eq!(used_pct(200, 50), Some(75.0));
        assert_eq!(used_pct(0, 0), None);
    }

    #[test]
    fn volume_identity_change_forces_rediscover() {
        assert!(!volume_identity_changed("guid-a", Some("guid-a")));
        assert!(volume_identity_changed("guid-a", Some("guid-b")));
        assert!(volume_identity_changed("guid-a", None));
    }

    #[test]
    fn disk_names_list_volumes() {
        assert_eq!(
            disk_name(&parse_disk_instance("0 C: D:").unwrap()),
            "Disk 0 (C:, D:)"
        );
        assert_eq!(disk_name(&parse_disk_instance("1").unwrap()), "Disk 1");
    }
}
