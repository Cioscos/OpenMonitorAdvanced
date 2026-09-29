//! In-memory [`SettingsFs`] for tests: it can fail `write_atomic`, `preserve`
//! or `read` on command and block a write until released. Shared by the store
//! tests and, later, the migration and service tests.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::store::Timings;
use super::{SettingsFs, SettingsStore};

#[derive(Default)]
struct State {
    files: HashMap<PathBuf, Vec<u8>>,
    /// Operation names in call order: `read`, `write`, `preserve`, `remove`.
    ops: Vec<String>,
    /// Payloads of the successful writes, in order.
    writes: Vec<Vec<u8>>,
    write_attempts: usize,
    fail_writes: usize,
    fail_preserve: bool,
    preserve_collisions: usize,
    fail_read: bool,
    block_writes: bool,
}

#[derive(Default)]
pub(crate) struct FakeFs {
    state: Mutex<State>,
    changed: Condvar,
}

impl FakeFs {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn with_file(self: Arc<Self>, path: &Path, bytes: &[u8]) -> Arc<Self> {
        self.lock().files.insert(path.to_path_buf(), bytes.to_vec());
        self
    }

    pub(crate) fn file(&self, path: &Path) -> Option<Vec<u8>> {
        self.lock().files.get(path).cloned()
    }

    /// Successful writes, in order.
    pub(crate) fn writes(&self) -> Vec<Vec<u8>> {
        self.lock().writes.clone()
    }

    pub(crate) fn write_attempts(&self) -> usize {
        self.lock().write_attempts
    }

    pub(crate) fn ops(&self) -> Vec<String> {
        self.lock().ops.clone()
    }

    /// Paths of every stored file whose name starts with `prefix`.
    pub(crate) fn files_with_prefix(&self, prefix: &str) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = self
            .lock()
            .files
            .keys()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(prefix))
            })
            .cloned()
            .collect();
        found.sort();
        found
    }

    /// The next `n` calls of `write_atomic` fail.
    pub(crate) fn fail_next_writes(&self, n: usize) {
        self.lock().fail_writes = n;
    }

    pub(crate) fn set_fail_preserve(&self, fail: bool) {
        self.lock().fail_preserve = fail;
    }

    /// The next `n` calls of `preserve` report that the target name exists.
    pub(crate) fn collide_next_preserves(&self, n: usize) {
        self.lock().preserve_collisions = n;
    }

    pub(crate) fn set_fail_read(&self, fail: bool) {
        self.lock().fail_read = fail;
    }

    /// `write_atomic` blocks (before touching the file) until released.
    pub(crate) fn block_writes(&self) {
        self.lock().block_writes = true;
    }

    pub(crate) fn release_writes(&self) {
        self.lock().block_writes = false;
        self.changed.notify_all();
    }

    /// Waits until `write_atomic` has been entered at least `n` times.
    pub(crate) fn wait_write_attempts(&self, n: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        while state.write_attempts < n {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            state = self
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

impl SettingsFs for FakeFs {
    fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        let mut state = self.lock();
        state.ops.push("read".into());
        if state.fail_read {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected read",
            ));
        }
        Ok(state.files.get(path).cloned())
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut state = self.lock();
        state.write_attempts += 1;
        self.changed.notify_all();
        while state.block_writes {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.ops.push("write".into());
        if state.fail_writes > 0 {
            state.fail_writes -= 1;
            return Err(io::Error::other("injected write"));
        }
        state.files.insert(path.to_path_buf(), bytes.to_vec());
        state.writes.push(bytes.to_vec());
        Ok(())
    }

    fn preserve(&self, path: &Path, to: &Path) -> io::Result<()> {
        let mut state = self.lock();
        state.ops.push("preserve".into());
        if state.fail_preserve {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected preserve",
            ));
        }
        if state.preserve_collisions > 0 {
            state.preserve_collisions -= 1;
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        if state.files.contains_key(to) {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        let bytes = state
            .files
            .remove(path)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        state.files.insert(to.to_path_buf(), bytes);
        Ok(())
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        let mut state = self.lock();
        state.ops.push("remove".into());
        state.files.remove(path);
        Ok(())
    }
}

/// The settings path used by the tests.
pub(crate) fn test_path() -> PathBuf {
    PathBuf::from("C:/oma-test/settings.json")
}

/// Short timings for tests that do not depend on the coalescing window.
pub(crate) fn fast() -> Timings {
    Timings {
        coalesce: Duration::from_millis(20),
        retry: Duration::from_millis(30),
    }
}

pub(crate) fn open_fast(fs: &Arc<FakeFs>) -> SettingsStore {
    SettingsStore::open_with(Some(test_path()), fs.clone(), fast())
}

/// Polls `condition` until it holds; panics after 10 s.
pub(crate) fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The JSON document stored at the test path.
pub(crate) fn stored_json(fs: &FakeFs) -> Value {
    serde_json::from_slice(&fs.file(&test_path()).expect("settings file")).expect("valid JSON")
}
