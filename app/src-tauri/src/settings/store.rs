//! The settings store: state under one mutex, revisions, listeners.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use oma_core::settings::{
    apply_patch, decode_lenient, encode, Decoded, PatchError, Settings, VersionStatus,
};
use serde_json::Value;

use super::{
    bad_file_name, format_stamp, tmp_path, ApplyStatus, Effect, EffectStatus, Listener,
    Persistence, SettingsFs, SettingsState,
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
    /// Replaced (never mutated) on every change, so readers share it.
    pub settings: Arc<Settings>,
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
    items: VecDeque<(Arc<Settings>, SettingsState)>,
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
            .push_back((Arc::clone(&core.settings), state.clone()));
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
                if catch_unwind(AssertUnwindSafe(|| listener(item.0.as_ref(), &item.1))).is_err() {
                    tracing::error!("a settings listener panicked");
                }
            }
        }
    }

    /// Applies `next` if it differs from the current settings.
    fn apply(&self, mut core: MutexGuard<'_, Core>, next: Settings) -> SettingsState {
        if next == *core.settings {
            return core.state();
        }
        core.settings = Arc::new(next);
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

impl Loaded {
    fn defaults(blocked: Option<Persistence>, recovered: Option<String>, revision: u64) -> Self {
        Self {
            settings: Settings::default(),
            revision,
            blocked,
            recovered,
        }
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Runs `place` with unique `<name>.bad-<stamp>-<pid>[-n]` targets next to
/// `path` until one is free; `Ok` carries the target that was used.
fn place_aside(
    path: &Path,
    mut place: impl FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<PathBuf> {
    let base = path.file_name().map_or_else(
        || "settings.json".into(),
        |n| n.to_string_lossy().into_owned(),
    );
    let stamp = format_stamp(unix_secs());
    let pid = std::process::id();
    let mut attempt = 1;
    loop {
        let target = path.with_file_name(bad_file_name(&base, &stamp, pid, attempt));
        match place(&target) {
            Ok(()) => return Ok(target),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && attempt < 1_000 => {
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

/// The decoded document when `bytes` are a JSON object, `None` when corrupt.
fn parse(bytes: &[u8]) -> Option<Decoded> {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(value) if value.is_object() => {
            let decoded = decode_lenient(&value);
            for diagnostic in &decoded.diagnostics {
                tracing::warn!(path = %diagnostic.path, kind = ?diagnostic.kind, "settings file deviation");
            }
            Some(decoded)
        }
        _ => None,
    }
}

/// The blocking state a decoded document implies: `ReadOnly` for a newer version.
fn version_block(decoded: &Decoded) -> Option<Persistence> {
    match decoded.version {
        VersionStatus::Current => None,
        VersionStatus::Future(version) => {
            tracing::warn!(version, "settings file is from a newer version: read-only");
            Some(Persistence::ReadOnly {
                reason: "futureVersion".into(),
            })
        }
    }
}

/// The file cannot be used: move it aside and start from defaults, which the
/// writer saves (revision 1 is unsaved). If it cannot be moved, writes stay
/// blocked so the file is never overwritten.
fn recover_corrupt(fs: &dyn SettingsFs, file: &Path) -> Loaded {
    match place_aside(file, |target| fs.preserve(file, target)) {
        Ok(kept) => {
            tracing::warn!(kept = %kept.display(), "corrupt settings file kept; defaults will be saved");
            Loaded::defaults(None, Some(kept.display().to_string()), 1)
        }
        Err(err) => {
            tracing::error!(%err, "cannot preserve the corrupt settings file; changes will not be saved");
            Loaded::defaults(
                Some(Persistence::Error {
                    reason: format!("preserve: {err}"),
                }),
                None,
                0,
            )
        }
    }
}

/// `settings.json` is missing: a failed save may have left the new content in
/// `settings.json.tmp`. It is read with the same checks as the file itself.
fn load_leftover(fs: &dyn SettingsFs, path: &Path) -> Loaded {
    let tmp = tmp_path(path);
    let bytes = match fs.read(&tmp) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Loaded::defaults(None, None, 0),
        Err(err) => {
            tracing::error!(%err, path = %tmp.display(), "cannot read the leftover settings file; changes will not be saved");
            return Loaded::defaults(
                Some(Persistence::Error {
                    reason: format!("read: {err}"),
                }),
                None,
                0,
            );
        }
    };
    let Some(decoded) = parse(&bytes) else {
        return recover_corrupt(fs, &tmp);
    };
    if let Some(blocked) = version_block(&decoded) {
        // Read-only: no file is touched.
        return Loaded {
            settings: decoded.settings,
            revision: 0,
            blocked: Some(blocked),
            recovered: None,
        };
    }
    // `write_atomic` reuses the temporary file, so copy it aside first; the
    // copy is the stable backup `Recovered` points to. Revision 1 is unsaved.
    match place_aside(&tmp, |target| fs.copy_exclusive(&tmp, target)) {
        Ok(kept) => {
            tracing::warn!(kept = %kept.display(), "adopted a leftover settings.json.tmp; it will be saved as settings.json");
            Loaded {
                settings: decoded.settings,
                revision: 1,
                blocked: None,
                recovered: Some(kept.display().to_string()),
            }
        }
        Err(err) => {
            tracing::error!(%err, "cannot back up the leftover settings file; changes will not be saved");
            Loaded {
                settings: decoded.settings,
                revision: 0,
                blocked: Some(Persistence::Error {
                    reason: format!("preserve: {err}"),
                }),
                recovered: None,
            }
        }
    }
}

fn load(path: Option<&Path>, fs: &dyn SettingsFs) -> Loaded {
    let Some(path) = path else {
        tracing::warn!("no settings path: settings stay in memory");
        return Loaded::defaults(
            Some(Persistence::Error {
                reason: "noSettingsPath".into(),
            }),
            None,
            0,
        );
    };
    let bytes = match fs.read(path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return load_leftover(fs, path),
        Err(err) => {
            tracing::error!(%err, path = %path.display(), "cannot read the settings file; changes will not be saved");
            return Loaded::defaults(
                Some(Persistence::Error {
                    reason: format!("read: {err}"),
                }),
                None,
                0,
            );
        }
    };
    let Some(decoded) = parse(&bytes) else {
        return recover_corrupt(fs, path);
    };
    let blocked = version_block(&decoded);
    if blocked.is_none() {
        // A readable, valid file prevails over a leftover from an earlier save.
        // With any other outcome the leftover stays: it may hold the only good copy.
        match fs.remove(&tmp_path(path)) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!(%err, "cannot remove the leftover settings.json.tmp"),
        }
    }
    Loaded {
        settings: decoded.settings,
        revision: 0,
        blocked,
        recovered: None,
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
            settings: Arc::new(loaded.settings),
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

    /// An owned copy, for callers that change it. Readers that only look
    /// (the tick) use [`Self::snapshot`].
    pub fn settings(&self) -> Settings {
        Settings::clone(&lock(&self.inner.core).settings)
    }

    /// The current settings, shared: the value is replaced on every change, so
    /// this is a reference-count bump and never a deep clone.
    pub fn snapshot(&self) -> Arc<Settings> {
        Arc::clone(&lock(&self.inner.core).settings)
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
        let mut next = Settings::clone(&core.settings);
        change(&mut next);
        self.inner.apply(core, next)
    }

    /// Waits until the current revision is saved, skipping the coalescing wait.
    pub fn flush_now(&self, timeout: Duration) -> Result<(), String> {
        self.inner.flush_now(timeout)
    }

    /// Records the state of an effect outside the file and tells the listeners.
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
    use crate::settings::fake_fs::{
        open_fast, stored_json, test_path, test_tmp_path, wait_until, FakeFs,
    };
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

    const MAIN_2000: &[u8] = br#"{"version":1,"general":{"intervalMs":2000}}"#;
    const TMP_3000: &[u8] = br#"{"version":1,"general":{"intervalMs":3000}}"#;

    #[test]
    fn leftover_tmp_is_removed_when_the_file_exists() {
        let fs = FakeFs::new()
            .with_file(&test_path(), MAIN_2000)
            .with_file(&test_tmp_path(), TMP_3000);
        let store = open_fast(&fs);
        let state = store.state();
        assert_eq!(state.persistence, Persistence::Ok);
        assert_eq!(interval(&state), 2000);
        assert_eq!(fs.file(&test_tmp_path()), None);
        assert_eq!(fs.file(&test_path()).unwrap(), MAIN_2000);
        store.shutdown(LONG).unwrap();
        assert_eq!(fs.write_attempts(), 0);
    }

    #[test]
    fn leftover_tmp_is_adopted_when_the_file_is_missing() {
        let fs = FakeFs::new().with_file(&test_tmp_path(), TMP_3000);
        let store = open_fast(&fs);
        let Persistence::Recovered { path } = store.state().persistence else {
            panic!("expected Recovered, got {:?}", store.state().persistence);
        };
        assert!(path.contains("settings.json.tmp.bad-"), "{path}");
        assert_eq!(store.settings().general.interval_ms, 3000);
        store.flush_now(LONG).unwrap();
        // The next save goes to settings.json with the adopted values.
        assert_eq!(stored_json(&fs)["general"]["intervalMs"], 3000);
        let state = store.state();
        assert!(matches!(state.persistence, Persistence::Recovered { .. }));
        assert_eq!(state.persisted_revision, state.revision);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn corrupt_leftover_tmp_is_preserved_and_defaults_are_used() {
        let fs = FakeFs::new().with_file(&test_tmp_path(), b"{ not json");
        let store = open_fast(&fs);
        let Persistence::Recovered { path } = store.state().persistence else {
            panic!("expected Recovered, got {:?}", store.state().persistence);
        };
        assert!(path.contains("settings.json.tmp.bad-"), "{path}");
        assert_eq!(store.settings(), Settings::default());
        store.flush_now(LONG).unwrap();
        assert_eq!(fs.ops(), ["read", "read", "preserve", "write"]);
        let kept = fs.files_with_prefix("settings.json.tmp.bad-");
        assert_eq!(kept.len(), 1);
        assert_eq!(fs.file(&kept[0]).unwrap(), b"{ not json");
        assert_eq!(fs.file(&test_tmp_path()), None);
        assert_eq!(stored_json(&fs)["general"]["intervalMs"], 1000);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn future_version_tmp_is_read_only() {
        let tmp = br#"{"version":99,"general":{"intervalMs":3000}}"#;
        let fs = FakeFs::new().with_file(&test_tmp_path(), tmp);
        let store = open_fast(&fs);
        assert_eq!(
            store.state().persistence,
            Persistence::ReadOnly {
                reason: "futureVersion".into()
            }
        );
        assert_eq!(store.settings().general.interval_ms, 3000);
        store
            .update(&json!({"general": {"intervalMs": 4000}}))
            .unwrap();
        store.flush_now(LONG).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fs.write_attempts(), 0);
        // No file was touched: no backup, no move, no removal.
        assert_eq!(fs.ops(), ["read", "read"]);
        assert_eq!(fs.file(&test_tmp_path()).unwrap(), tmp);
        assert_eq!(fs.file(&test_path()), None);
    }

    #[test]
    fn invalid_primary_does_not_delete_tmp() {
        // A corrupt file, a file from a newer version and an unreadable file:
        // the leftover may hold the only good copy, so it is never removed.
        enum Primary {
            Corrupt,
            Future,
            Unreadable,
        }
        for kind in [Primary::Corrupt, Primary::Future, Primary::Unreadable] {
            let main: &[u8] = match kind {
                Primary::Corrupt => b"{ not json",
                Primary::Future => br#"{"version":99}"#,
                Primary::Unreadable => b"{}",
            };
            let fs = FakeFs::new()
                .with_file(&test_path(), main)
                .with_file(&test_tmp_path(), TMP_3000);
            fs.set_fail_read(matches!(kind, Primary::Unreadable));
            let store = open_fast(&fs);
            // The existing recovery or blocking path ran; the leftover is
            // neither read, adopted nor removed.
            assert_eq!(store.settings().general.interval_ms, 1000);
            assert_eq!(fs.ops().first().map(String::as_str), Some("read"));
            assert!(!fs.ops().contains(&"remove".to_string()), "{:?}", fs.ops());
            let persistence = store.state().persistence;
            match kind {
                Primary::Corrupt => assert!(
                    matches!(&persistence, Persistence::Recovered { path } if path.contains("settings.json.bad-")),
                    "{persistence:?}"
                ),
                Primary::Future => {
                    assert!(matches!(persistence, Persistence::ReadOnly { .. }));
                }
                Primary::Unreadable => {
                    assert!(matches!(persistence, Persistence::Error { .. }));
                }
            }
            let _ = store.shutdown(Duration::from_millis(500));
            assert_eq!(fs.file(&test_tmp_path()).unwrap(), TMP_3000);
        }
    }

    #[test]
    fn adopted_tmp_is_backed_up_before_atomic_write() {
        let fs = FakeFs::new().with_file(&test_tmp_path(), TMP_3000);
        let store = open_fast(&fs);
        store.flush_now(LONG).unwrap();
        // Read the file, read the leftover, copy it aside, only then save.
        assert_eq!(fs.ops(), ["read", "read", "copy", "write"]);
        let Persistence::Recovered { path } = store.state().persistence else {
            panic!("expected Recovered");
        };
        assert_eq!(fs.file(std::path::Path::new(&path)).unwrap(), TMP_3000);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn adopted_tmp_backup_survives_a_failed_write() {
        let fs = FakeFs::new().with_file(&test_tmp_path(), TMP_3000);
        fs.fail_next_writes(1_000);
        let store = open_fast(&fs);
        assert!(store.flush_now(Duration::from_millis(300)).is_err());
        assert!(matches!(
            store.state().persistence,
            Persistence::Error { .. }
        ));
        let kept = fs.files_with_prefix("settings.json.tmp.bad-");
        assert_eq!(kept.len(), 1);
        assert_eq!(fs.file(&kept[0]).unwrap(), TMP_3000);
        // The leftover stays until a save succeeds, so a restart adopts it again.
        assert_eq!(fs.file(&test_tmp_path()).unwrap(), TMP_3000);
        assert_eq!(fs.file(&test_path()), None);
    }

    #[test]
    fn adopted_tmp_backup_failure_blocks_writes() {
        let fs = FakeFs::new().with_file(&test_tmp_path(), TMP_3000);
        fs.set_fail_copy(true);
        let store = open_fast(&fs);
        assert!(matches!(
            store.state().persistence,
            Persistence::Error { .. }
        ));
        store
            .update(&json!({"general": {"intervalMs": 4000}}))
            .unwrap();
        assert!(store.flush_now(Duration::from_millis(200)).is_err());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fs.write_attempts(), 0);
        assert_eq!(fs.file(&test_tmp_path()).unwrap(), TMP_3000);
        assert!(fs.files_with_prefix("settings.json.tmp.bad-").is_empty());
    }

    #[test]
    fn snapshot_shares_the_current_value() {
        let fs = FakeFs::new();
        let store = open_fast(&fs);
        let (first, second) = (store.snapshot(), store.snapshot());
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(*first, store.settings());
        // An update that changes nothing keeps the same allocation.
        store
            .update(&json!({"general": {"intervalMs": 1000}}))
            .unwrap();
        assert!(Arc::ptr_eq(&first, &store.snapshot()));
        store
            .update(&json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        let third = store.snapshot();
        assert!(!Arc::ptr_eq(&first, &third));
        assert_eq!(third.general.interval_ms, 2000);
        // The old snapshot is unchanged.
        assert_eq!(first.general.interval_ms, 1000);
        store.shutdown(LONG).unwrap();
    }
}
