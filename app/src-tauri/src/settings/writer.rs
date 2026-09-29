//! The writer thread: coalesced atomic saves, retries, flush and shutdown.

use std::sync::Arc;
use std::time::{Duration, Instant};

use oma_core::settings::encode;

use super::store::{lock, Inner};

/// Pause between the save attempts of `shutdown`.
const SHUTDOWN_RETRY_PAUSE: Duration = Duration::from_millis(150);

/// Why a flush did not complete.
enum FlushError {
    /// Nothing will ever be written (blocked store, stopped writer).
    Permanent(String),
    /// A save attempt failed; another attempt may succeed.
    WriteFailed(String),
    Timeout,
}

impl FlushError {
    fn into_message(self) -> String {
        match self {
            Self::Permanent(reason) | Self::WriteFailed(reason) => reason,
            Self::Timeout => "timed out waiting for the settings to be saved".into(),
        }
    }
}

impl Inner {
    /// Starts the `oma-settings-writer` thread.
    pub(super) fn spawn_writer(self: &Arc<Self>) {
        let inner = self.clone();
        match std::thread::Builder::new()
            .name("oma-settings-writer".into())
            .spawn(move || inner.writer_loop())
        {
            Ok(handle) => *lock(&self.writer) = Some(handle),
            Err(err) => {
                tracing::error!(%err, "cannot start the settings writer");
                let mut core = lock(&self.core);
                core.writer_done = true;
                if core.blocked.is_none() {
                    core.blocked = Some(super::Persistence::Error {
                        reason: format!("writer: {err}"),
                    });
                }
            }
        }
    }

    /// Saves the latest revision at most once per coalescing window, retries a
    /// failure after the retry delay, and never saves an older revision than
    /// the one already saved (it is the only writer and revisions only grow).
    fn writer_loop(&self) {
        let mut core = lock(&self.core);
        loop {
            if core.stop {
                break;
            }
            if core.blocked.is_some() || !core.is_dirty() {
                core = self.cv.wait(core).unwrap_or_else(|e| e.into_inner());
                continue;
            }
            let now = Instant::now();
            let flush_wanted =
                core.flush_target > core.persisted_revision && core.flush_failed.is_none();
            let due = if flush_wanted {
                now
            } else {
                core.due(now, self.timings.coalesce)
            };
            if due > now {
                core = self
                    .cv
                    .wait_timeout(core, due - now)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
                continue;
            }

            // Snapshot under the lock, write outside it.
            let revision = core.revision;
            let bytes = serde_json::to_vec_pretty(&encode(&core.settings)).unwrap_or_default();
            core.dirty_since = None;
            drop(core);
            let result = match &self.path {
                Some(path) => self.fs.write_atomic(path, &bytes),
                None => Err(std::io::Error::other("no settings path")),
            };
            core = lock(&self.core);
            match result {
                Ok(()) => {
                    core.persisted_revision = core.persisted_revision.max(revision);
                    core.write_error = None;
                    core.retry_at = None;
                    if core.is_dirty() && core.dirty_since.is_none() {
                        core.dirty_since = Some(Instant::now());
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, revision, "cannot save the settings; will retry");
                    let now = Instant::now();
                    core.write_error = Some(err.to_string());
                    core.retry_at = Some(now + self.timings.retry);
                    core.dirty_since = Some(now);
                    if core.flush_target > core.persisted_revision {
                        core.flush_failed = Some(err.to_string());
                    }
                }
            }
            self.commit(core);
            core = lock(&self.core);
        }
        core.writer_done = true;
        drop(core);
        self.cv.notify_all();
    }

    pub(super) fn flush_now(&self, timeout: Duration) -> Result<(), String> {
        self.flush(timeout).map_err(FlushError::into_message)
    }

