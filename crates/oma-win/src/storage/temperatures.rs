//! The temperatures of one disk, as read locally or imported from the service.

use super::*;

/// Temperature sensors of one disk: the driver indices declared at
/// discovery, their latest values (repeated between refreshes) and when they
/// were read.
#[derive(Default)]
pub(super) struct DiskTemperatures {
    pub(super) positions: Vec<usize>,
    pub(super) values: Vec<Option<f64>>,
    /// The last attempt; `None` for a disk never queried, due at once.
    pub(super) read_at: Option<Instant>,
    /// The last successful answer: the limits, and the sensors the next
    /// discovery declares without querying the disk again.
    pub(super) report: Option<TemperatureReport>,
    /// The values come from a read no poll has published yet.
    pub(super) unpublished: bool,
    /// The last main temperature taken from the service.
    pub(super) imported: Option<Imported>,
}

/// A main temperature taken from the service: which disk it was measured on
/// (its wire key) and the snapshot that carried it.
pub(super) struct Imported {
    pub(super) value: f64,
    pub(super) key: String,
    pub(super) snapshot: SnapshotId,
    /// No poll has published it yet.
    pub(super) unpublished: bool,
}

impl DiskTemperatures {
    /// The state after a read at discovery, as before the gate: the sensors
    /// are the ones this answer reports.
    pub(super) fn read(report: Option<TemperatureReport>, now: Instant) -> Self {
        let positions = report.as_ref().map(declared_positions).unwrap_or_default();
        Self {
            values: declared_values(report.as_ref(), &positions),
            positions,
            read_at: Some(now),
            report,
            unpublished: true,
            imported: None,
        }
    }

    /// Refreshes values and the attempt deadline, including failed/asleep reads.
    /// New driver indices require a schema rebuild, never a value-vector resize.
    pub(super) fn refresh(&mut self, report: Option<&TemperatureReport>, now: Instant) -> bool {
        self.values = declared_values(report, &self.positions);
        self.read_at = Some(now);
        self.unpublished = true;
        let Some(report) = report else {
            return false;
        };
        self.report = Some(report.clone());
        declared_positions(report)
            .iter()
            .any(|i| !self.positions.contains(i))
    }

    /// The refresh of a disk whose main temperature is the service's: only
    /// the additional sensors take the disk's answer.
    pub(super) fn refresh_additional(
        &mut self,
        report: Option<&TemperatureReport>,
        now: Instant,
    ) -> bool {
        let main = self.positions.iter().position(|&p| p == MAIN_POSITION);
        let kept = main.map(|slot| self.values[slot]);
        let rediscover = self.refresh(report, now);
        if let (Some(slot), Some(kept)) = (main, kept) {
            self.values[slot] = kept;
        }
        rediscover
    }

    /// The state a discovery that does not query the disk starts from: the
    /// sensors already declared keep their values, the indices the last
    /// answer revealed are declared with the values it carried, and a
    /// temperature taken from the service declares the main one.
    pub(super) fn redeclared(mut self) -> Self {
        let mut sensors: BTreeMap<usize, Option<f64>> =
            self.positions.iter().copied().zip(self.values).collect();
        if let Some(report) = &self.report {
            for position in declared_positions(report) {
                sensors
                    .entry(position)
                    .or_insert_with(|| report.sensors.get(&position).copied().flatten());
            }
        }
        if let Some(imported) = &self.imported {
            sensors.entry(MAIN_POSITION).or_insert(Some(imported.value));
        }
        (self.positions, self.values) = sensors.into_iter().unzip();
        self
    }

    /// Takes `temperature`, measured by the service on the disk with this
    /// `key` and carried by `snapshot`, as the main temperature. The local
    /// deadline is not touched. Returns whether the sensor is still to be
    /// declared, by a rediscovery.
    pub(super) fn import(
        &mut self,
        temperature: ServiceTemperature,
        key: &str,
        snapshot: SnapshotId,
    ) -> bool {
        let known = self
            .imported
            .as_ref()
            .is_some_and(|imported| imported.snapshot == snapshot);
        if !known {
            self.imported = Some(Imported {
                value: temperature.value,
                key: key.to_owned(),
                snapshot,
                unpublished: true,
            });
        }
        match self.positions.iter().position(|&p| p == MAIN_POSITION) {
            Some(slot) => {
                self.values[slot] = Some(temperature.value);
                false
            }
            None => true,
        }
    }

    /// Forgets every value when the last one taken from the service was
    /// measured on a disk with another key: nothing is shown for an identity
    /// it was not measured on. The sensors stay declared.
    pub(super) fn forget_another_disk(&mut self, key: Option<&str>) {
        let other = self
            .imported
            .as_ref()
            .is_some_and(|imported| Some(imported.key.as_str()) != key);
        if other {
            *self = Self {
                values: vec![None; self.positions.len()],
                positions: std::mem::take(&mut self.positions),
                ..Self::default()
            };
        }
    }
}

/// The driver index of a disk's main temperature (`temperature/drive`).
pub(super) const MAIN_POSITION: usize = 0;

/// One snapshot of the service: when it was received and its `seq`. A
/// restarted service may count from the same number, but not at the same
/// instant; the feed's generation is no part of it, because it also moves
/// when the user's request changes under the same snapshot.
pub(super) type SnapshotId = (Instant, u64);

pub(super) fn snapshot_id(view: &FeedView) -> Option<SnapshotId> {
    let (received, snapshot) = view.snapshot.as_ref()?;
    Some((*received, snapshot.seq))
}
