//! The latest schema and snapshot received from the sensor service, written
//! by the connection thread and read by the `svc` provider on its tick.
//!
//! The schema and the snapshot change under one lock: a new schema drops
//! the old snapshot in the same step, so a reader never pairs a snapshot
//! with a schema it was not indexed by.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

pub use oma_ipc::SourceRequest;
use oma_ipc::{WireSchema, WireSnapshot};

/// Interval assumed until the connection subscribes.
const DEFAULT_INTERVAL: Duration = Duration::from_millis(1000);

/// What the provider reads.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedView {
    /// Changes whenever the schema changes or the feed is cleared: the
    /// provider rediscovers when it differs from the one it bound.
    pub generation: u64,
    pub schema: Option<Arc<WireSchema>>,
    /// The last snapshot for `schema`, with the time it was received.
    pub snapshot: Option<(Instant, WireSnapshot)>,
    /// The subscribed sampling interval.
    pub interval: Duration,
    /// The sources the user turned off; a change bumps `generation`.
    pub request: Arc<SourceRequest>,
}

struct Inner {
    generation: u64,
    request: Arc<SourceRequest>,
    schema: Option<Arc<WireSchema>>,
    snapshot: Option<(Instant, WireSnapshot)>,
    interval: Duration,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            generation: 0,
            request: Arc::default(),
            schema: None,
            snapshot: None,
            interval: DEFAULT_INTERVAL,
        }
    }
}

/// Latest data from the service; cheap to clone (shared state).
#[derive(Clone, Default)]
pub struct SvcFeed(Arc<Mutex<Inner>>);

impl SvcFeed {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the schema and drops the snapshot in the same step;
    /// the generation bumps. A schema equal to the current one changes
    /// nothing: the service describes itself again after every accepted
    /// `Subscribe` (an interval change), and the provider must not rediscover
    /// (and lose a sample of every series) for a description that did not
    /// change. Any difference at all counts as a new schema.
    ///
    /// The rest of the `service` block is not part of that comparison: it
    /// describes the service's configuration, not the sensors, and it changes
    /// when a reconfiguration finishes. A schema that differs only there
    /// replaces the stored one and leaves the snapshot and the generation
    /// alone. The drive table (`service.drives`) is the exception: a change
    /// there drops the previous snapshot and bumps the generation even with
    /// devices and sensors unchanged, so a drive state never pairs with a
    /// snapshot taken under another table; only the next snapshot of the
    /// same connection gives the new states their authority back.
    pub fn set_schema(&self, schema: WireSchema) {
        let mut inner = self.lock();
        if let Some(current) = inner.schema.as_deref() {
            if current.devices == schema.devices
                && current.sensors == schema.sensors
                && current.service.drives == schema.service.drives
            {
                if current.service != schema.service {
                    inner.schema = Some(Arc::new(schema));
                }
                return;
            }
        }
        inner.schema = Some(Arc::new(schema));
        inner.snapshot = None;
        inner.generation += 1;
    }

    /// Stores the snapshot for the current schema, received at `received`.
    pub fn set_snapshot(&self, snapshot: WireSnapshot, received: Instant) {
        self.lock().snapshot = Some((received, snapshot));
    }

    pub fn set_interval(&self, interval: Duration) {
        self.lock().interval = interval;
    }

    /// Stores what the user turned off. The generation bumps when it
    /// changes, so the provider rediscovers with the new filter; the request
    /// survives [`clear`](Self::clear).
    pub fn set_request(&self, request: SourceRequest) {
        let mut inner = self.lock();
        if *inner.request != request {
            inner.request = Arc::new(request);
            inner.generation += 1;
        }
    }

    /// Drops the schema and the snapshot; the generation bumps when there
    /// was something to drop (clearing an empty feed changes nothing, so a
    /// disconnected link retrying every 5 s does not make the provider
    /// rediscover every 5 s).
    pub fn clear(&self) {
        let mut inner = self.lock();
        if inner.schema.is_some() || inner.snapshot.is_some() {
            inner.schema = None;
            inner.snapshot = None;
            inner.generation += 1;
        }
    }

    pub fn view(&self) -> FeedView {
        let inner = self.lock();
        FeedView {
            generation: inner.generation,
            schema: inner.schema.clone(),
            snapshot: inner.snapshot.clone(),
            interval: inner.interval,
            request: Arc::clone(&inner.request),
        }
    }
}