    fn flush(&self, timeout: Duration) -> Result<(), FlushError> {
        let deadline = Instant::now() + timeout;
        let mut core = lock(&self.core);
        if !core.is_dirty() {
            return Ok(());
        }
        match &core.blocked {
            // Read-only by design: nothing is ever written, which is not a failure.
            Some(super::Persistence::ReadOnly { .. }) => return Ok(()),
            Some(super::Persistence::Error { reason }) => {
                return Err(FlushError::Permanent(reason.clone()))
            }
            _ => {}
        }
        if core.writer_done {
            return Err(FlushError::Permanent(
                "the settings writer is stopped".into(),
            ));
        }
        let target = core.revision;
        core.flush_target = target;
        core.flush_failed = None;
        self.cv.notify_all();
        loop {
            if core.persisted_revision >= target {
                return Ok(());
            }
            if let Some(reason) = &core.flush_failed {
                return Err(FlushError::WriteFailed(reason.clone()));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(FlushError::Timeout);
            }
            core = self
                .cv
                .wait_timeout(core, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// Final flush, then stops the writer; never waits longer than `timeout`.
    /// A failed save is retried every [`SHUTDOWN_RETRY_PAUSE`] until the
    /// deadline, so a transient failure (an antivirus scanning the temporary
    /// file) does not lose the last changes.
    pub(super) fn shutdown(&self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        let flushed = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.flush(remaining) {
                Err(FlushError::WriteFailed(reason)) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break Err(reason);
                    }
                    tracing::warn!(%reason, "final settings save failed; retrying");
                    std::thread::sleep(SHUTDOWN_RETRY_PAUSE.min(remaining));
                }
                other => break other.map_err(FlushError::into_message),
            }
        };
        let mut core = lock(&self.core);
        core.stop = true;
        self.cv.notify_all();
        while !core.writer_done {
            let now = Instant::now();
            if now >= deadline {
                // A write is stuck: detach the writer instead of hanging the exit.
                drop(core);
                lock(&self.writer).take();
                return flushed.and(Err("the settings writer did not stop in time".into()));
            }
            core = self
                .cv
                .wait_timeout(core, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        drop(core);
        if let Some(handle) = lock(&self.writer).take() {
            let _ = handle.join();
        }
        flushed
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use serde_json::json;

    use crate::settings::fake_fs::{open_fast, stored_json, test_path, wait_until, FakeFs};
    use crate::settings::store::Timings;
    use crate::settings::{Persistence, SettingsStore};

    const LONG: Duration = Duration::from_secs(5);

    fn slow_window(fs: &Arc<FakeFs>, coalesce: Duration) -> SettingsStore {
        SettingsStore::open_with(
            Some(test_path()),
            fs.clone(),
            Timings {
                coalesce,
                retry: Duration::from_millis(30),
            },
        )
    }

    #[test]
    fn updates_coalesce_into_one_write() {
        let fs = FakeFs::new();
        let store = slow_window(&fs, Duration::from_millis(400));
        for value in [1500, 2000, 2500] {
            store
                .update(&json!({"general": {"intervalMs": value}}))
                .unwrap();
        }
        wait_until("the coalesced write", || {
            store.state().persisted_revision == 3
        });
        assert_eq!(fs.writes().len(), 1);
        assert_eq!(stored_json(&fs)["general"]["intervalMs"], 2500);
        assert_eq!(store.state().persistence, Persistence::Ok);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn replace_failure_keeps_dirty_and_retries() {
        let fs = FakeFs::new();
        fs.fail_next_writes(1);
        let store = open_fast(&fs);
        store
            .update(&json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        wait_until("the failed attempt", || {
            matches!(store.state().persistence, Persistence::Error { .. })
        });
        let failed = store.state();
        assert_eq!(failed.persisted_revision, 0);
        assert_eq!(failed.revision, 1);
        // The retry succeeds.
        wait_until("the retry", || store.state().persisted_revision == 1);
        let state = store.state();
        assert_eq!(state.persistence, Persistence::Ok);
        assert_eq!(state.persisted_revision, state.revision);
        assert_eq!(stored_json(&fs)["general"]["intervalMs"], 2000);
        assert_eq!(fs.write_attempts(), 2);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn older_revision_never_overwrites_newer() {
        let fs = FakeFs::new();
        let store = open_fast(&fs);
        fs.block_writes();
        store
            .update(&json!({"general": {"intervalMs": 1500}}))
            .unwrap();
        assert!(fs.wait_write_attempts(1, LONG), "the first write starts");
        // The revision-1 write is stuck; a newer revision arrives meanwhile.
        store
            .update(&json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        fs.release_writes();
        wait_until("the newest revision", || {
            store.state().persisted_revision == 2
        });
        assert_eq!(stored_json(&fs)["general"]["intervalMs"], 2000);
        let writes = fs.writes();
        let last: serde_json::Value = serde_json::from_slice(writes.last().unwrap()).unwrap();
        assert_eq!(last["general"]["intervalMs"], 2000);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn flush_now_waits_for_the_write() {
        let fs = FakeFs::new();
        // A window far longer than the test: only the flush can trigger the write.
        let store = slow_window(&fs, Duration::from_secs(60));
        store
            .update(&json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        assert_eq!(store.state().persistence, Persistence::Pending);
        assert_eq!(fs.write_attempts(), 0);
        store.flush_now(LONG).unwrap();
        let state = store.state();
        assert_eq!(state.persisted_revision, 1);
        assert_eq!(state.persistence, Persistence::Ok);
        assert_eq!(stored_json(&fs)["general"]["intervalMs"], 2000);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn flush_now_reports_a_failed_write_without_waiting_for_the_timeout() {
        let fs = FakeFs::new();
        fs.fail_next_writes(1);
        let store = slow_window(&fs, Duration::from_secs(60));
        store
            .update(&json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        let started = Instant::now();
        assert!(store.flush_now(LONG).is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        // A second flush retries at once and succeeds.
        store.flush_now(LONG).unwrap();
        assert_eq!(store.state().persisted_revision, 1);
        store.shutdown(LONG).unwrap();
    }

    #[test]
    fn shutdown_flushes_and_stops_the_writer() {
        let fs = FakeFs::new();
        let store = slow_window(&fs, Duration::from_secs(60));
        store
            .update(&json!({"general": {"intervalMs": 2500}}))
            .unwrap();
        store.shutdown(LONG).unwrap();
        assert_eq!(stored_json(&fs)["general"]["intervalMs"], 2500);
        // Later changes stay in memory.
        store
            .update(&json!({"general": {"intervalMs": 3000}}))
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(fs.writes().len(), 1);
    }

    #[test]
    fn shutdown_retries_a_failed_write_within_its_deadline() {
        let fs = FakeFs::new();
        fs.fail_next_writes(2);
        // The window and the retry delay are far longer than the test.
        let store = SettingsStore::open_with(
            Some(test_path()),
            fs.clone(),
            Timings {
                coalesce: Duration::from_secs(60),
                retry: Duration::from_secs(60),
            },
        );
        store
            .update(&json!({"general": {"intervalMs": 2500}}))
            .unwrap();
        store.shutdown(LONG).unwrap();
        assert_eq!(fs.write_attempts(), 3);
        assert_eq!(stored_json(&fs)["general"]["intervalMs"], 2500);
        assert_eq!(store.state().persisted_revision, 1);
    }

    #[test]
    fn shutdown_gives_up_on_a_persistent_failure_at_its_deadline() {
        let fs = FakeFs::new();
        fs.fail_next_writes(1_000_000);
        let store = slow_window(&fs, Duration::from_secs(60));
        store
            .update(&json!({"general": {"intervalMs": 2500}}))
            .unwrap();
        let started = Instant::now();
        let result = store.shutdown(Duration::from_millis(500));
        let elapsed = started.elapsed();
        assert!(result.is_err());
        assert!(elapsed >= Duration::from_millis(450), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
        assert!(fs.write_attempts() >= 2, "it retried");
        assert_eq!(store.state().persisted_revision, 0);
    }

    #[test]
    fn shutdown_is_bounded() {
        let fs = FakeFs::new();
        let store = open_fast(&fs);
        fs.block_writes();
        store
            .update(&json!({"general": {"intervalMs": 2000}}))
            .unwrap();
        assert!(
            fs.wait_write_attempts(1, LONG),
            "the write starts and blocks"
        );
        let started = Instant::now();
        let result = store.shutdown(Duration::from_millis(50));
        let elapsed = started.elapsed();
        assert!(result.is_err());
        assert!(elapsed >= Duration::from_millis(40), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
        fs.release_writes();
    }
}
