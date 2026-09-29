//! Start with Windows: keeps the user's `Run` entry in step with
//! `tray.autostart` and tells the UI what Windows makes of it.

use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use tauri::State;

use crate::settings::{Effect, EffectStatus, SettingsStore};

#[cfg(windows)]
pub use oma_win::autostart::Effective;

/// Off Windows there is no `Run` key; the states mirror `oma_win`'s.
#[cfg(not(windows))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Effective {
    NotConfigured,
    Enabled,
    DisabledByWindows,
    Unknown,
}

/// The app's start-up entry, as far as the shell needs it.
pub trait StartupEntry: Send + Sync {
    /// Whether the app's entry exists.
    fn configured(&self) -> io::Result<bool>;
    /// Creates (`true`) or removes (`false`) the entry.
    fn set(&self, on: bool) -> io::Result<()>;
    /// Whether Windows will start the entry.
    fn effective(&self) -> Effective;
}

/// What the Settings screen shows about the start-up entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutostartStatus {
    pub configured: bool,
    pub effective: Effective,
    pub error: Option<String>,
}

/// Follows `tray.autostart` and owns the last value acted on.
pub struct Autostart {
    store: Arc<SettingsStore>,
    entry: Arc<dyn StartupEntry>,
    /// The setting last acted on. Held while the entry is touched (a quick
    /// local call), never across a call into the store.
    last: Mutex<bool>,
}

impl Autostart {
    /// Subscribes to `tray.autostart`. The setting seen now is the starting
    /// point (nothing is written for it), and the store is checked once right
    /// after subscribing so a change made in between is not lost.
    ///
    /// The listener may run on the settings writer thread: it compares the new
    /// value with the last one acted on and, only on a change, makes one quick
    /// registry call. It never waits for the store.
    pub fn follow(store: &Arc<SettingsStore>, entry: Arc<dyn StartupEntry>) -> Arc<Self> {
        let autostart = Arc::new(Self {
            store: Arc::clone(store),
            entry,
            last: Mutex::new(store.settings().tray.autostart),
        });
        let listener = Arc::clone(&autostart);
        store.subscribe(Box::new(move |settings, _| {
            listener.apply(settings.tray.autostart)
        }));
        autostart.apply(store.settings().tray.autostart);
        autostart
    }

    fn apply(&self, want: bool) {
        let failure = {
            let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            if *last == want {
                return;
            }
            match self.entry.set(want) {
                Ok(()) => {
                    *last = want;
                    None
                }
                Err(err) => {
                    // What the registry holds now is the truth; a failed
                    // removal leaves the entry on, so the setting stays on.
                    let actual = self.entry.configured().unwrap_or(!want);
                    *last = actual;
                    Some((err, actual))
                }
            }
        };
        // Outside the lock: the store delivers to its listeners, this one included.
        match failure {
            None => self
                .store
                .set_effect(Effect::Autostart, EffectStatus::Applied),
            Some((err, actual)) => {
                tracing::warn!(%err, want, "cannot change the start-with-Windows entry");
                if actual != want {
                    self.store.update_with(|s| s.tray.autostart = actual);
                }
                self.store.set_effect(
                    Effect::Autostart,
                    EffectStatus::Failed {
                        reason: err.to_string(),
                    },
                );
            }
        }
    }

    /// Reads the registry again for the Settings screen and, when the entry
    /// no longer matches `tray.autostart`, puts the setting right without
    /// writing to the registry.
    pub fn refresh(&self) -> AutostartStatus {
        match self.entry.configured() {
            Ok(configured) => {
                {
                    let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
                    *last = configured;
                }
                if self.store.settings().tray.autostart != configured {
                    self.store.update_with(|s| s.tray.autostart = configured);
                }
                AutostartStatus {
                    configured,
                    effective: self.entry.effective(),
                    error: None,
                }
            }
            Err(err) => AutostartStatus {
                configured: self.store.settings().tray.autostart,
                effective: Effective::Unknown,
                error: Some(err.to_string()),
            },
        }
    }
}

/// Re-reads the registry and, if the entry no longer matches the setting,
/// puts the setting right. Blocking work stays off the main thread.
#[tauri::command(async)]
pub fn refresh_autostart(autostart: State<'_, Arc<Autostart>>) -> AutostartStatus {
    autostart.refresh()
}

/// The `Run` value of this executable in the user's registry.
#[cfg(windows)]
struct RunEntry {
    key: oma_win::autostart::RunKey,
    exe: std::path::PathBuf,
}

#[cfg(windows)]
impl StartupEntry for RunEntry {
    fn configured(&self) -> io::Result<bool> {
        self.key.read().map(|value| value.is_some())
    }

    fn set(&self, on: bool) -> io::Result<()> {
        if on {
            self.key.write(&self.exe)
        } else {
            self.key.remove()
        }
    }

    fn effective(&self) -> Effective {
        self.key.effective()
    }
}

/// The registry-backed entry of this executable.
#[cfg(windows)]
pub fn system_entry() -> io::Result<Arc<dyn StartupEntry>> {
    Ok(Arc::new(RunEntry {
        key: oma_win::autostart::RunKey::production(),
        exe: std::env::current_exe()?,
    }))
}