#[cfg(test)]
mod tests {
    use oma_ipc::{WireDevice, WireSensor};

    use super::*;

    fn schema(sensors: usize) -> WireSchema {
        WireSchema {
            service: Default::default(),
            devices: vec![WireDevice {
                id: "cpu".to_owned(),
                kind: "cpu".to_owned(),
                name: "CPU".to_owned(),
                vendor: None,
                properties: Default::default(),
                hint: None,
            }],
            sensors: (0..sensors)
                .map(|i| WireSensor {
                    device_id: "cpu".to_owned(),
                    kind: "temperature".to_owned(),
                    name: format!("s{i}"),
                    unit: "celsius".to_owned(),
                    label_key: "lhm.raw".to_owned(),
                    label_arg: Some(format!("S{i}")),
                    category: "temperature".to_owned(),
                })
                .collect(),
        }
    }

    fn snapshot(seq: u64, values: usize) -> WireSnapshot {
        WireSnapshot {
            seq,
            timestamp_ms: seq * 1000,
            values: vec![Some(1.0); values],
            held: vec![false; values],
        }
    }

    fn drive(physical_drive: u32, state: &str) -> oma_ipc::WireDrive {
        oma_ipc::WireDrive {
            physical_drive,
            key: None,
            model: Some("Disk".to_owned()),
            state: state.to_owned(),
            blocks_smart: false,
        }
    }

    fn schema_with_drives(drives: Vec<oma_ipc::WireDrive>) -> WireSchema {
        let mut schema = schema(2);
        schema.service.drives = drives;
        schema
    }

    #[test]
    fn a_new_feed_is_empty() {
        let view = SvcFeed::default().view();
        assert_eq!(view.generation, 0);
        assert!(view.schema.is_none());
        assert!(view.snapshot.is_none());
        assert_eq!(view.interval, DEFAULT_INTERVAL);
    }

    #[test]
    fn new_schema_clears_old_snapshot_atomically() {
        let feed = SvcFeed::default();
        feed.set_schema(schema(2));
        let at = Instant::now();
        feed.set_snapshot(snapshot(1, 2), at);
        let before = feed.view();
        assert_eq!(before.snapshot, Some((at, snapshot(1, 2))));

        feed.set_schema(schema(3));
        let after = feed.view();
        assert!(after.generation > before.generation);
        assert_eq!(after.schema.as_deref(), Some(&schema(3)));
        assert!(
            after.snapshot.is_none(),
            "the old snapshot must not survive"
        );
    }

    #[test]
    fn snapshot_and_interval_do_not_change_the_generation() {
        let feed = SvcFeed::default();
        feed.set_schema(schema(1));
        let generation = feed.view().generation;
        feed.set_interval(Duration::from_millis(500));
        feed.set_snapshot(snapshot(1, 1), Instant::now());
        let view = feed.view();
        assert_eq!(view.generation, generation);
        assert_eq!(view.interval, Duration::from_millis(500));
        assert_eq!(view.snapshot.map(|(_, s)| s.seq), Some(1));
    }

    #[test]
    fn the_same_schema_again_changes_nothing() {
        let feed = SvcFeed::default();
        feed.set_schema(schema(2));
        feed.set_snapshot(snapshot(1, 2), Instant::now());
        let generation = feed.view().generation;
        // A resubscribe makes the service describe itself again.
        feed.set_schema(schema(2));
        let view = feed.view();
        assert_eq!(view.generation, generation);
        assert_eq!(view.snapshot.map(|(_, s)| s.seq), Some(1));
        // Any difference still counts as a new schema.
        feed.set_schema(schema(3));
        let view = feed.view();
        assert!(view.generation > generation);
        assert!(view.snapshot.is_none());
    }

    #[test]
    fn clear_drops_everything_and_bumps_once() {
        let feed = SvcFeed::default();
        feed.set_schema(schema(1));
        feed.set_snapshot(snapshot(1, 1), Instant::now());
        let before = feed.view().generation;

        feed.clear();
        let view = feed.view();
        assert!(view.generation > before);
        assert!(view.schema.is_none());
        assert!(view.snapshot.is_none());

        // Nothing left to drop: no rediscovery for the provider.
        feed.clear();
        assert_eq!(feed.view().generation, view.generation);
    }

