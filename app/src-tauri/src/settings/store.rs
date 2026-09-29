//! The settings store: state under one mutex, revisions, listeners.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use oma_core::settings::{
    apply_patch, decode_lenient, encode, PatchError, Settings, VersionStatus,
};
use serde_json::Value;

use super::{
    bad_file_name, format_stamp, ApplyStatus, Effect, EffectStatus, Listener, Persistence,
    SettingsFs, SettingsState,
};

/// Writer timings: the coalescing window and the delay before a retry.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timings {
    pub coalesce: Duration,
    pub retry: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            coalesce: Duration::from_millis(500),
            retry: Duration::from_secs(5),
        }
    }
}

/// Everything protected by the store mutex.
pub(super) struct Core {
    pub settings: Settings,
    pub revision: u64,
    pub persisted_revision: u64,
    pub seq: u64,
    /// Writes are never attempted: `ReadOnly` (newer file version) or `Error`
    /// (file unreadable, not preserved, or no path).
    pub blocked: Option<Persistence>,
    /// Where the corrupt file was kept; sticky for the session.
    pub recovered: Option<String>,
    /// Reason of the last failed save, cleared by the next successful one.
    pub write_error: Option<String>,
    pub apply_status: ApplyStatus,
    /// Since when the unsaved changes have been waiting.
    pub dirty_since: Option<Instant>,
    /// No save attempt before this instant (after a failure).
    pub retry_at: Option<Instant>,
    /// A flush wants everything up to this revision saved without delay.
    pub flush_target: u64,
    /// A save failed since the flush was requested.
    pub flush_failed: Option<String>,
    /// The writer must exit.
    pub stop: bool,
    /// The writer has exited.
    pub writer_done: bool,
}

impl Core {
    pub(super) fn is_dirty(&self) -> bool {
        self.revision > self.persisted_revision
    }

    pub(super) fn persistence(&self) -> Persistence {
        if let Some(blocked) = &self.blocked {
            blocked.clone()
        } else if let Some(reason) = &self.write_error {
            Persistence::Error {
                reason: reason.clone(),
            }
        } else if let Some(path) = &self.recovered {
            Persistence::Recovered { path: path.clone() }
        } else if self.is_dirty() {
            Persistence::Pending
        } else {
            Persistence::Ok
        }
    }

    pub(super) fn state(&self) -> SettingsState {
        SettingsState {
            settings: encode(&self.settings),
            revision: self.revision,
            persisted_revision: self.persisted_revision,
            seq: self.seq,
            persistence: self.persistence(),
            apply_status: self.apply_status.clone(),
        }
    }

    /// When the writer may save the pending changes.
    pub(super) fn due(&self, now: Instant, coalesce: Duration) -> Instant {
        let coalesced = self.dirty_since.unwrap_or(now) + coalesce;
        match self.retry_at {
            Some(retry) => coalesced.max(retry),
            None => coalesced,
        }
    }
}

#[derive(Default)]
struct Queue {
    items: VecDeque<(Settings, SettingsState)>,
    draining: bool,
}

