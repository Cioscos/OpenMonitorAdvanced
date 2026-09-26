//! The latest schema and snapshot received from the sensor service, written
//! by the connection thread and read by the `svc` provider on its tick.
//!
//! The schema and the snapshot change under one lock: a new schema drops
//! the old snapshot in the same step, so a reader never pairs a snapshot
//! with a schema it was not indexed by.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

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
}

struct Inner {
    generation: u64,
    schema: Option<Arc<WireSchema>>,
    snapshot: Option<(Instant, WireSnapshot)>,
    interval: Duration,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            generation: 0,
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
    /// the generation bumps.
    pub fn set_schema(&self, schema: WireSchema) {
        let mut inner = self.lock();
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
        }
    }
}

#[cfg(test)]
mod tests {
    use oma_ipc::{WireDevice, WireSensor};

    use super::*;

    fn schema(sensors: usize) -> WireSchema {
        WireSchema {
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
        }
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
}