    #[test]
    fn only_a_changed_service_block_keeps_snapshot_and_generation() {
        let feed = SvcFeed::default();
        feed.set_schema(schema(2));
        feed.set_snapshot(snapshot(1, 2), Instant::now());
        let generation = feed.view().generation;

        // Same devices and sensors, another `service` block (the
        // reconfiguration went from pending to applied): the provider must
        // not rediscover and the snapshot still matches.
        let mut same_shape = schema(2);
        same_shape.service.reconfiguration = "pending".to_owned();
        feed.set_schema(same_shape.clone());
        let view = feed.view();
        assert_eq!(view.generation, generation);
        assert_eq!(view.snapshot.map(|(_, s)| s.seq), Some(1));
        assert_eq!(view.schema.as_deref(), Some(&same_shape));
    }

    #[test]
    fn a_drive_state_change_drops_the_previous_snapshot() {
        let feed = SvcFeed::default();
        feed.set_schema(schema_with_drives(vec![drive(0, "active")]));
        feed.set_snapshot(snapshot(1, 2), Instant::now());
        let before = feed.view();

        // Same devices and sensors, another state for the drive.
        let changed = schema_with_drives(vec![drive(0, "standby")]);
        feed.set_schema(changed.clone());
        let after = feed.view();
        assert!(after.generation > before.generation);
        assert!(
            after.snapshot.is_none(),
            "the old snapshot must not survive"
        );
        assert_eq!(after.schema.as_deref(), Some(&changed));

        // The next snapshot of the same connection restores authority.
        feed.set_snapshot(snapshot(2, 2), Instant::now());
        assert_eq!(feed.view().snapshot.map(|(_, s)| s.seq), Some(2));
    }

    #[test]
    fn an_equal_drive_table_keeps_the_snapshot() {
        let feed = SvcFeed::default();
        feed.set_schema(schema_with_drives(vec![drive(0, "active")]));
        feed.set_snapshot(snapshot(1, 2), Instant::now());
        let generation = feed.view().generation;

        // Only the reconfiguration differs: the drives are equal.
        let mut same_drives = schema_with_drives(vec![drive(0, "active")]);
        same_drives.service.reconfiguration = "pending".to_owned();
        feed.set_schema(same_drives.clone());
        let view = feed.view();
        assert_eq!(view.generation, generation);
        assert_eq!(view.snapshot.map(|(_, s)| s.seq), Some(1));
        assert_eq!(view.schema.as_deref(), Some(&same_drives));
    }

    #[test]
    fn clearing_the_feed_revokes_drive_authority() {
        let feed = SvcFeed::default();
        feed.set_schema(schema_with_drives(vec![drive(0, "standby")]));
        feed.set_snapshot(snapshot(1, 2), Instant::now());
        let before = feed.view().generation;

        feed.clear();
        let view = feed.view();
        assert!(view.generation > before);
        assert!(view.schema.is_none(), "no drive table survives a clear");
        assert!(view.snapshot.is_none());

        // A schema with the same table on the next connection starts without a snapshot.
        feed.set_schema(schema_with_drives(vec![drive(0, "standby")]));
        assert!(feed.view().snapshot.is_none());
    }

    #[test]
    fn a_changed_request_bumps_the_generation_once() {
        let feed = SvcFeed::default();
        assert_eq!(*feed.view().request, SourceRequest::default());
        let generation = feed.view().generation;

        let request = SourceRequest {
            disabled_modules: vec!["psu".to_owned()],
            smart_disabled_drives: vec!["storage/device-a".to_owned()],
            smart_enabled_drives: vec!["storage/device-b".to_owned()],
        };
        feed.set_request(request.clone());
        let view = feed.view();
        assert_eq!(*view.request, request);
        assert!(view.generation > generation);

        // The same request again changes nothing.
        feed.set_request(request);
        assert_eq!(feed.view().generation, view.generation);
    }

    #[test]
    fn clear_keeps_the_request() {
        let feed = SvcFeed::default();
        let request = SourceRequest {
            disabled_modules: vec!["cpu".to_owned()],
            smart_disabled_drives: Vec::new(),
            smart_enabled_drives: Vec::new(),
        };
        feed.set_request(request.clone());
        feed.set_schema(schema(1));
        feed.clear();
        assert_eq!(*feed.view().request, request);
    }
}