pub(super) struct Inner {
    pub path: Option<PathBuf>,
    pub fs: Arc<dyn SettingsFs>,
    pub timings: Timings,
    pub core: Mutex<Core>,
    /// Wakes the writer and the threads waiting for a save.
    pub cv: Condvar,
    listeners: Mutex<Vec<Arc<Listener>>>,
    queue: Mutex<Queue>,
    pub writer: Mutex<Option<JoinHandle<()>>>,
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Inner {
    /// Publishes the state after a change made under `core`: bumps `seq`,
    /// queues the snapshot, releases the lock and delivers queued snapshots to
    /// the listeners. Whoever finds the queue idle delivers (also snapshots
    /// queued meanwhile by other threads, or by a listener calling back into
    /// the store), so listeners run outside the lock and in `seq` order.
    pub(super) fn commit(&self, mut core: MutexGuard<'_, Core>) -> SettingsState {
        core.seq += 1;
        let state = core.state();
        lock(&self.queue)
            .items
            .push_back((core.settings.clone(), state.clone()));
        drop(core);
        self.cv.notify_all();
        self.drain();
        state
    }

    fn drain(&self) {
        {
            let mut queue = lock(&self.queue);
            if queue.draining {
                return;
            }
            queue.draining = true;
        }
        loop {
            let item = {
                let mut queue = lock(&self.queue);
                match queue.items.pop_front() {
                    Some(item) => item,
                    None => {
                        queue.draining = false;
                        return;
                    }
                }
            };
            let listeners = lock(&self.listeners).clone();
            for listener in listeners {
                if catch_unwind(AssertUnwindSafe(|| listener(&item.0, &item.1))).is_err() {
                    tracing::error!("a settings listener panicked");
                }
            }
        }
    }

    /// Applies `next` if it differs from the current settings.
    fn apply(&self, mut core: MutexGuard<'_, Core>, next: Settings) -> SettingsState {
        if next == core.settings {
            return core.state();
        }
        core.settings = next;
        core.revision += 1;
        if core.dirty_since.is_none() {
            core.dirty_since = Some(Instant::now());
        }
        self.commit(core)
    }
}

/// What opening the file found.
struct Loaded {
    settings: Settings,
    revision: u64,
    blocked: Option<Persistence>,
    recovered: Option<String>,
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Moves a corrupt file aside under a unique name; `Ok` carries the new path.
fn preserve_corrupt(fs: &dyn SettingsFs, path: &Path) -> std::io::Result<PathBuf> {
    let base = path.file_name().map_or_else(
        || "settings.json".into(),
        |n| n.to_string_lossy().into_owned(),
    );
    let stamp = format_stamp(unix_secs());
    let pid = std::process::id();
    let mut attempt = 1;
    loop {
        let target = path.with_file_name(bad_file_name(&base, &stamp, pid, attempt));
        match fs.preserve(path, &target) {
            Ok(()) => return Ok(target),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && attempt < 1_000 => {
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

fn load(path: Option<&Path>, fs: &dyn SettingsFs) -> Loaded {
    let defaults = |blocked, recovered, revision| Loaded {
        settings: Settings::default(),
        revision,
        blocked,
        recovered,
    };
    let Some(path) = path else {
        tracing::warn!("no settings path: settings stay in memory");
        return defaults(
            Some(Persistence::Error {
                reason: "noSettingsPath".into(),
            }),
            None,
            0,
        );
    };
    let bytes = match fs.read(path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return defaults(None, None, 0),
        Err(err) => {
            tracing::error!(%err, path = %path.display(), "cannot read the settings file; changes will not be saved");
            return defaults(
                Some(Persistence::Error {
                    reason: format!("read: {err}"),
                }),
                None,
                0,
            );
        }
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) if value.is_object() => {
            let decoded = decode_lenient(&value);
            for diagnostic in &decoded.diagnostics {
                tracing::warn!(path = %diagnostic.path, kind = ?diagnostic.kind, "settings file deviation");
            }
            let blocked = match decoded.version {
                VersionStatus::Current => None,
                VersionStatus::Future(version) => {
                    tracing::warn!(version, "settings file is from a newer version: read-only");
                    Some(Persistence::ReadOnly {
                        reason: "futureVersion".into(),
                    })
                }
            };
            Loaded {
                settings: decoded.settings,
                revision: 0,
                blocked,
                recovered: None,
            }
        }
        _ => match preserve_corrupt(fs, path) {
            Ok(kept) => {
                tracing::warn!(kept = %kept.display(), "corrupt settings file kept; defaults will be saved");
                // Revision 1 is unsaved: the writer saves the defaults.
                defaults(None, Some(kept.display().to_string()), 1)
            }
            Err(err) => {
                tracing::error!(%err, "cannot preserve the corrupt settings file; changes will not be saved");
                defaults(
                    Some(Persistence::Error {
                        reason: format!("preserve: {err}"),
                    }),
                    None,
                    0,
                )
            }
        },
    }
}

pub struct SettingsStore {
    pub(super) inner: Arc<Inner>,
}

impl SettingsStore {
    /// Opens the store with the production timings (500 ms window, 5 s retry).
    pub fn open(path: Option<PathBuf>, fs: Arc<dyn SettingsFs>) -> Self {
        Self::open_with(path, fs, Timings::default())
    }

    pub(crate) fn open_with(
        path: Option<PathBuf>,
        fs: Arc<dyn SettingsFs>,
        timings: Timings,
    ) -> Self {
        let loaded = load(path.as_deref(), fs.as_ref());
        let core = Core {
            settings: loaded.settings,
            revision: loaded.revision,
            persisted_revision: 0,
            seq: 0,
            blocked: loaded.blocked,
            recovered: loaded.recovered,
            write_error: None,
            apply_status: ApplyStatus::default(),
            dirty_since: (loaded.revision > 0).then(Instant::now),
            retry_at: None,
            flush_target: 0,
            flush_failed: None,
            stop: false,
            writer_done: false,
        };
        let inner = Arc::new(Inner {
            path,
            fs,
            timings,
            core: Mutex::new(core),
            cv: Condvar::new(),
            listeners: Mutex::new(Vec::new()),
            queue: Mutex::new(Queue::default()),
            writer: Mutex::new(None),
        });
        inner.spawn_writer();
        Self { inner }
    }

    pub fn settings(&self) -> Settings {
        lock(&self.inner.core).settings.clone()
    }

    pub fn state(&self) -> SettingsState {
        lock(&self.inner.core).state()
    }

    /// Validates `patch` against the whole result and applies it in memory;
    /// saving is deferred. A patch that changes nothing returns the current
    /// state without a new revision.
    pub fn update(&self, patch: &Value) -> Result<SettingsState, PatchError> {
        let core = lock(&self.inner.core);
        let next = apply_patch(&core.settings, patch)?;
        Ok(self.inner.apply(core, next))
    }

    /// Internal change (migrations, tray, autostart read-back). `change` runs
    /// under the store lock and must not call back into the store.
    pub fn update_with(&self, change: impl FnOnce(&mut Settings)) -> SettingsState {
        let core = lock(&self.inner.core);
        let mut next = core.settings.clone();
        change(&mut next);
        self.inner.apply(core, next)
    }

    /// Waits until the current revision is saved, skipping the coalescing wait.
    pub fn flush_now(&self, timeout: Duration) -> Result<(), String> {
        self.inner.flush_now(timeout)
    }

    // Called by the effects of the next M5a tasks.
    #[allow(dead_code)]
    pub fn set_effect(&self, effect: Effect, status: EffectStatus) {
        let mut core = lock(&self.inner.core);
        let slot = match effect {
            Effect::Service => &mut core.apply_status.service,
            Effect::Autostart => &mut core.apply_status.autostart,
            Effect::VendorLibraries => &mut core.apply_status.vendor_libraries,
        };
        if *slot == status {
            return;
        }
        *slot = status;
        self.inner.commit(core);
    }

    /// Registers a listener for every later applied change and state change.
    pub fn subscribe(&self, listener: Listener) {
        lock(&self.inner.listeners).push(Arc::new(listener));
    }

    /// Final flush, then stops the writer; gives up after `timeout`.
    pub fn shutdown(&self, timeout: Duration) -> Result<(), String> {
        self.inner.shutdown(timeout)
    }
}

impl Drop for SettingsStore {
    fn drop(&mut self) {
        // The writer thread owns a reference of its own; tell it to leave.
        lock(&self.inner.core).stop = true;
        self.inner.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::settings::fake_fs::{open_fast, stored_json, test_path, wait_until, FakeFs};
    use crate::settings::Persistence;

    const LONG: Duration = Duration::from_secs(5);

    fn interval(state: &SettingsState) -> u64 {
        state.settings["general"]["intervalMs"].as_u64().unwrap()
    }

    #[test]
    fn missing_file_starts_with_defaults_and_writes_nothing() {
        let fs = FakeFs::new();
        let store = open_fast(&fs);
        let state = store.state();
        assert_eq!(state.persistence, Persistence::Ok);
        assert_eq!((state.revision, state.persisted_revision), (0, 0));
        assert_eq!(store.settings(), Settings::default());
        store.shutdown(LONG).unwrap();
        assert_eq!(fs.write_attempts(), 0);
    }

    #[test]
    fn valid_file_is_loaded_and_diagnostics_do_not_block() {
        let file = br#"{"version":1,"general":{"intervalMs":2000,"chartFps":"fast"}}"#;
        let fs = FakeFs::new().with_file(&test_path(), file);
        let store = open_fast(&fs);
        let state = store.state();
        assert_eq!(state.persistence, Persistence::Ok);
        assert_eq!(interval(&state), 2000);
        // The wrong-typed field fell back to its default.
        assert_eq!(state.settings["general"]["chartFps"], 60);
        store.shutdown(LONG).unwrap();
        assert_eq!(fs.write_attempts(), 0);
    }

    #[test]
    fn corrupt_json_is_preserved_then_defaults_are_saved() {
        let fs = FakeFs::new().with_file(&test_path(), b"{ not json");
        let store = open_fast(&fs);
        let Persistence::Recovered { path } = store.state().persistence else {
            panic!("expected Recovered, got {:?}", store.state().persistence);
        };
        assert!(path.contains("settings.json.bad-"), "{path}");
        store.flush_now(LONG).unwrap();
        // The original was moved away before anything was written.
        assert_eq!(fs.ops(), ["read", "preserve", "write"]);
        let kept = fs.files_with_prefix("settings.json.bad-");
        assert_eq!(kept.len(), 1);
        assert_eq!(fs.file(&kept[0]).unwrap(), b"{ not json");
        assert_eq!(stored_json(&fs)["general"]["intervalMs"], 1000);
        let state = store.state();
        assert!(matches!(state.persistence, Persistence::Recovered { .. }));
        assert_eq!(state.persisted_revision, state.revision);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn corrupt_json_name_collisions_get_a_numeric_suffix() {
        let fs = FakeFs::new().with_file(&test_path(), b"[1,2]");
        fs.collide_next_preserves(2);
        let store = open_fast(&fs);
        let Persistence::Recovered { path } = store.state().persistence else {
            panic!("expected Recovered");
        };
        assert!(path.ends_with("-3"), "{path}");
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn preserve_failure_blocks_writes() {
        let fs = FakeFs::new().with_file(&test_path(), b"garbage");
        fs.set_fail_preserve(true);
        let store = open_fast(&fs);
        assert!(matches!(
            store.state().persistence,
            Persistence::Error { .. }
        ));
        store
            .update(&json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        assert!(store.flush_now(Duration::from_millis(200)).is_err());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fs.write_attempts(), 0);
        assert!(matches!(
            store.state().persistence,
            Persistence::Error { .. }
        ));
        // The original is untouched.
        assert_eq!(fs.file(&test_path()).unwrap(), b"garbage");
    }

    #[test]
    fn read_error_blocks_writes() {
        let fs = FakeFs::new().with_file(&test_path(), b"{}");
        fs.set_fail_read(true);
        let store = open_fast(&fs);
        assert!(matches!(
            store.state().persistence,
            Persistence::Error { .. }
        ));
        store
            .update(&json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fs.write_attempts(), 0);
        assert!(matches!(
            store.state().persistence,
            Persistence::Error { .. }
        ));
        assert_eq!(store.settings().general.interval_ms, 2000);
    }

    #[test]
    fn missing_settings_path_keeps_settings_in_memory() {
        let fs = FakeFs::new();
        let store = crate::settings::SettingsStore::open(None, fs.clone());
        assert!(matches!(
            store.state().persistence,
            Persistence::Error { .. }
        ));
        store
            .update(&json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(fs.write_attempts(), 0);
    }

    #[test]
    fn future_version_is_read_only_forever() {
        let file = br#"{"version":99,"general":{"intervalMs":3000}}"#;
        let fs = FakeFs::new().with_file(&test_path(), file);
        let store = open_fast(&fs);
        assert_eq!(
            store.state().persistence,
            Persistence::ReadOnly {
                reason: "futureVersion".into()
            }
        );
        assert_eq!(store.settings().general.interval_ms, 3000);
        let state = store
            .update(&json!({"general": {"intervalMs": 4000}}))
            .unwrap();
        assert_eq!(interval(&state), 4000);
        // Read-only by design: a flush has nothing to do and is not a failure.
        store.flush_now(LONG).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fs.write_attempts(), 0);
        assert!(matches!(
            store.state().persistence,
            Persistence::ReadOnly { .. }
        ));
        assert_eq!(fs.file(&test_path()).unwrap(), file);
    }

    #[test]
    fn identical_update_is_a_no_op() {
        let fs = FakeFs::new();
        let store = open_fast(&fs);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        store.subscribe(Box::new(move |_, _| {
            seen.fetch_add(1, Ordering::SeqCst);
        }));
        let before = store.state();
        let after = store
            .update(&json!({"general": {"intervalMs": 1000}}))
            .unwrap();
        assert_eq!(after, before);
        let after = store.update_with(|_| {});
        assert_eq!(after, before);
        store.shutdown(LONG).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(fs.write_attempts(), 0);
    }

    #[test]
    fn invalid_patch_changes_nothing() {
        let fs = FakeFs::new();
        let store = open_fast(&fs);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        store.subscribe(Box::new(move |_, _| {
            seen.fetch_add(1, Ordering::SeqCst);
        }));
        let before = store.state();
        let err = store
            .update(&json!({"general": {"intervalMs": 700}}))
            .unwrap_err();
        assert_eq!(err.field, "general.intervalMs");
        assert!(err.key.starts_with("settings.error."), "{}", err.key);
        assert_eq!(store.state(), before);
        store.shutdown(LONG).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(fs.write_attempts(), 0);
    }

    #[test]
    fn concurrent_updates_serialize_last_writer_wins() {
        let fs = FakeFs::new();
        let store = Arc::new(open_fast(&fs));
        let handles: Vec<_> = (1..=8u32)
            .map(|i| {
                let store = store.clone();
                std::thread::spawn(move || {
                    store
                        .update(&json!({"general": {"intervalMs": 1000 + 500 * i}}))
                        .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        store.flush_now(LONG).unwrap();
        let state = store.state();
        assert_eq!(state.revision, 8);
        assert_eq!(state.persisted_revision, 8);
        assert_eq!(stored_json(&fs), state.settings);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn listeners_see_every_change_in_seq_order() {
        let fs = FakeFs::new();
        let store = open_fast(&fs);
        let seen: Arc<Mutex<Vec<(u64, u64)>>> = Arc::default();
        let sink = seen.clone();
        store.subscribe(Box::new(move |_, state| {
            sink.lock().unwrap().push((state.seq, state.revision));
        }));
        store
            .update(&json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        store.set_effect(Effect::Autostart, EffectStatus::Pending);
        store.update_with(|s| s.general.interval_ms = 3000);
        store.flush_now(LONG).unwrap();
        // The save itself (persisted revision) is emitted too.
        wait_until("the save notification", || {
            seen.lock().unwrap().last().map(|e| e.0) == Some(store.state().seq)
        });
        let seen = seen.lock().unwrap().clone();
        assert!(seen.len() >= 4, "{seen:?}");
        let seqs: Vec<u64> = seen.iter().map(|e| e.0).collect();
        let expected: Vec<u64> = (1..=seqs.len() as u64).collect();
        assert_eq!(seqs, expected, "every seq exactly once, in order");
        assert!(seen.windows(2).all(|w| w[0].1 <= w[1].1));
        assert_eq!(store.state().apply_status.autostart, EffectStatus::Pending);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn a_listener_may_call_back_into_the_store() {
        let fs = FakeFs::new();
        let store = Arc::new(open_fast(&fs));
        let inner = store.clone();
        store.subscribe(Box::new(move |settings, _| {
            if settings.general.interval_ms == 2000 {
                inner.update_with(|s| s.general.interval_ms = 2500);
            }
        }));
        store
            .update(&json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        assert_eq!(store.settings().general.interval_ms, 2500);
        assert_eq!(store.state().revision, 2);
        store.shutdown(LONG).unwrap();
    }
}