/// Off Windows there is no `Run` key: nothing is configured and nothing can be.
#[cfg(not(windows))]
struct Unsupported;

#[cfg(not(windows))]
impl StartupEntry for Unsupported {
    fn configured(&self) -> io::Result<bool> {
        Ok(false)
    }

    fn set(&self, _on: bool) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    fn effective(&self) -> Effective {
        Effective::NotConfigured
    }
}

#[cfg(not(windows))]
pub fn system_entry() -> io::Result<Arc<dyn StartupEntry>> {
    Ok(Arc::new(Unsupported))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::settings::fake_fs::{open_fast, FakeFs};

    #[derive(Default)]
    struct FakeEntry {
        on: Mutex<bool>,
        fail: AtomicBool,
        calls: Mutex<Vec<bool>>,
    }

    impl FakeEntry {
        fn is_on(&self) -> bool {
            *self.on.lock().unwrap()
        }

        fn calls(&self) -> Vec<bool> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl StartupEntry for FakeEntry {
        fn configured(&self) -> io::Result<bool> {
            Ok(self.is_on())
        }

        fn set(&self, on: bool) -> io::Result<()> {
            self.calls.lock().unwrap().push(on);
            if self.fail.load(Ordering::SeqCst) {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            *self.on.lock().unwrap() = on;
            Ok(())
        }

        fn effective(&self) -> Effective {
            if self.is_on() {
                Effective::Enabled
            } else {
                Effective::NotConfigured
            }
        }
    }

    fn rig() -> (Arc<SettingsStore>, Arc<FakeEntry>, Arc<Autostart>) {
        let store = Arc::new(open_fast(&FakeFs::new()));
        let entry = Arc::new(FakeEntry::default());
        let autostart = Autostart::follow(&store, entry.clone());
        (store, entry, autostart)
    }

    fn status(store: &SettingsStore) -> EffectStatus {
        store.state().apply_status.autostart
    }

    #[test]
    fn the_entry_follows_the_setting_and_reports_applied() {
        let (store, entry, _autostart) = rig();
        assert_eq!(status(&store), EffectStatus::Idle, "nothing changed yet");
        assert!(entry.calls().is_empty(), "following alone writes nothing");

        store.update_with(|s| s.tray.autostart = true);
        assert!(entry.is_on());
        assert_eq!(status(&store), EffectStatus::Applied);

        store.update_with(|s| s.tray.autostart = false);
        assert!(!entry.is_on());
        assert_eq!(entry.calls(), vec![true, false]);
    }

    #[test]
    fn other_settings_leave_the_entry_alone() {
        let (store, entry, _autostart) = rig();
        store.update_with(|s| s.general.interval_ms = 2_000);
        store.update_with(|s| s.tray.close_to_tray = false);
        assert!(entry.calls().is_empty());
        assert_eq!(status(&store), EffectStatus::Idle);
    }

    #[test]
    fn failed_write_is_not_reported_as_applied() {
        let (store, entry, _autostart) = rig();
        entry.fail.store(true, Ordering::SeqCst);

        store.update_with(|s| s.tray.autostart = true);

        assert!(!entry.is_on());
        assert!(
            !store.settings().tray.autostart,
            "the setting goes back to what the registry holds"
        );
        assert!(matches!(status(&store), EffectStatus::Failed { .. }));
        assert_eq!(entry.calls(), vec![true], "the revert writes nothing");
    }

    #[test]
    fn a_failed_removal_keeps_the_setting_on() {
        let (store, entry, _autostart) = rig();
        store.update_with(|s| s.tray.autostart = true);
        entry.fail.store(true, Ordering::SeqCst);

        store.update_with(|s| s.tray.autostart = false);

        assert!(entry.is_on());
        assert!(store.settings().tray.autostart);
        assert!(matches!(status(&store), EffectStatus::Failed { .. }));
    }

    #[test]
    fn refresh_realigns_the_setting() {
        let (store, entry, autostart) = rig();
        store.update_with(|s| s.tray.autostart = true);
        assert_eq!(entry.calls(), vec![true]);

        // The user removes the entry behind our back.
        *entry.on.lock().unwrap() = false;
        let reading = autostart.refresh();
        assert_eq!(
            reading,
            AutostartStatus {
                configured: false,
                effective: Effective::NotConfigured,
                error: None
            }
        );
        assert!(!store.settings().tray.autostart);
        assert_eq!(entry.calls(), vec![true], "realigning does not write");

        // And the other way round.
        *entry.on.lock().unwrap() = true;
        let reading = autostart.refresh();
        assert!(reading.configured);
        assert_eq!(reading.effective, Effective::Enabled);
        assert!(store.settings().tray.autostart);
        assert_eq!(entry.calls(), vec![true], "still nothing written");
    }

    #[test]
    fn refresh_of_a_matching_entry_changes_nothing() {
        let (store, _entry, autostart) = rig();
        let revision = store.state().revision;
        let reading = autostart.refresh();
        assert!(!reading.configured);
        assert_eq!(store.state().revision, revision);
    }
}
