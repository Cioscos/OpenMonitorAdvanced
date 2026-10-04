//! The update check (spec §2.3-2.4): one GitHub request at a time, on
//! request or on a daily schedule, with its state kept next to the settings
//! in `update-state.json`. Toasts come only from automatic checks.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use oma_core::updates::{
    next_check_ms, parse_latest, should_notify, user_agent, CheckError, UpdateState, Version,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::i18n::{t, Lang};
use crate::notifier::{launch_for_about, ToastSink};
use crate::settings::{RealFs, SettingsFs, SettingsStore};

/// Emitted with an [`UpdateStatus`] at every change of state.
pub const EVENT_UPDATE_STATUS: &str = "oma:update-status";

/// The state file, in the settings folder.
const STATE_FILE: &str = "update-state.json";

/// The scheduler wakes at least this often, so a check that fell due during
/// a suspension starts within the hour.
const MAX_WAIT_MS: u64 = 3_600_000;

/// Whole-request deadline of a check.
#[cfg(windows)]
const REQUEST_DEADLINE: Duration = Duration::from_secs(10);

/// The HTTP side of a check: the status and body of the "latest release" call.
pub trait Fetch: Send + Sync {
    fn fetch(&self, user_agent: &str) -> Result<(u16, Vec<u8>), CheckError>;
}

/// The real request, over WinHTTP.
pub struct WinHttpFetch;

impl Fetch for WinHttpFetch {
    #[cfg(windows)]
    fn fetch(&self, user_agent: &str) -> Result<(u16, Vec<u8>), CheckError> {
        use oma_core::updates::{LATEST_RELEASE_URL, MAX_BODY_BYTES, REQUEST_HEADERS};
        oma_win::http::get(
            LATEST_RELEASE_URL,
            user_agent,
            &REQUEST_HEADERS,
            REQUEST_DEADLINE,
            MAX_BODY_BYTES,
        )
        .map(|response| (response.status, response.body))
    }

    #[cfg(not(windows))]
    fn fetch(&self, _user_agent: &str) -> Result<(u16, Vec<u8>), CheckError> {
        Err(CheckError::Offline)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateStateKind {
    Idle,
    Checking,
    UpToDate,
    Available,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestVersion {
    pub version: String,
}

/// What the About page shows. `latest` is the stored release when it is
/// newer than the installed version, whatever the state (an error too).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub state: UpdateStateKind,
    pub current: String,
    pub latest: Option<LatestVersion>,
    pub checked_at_ms: Option<u64>,
    /// `CheckError::category()` in the `error` state.
    pub error: Option<&'static str>,
}

/// The language of the toast, read when one is shown.
pub type LangSource = Box<dyn Fn() -> Lang + Send + Sync>;

type StatusListener = Box<dyn Fn(&UpdateStatus) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trigger {
    Manual,
    Auto,
}

/// The outcome of the last check of this session.
#[derive(Debug, Clone, Copy)]
enum Outcome {
    Idle,
    UpToDate,
    Available,
    Error(&'static str),
}

struct Inner {
    state: UpdateState,
    outcome: Outcome,
    checked_at_ms: Option<u64>,
    /// A request is running; other callers wait for its result.
    in_flight: bool,
    /// Bumped at the end of every check, for the callers waiting on it.
    generation: u64,
    auto: bool,
    /// Callers waiting on a running check (tests only).
    #[cfg(test)]
    waiting: usize,
}

pub struct UpdateService {
    fetch: Box<dyn Fetch>,
    state_path: Option<PathBuf>,
    current: Version,
    toasts: Arc<dyn ToastSink + Sync>,
    lang: LangSource,
    inner: Mutex<Inner>,
    /// Wakes the scheduler (setting changed, check finished) and the callers
    /// waiting on a running check.
    wake: Condvar,
    /// The listener, also held while a status is read and delivered, so
    /// emitted statuses never go back in time.
    listener: Mutex<Option<StatusListener>>,
}

impl UpdateService {
    /// Loads the state file (a missing or bad one counts as empty). Automatic
    /// checks start off: [`UpdateService::set_auto`] follows the setting.
    pub fn new(
        fetch: Box<dyn Fetch>,
        state_path: Option<PathBuf>,
        current: Version,
        toasts: Arc<dyn ToastSink + Sync>,
        lang: LangSource,
    ) -> Arc<Self> {
        let state = load_state(state_path.as_deref());
        // A stored release newer than this version is still news; one that
        // is not (the app was updated meanwhile) is simply gone.
        let (outcome, checked_at_ms) = match state.available(current) {
            Some(_) => (Outcome::Available, state.last_success_ms),
            None => (Outcome::Idle, None),
        };
        Arc::new(Self {
            fetch,
            state_path,
            current,
            toasts,
            lang,
            inner: Mutex::new(Inner {
                state,
                outcome,
                checked_at_ms,
                in_flight: false,
                generation: 0,
                auto: false,
                #[cfg(test)]
                waiting: 0,
            }),
            wake: Condvar::new(),
            listener: Mutex::new(None),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Called with every new status, outside the service's lock.
    pub fn on_status(&self, listener: StatusListener) {
        *self.listener.lock().unwrap_or_else(PoisonError::into_inner) = Some(listener);
    }

    /// Delivers the current status. It is read under the listener's lock,
    /// so a finished check never overwrites the `checking` of the next one.
    fn publish(&self) {
        let listener = self.listener.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(listener) = &*listener {
            listener(&self.status());
        }
    }

    pub fn status(&self) -> UpdateStatus {
        self.status_of(&self.lock())
    }

    fn status_of(&self, inner: &Inner) -> UpdateStatus {
        let (state, error) = if inner.in_flight {
            (UpdateStateKind::Checking, None)
        } else {
            match inner.outcome {
                Outcome::Idle => (UpdateStateKind::Idle, None),
                Outcome::UpToDate => (UpdateStateKind::UpToDate, None),
                Outcome::Available => (UpdateStateKind::Available, None),
                Outcome::Error(category) => (UpdateStateKind::Error, Some(category)),
            }
        };
        UpdateStatus {
            state,
            current: self.current.to_string(),
            latest: inner
                .state
                .available(self.current)
                .map(|release| LatestVersion {
                    version: release.version.to_string(),
                }),
            checked_at_ms: inner.checked_at_ms,
            error,
        }
    }

    /// The page of the available release, already validated; `None` when
    /// there is no newer release.
    pub fn release_page(&self) -> Option<String> {
        self.lock()
            .state
            .available(self.current)
            .map(|release| release.url)
    }

    /// A check on request: blocks until its result, or until the result of
    /// the check already running. Never toasts.
    pub fn check_now(&self) -> UpdateStatus {
        self.check(Trigger::Manual)
    }

    /// Follows `updates.checkAutomatically`; wakes the scheduler.
    pub fn set_auto(&self, auto: bool) {
        self.lock().auto = auto;
        self.wake.notify_all();
    }

    fn check_auto(&self) -> UpdateStatus {
        self.check(Trigger::Auto)
    }

    fn check(&self, trigger: Trigger) -> UpdateStatus {
        let mut inner = self.lock();
        if inner.in_flight {
            let generation = inner.generation;
            #[cfg(test)]
            {
                inner.waiting += 1;
            }
            while inner.generation == generation {
                inner = self
                    .wake
                    .wait(inner)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            #[cfg(test)]
            {
                inner.waiting -= 1;
            }
            return self.status_of(&inner);
        }
        inner.in_flight = true;
        drop(inner);
        self.publish();

        let agent = user_agent(&self.current.to_string());
        // A panic must not leave `in_flight` set, or every caller would wait forever.
        let result = catch_unwind(AssertUnwindSafe(|| self.fetch.fetch(&agent)))
            .unwrap_or_else(|_| {
                tracing::error!("the update request panicked");
                Err(CheckError::Invalid)
            })
            .and_then(|(status, body)| parse_latest(status, &body));
        let now = now_ms();

        let mut inner = self.lock();
        let mut toast = None;
        match result {
            Ok(release) => {
                inner.state.record_success(now, &release);
                inner.outcome = if release.version > self.current {
                    Outcome::Available
                } else {
                    Outcome::UpToDate
                };
                if should_notify(self.current, &inner.state, &release) {
                    // An automatic result toasts only if the user still wants
                    // automatic checks; a manual one is seen on the page.
                    let announce = trigger == Trigger::Auto && inner.auto;
                    if trigger == Trigger::Manual || announce {
                        inner.state.notified_version = Some(release.version.to_string());
                    }
                    if announce {
                        toast = Some(release.version);
                    }
                }
                tracing::info!(latest = %release.version, "update check done");
            }
            Err(err) => {
                tracing::info!(category = err.category(), "update check failed");
                inner.state.record_failure(now);
                inner.outcome = Outcome::Error(err.category());
            }
        }
        inner.checked_at_ms = Some(now);
        inner.in_flight = false;
        inner.generation = inner.generation.wrapping_add(1);
        // Under the lock, so two saves never cross; a failed save keeps the
        // state (and `notified_version`) in memory for this session.
        self.save(&inner.state);
        let status = self.status_of(&inner);
        drop(inner);
        self.wake.notify_all();

        if let Some(version) = toast {
            let lang = (self.lang)();
            let version = version.to_string();
            self.toasts.show(
                t(lang, "updates.toast.title", &[("version", &version)]),
                t(lang, "updates.toast.body", &[]),
                launch_for_about(),
            );
        }
        self.publish();
        status
    }

    fn save(&self, state: &UpdateState) {
        let Some(path) = &self.state_path else {
            return;
        };
        let written = serde_json::to_vec_pretty(state)
            .map_err(std::io::Error::other)
            .and_then(|bytes| RealFs.write_atomic(path, &bytes));
        if let Err(err) = written {
            tracing::warn!(%err, "cannot save the update state");
        }
    }

    /// Starts the `oma-updates` thread: it checks when [`next_check_ms`]
    /// says so (never with the setting off) and wakes at least hourly.
    pub fn spawn_scheduler(self: &Arc<Self>, started_ms: u64) {
        let service = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("oma-updates".into())
            .spawn(move || service.schedule(started_ms));
        if let Err(err) = spawned {
            tracing::warn!(%err, "cannot start the update scheduler");
        }
    }

    /// How long the scheduler sleeps at `now`; `None` to check at once.
    fn next_wait_ms(&self, inner: &Inner, now: u64, started_ms: u64) -> Option<u64> {
        match next_check_ms(now, started_ms, &inner.state, inner.auto) {
            Some(due) if due <= now && !inner.in_flight => None,
            // A manual check is running: its end wakes the scheduler.
            Some(due) if due <= now => Some(MAX_WAIT_MS),
            Some(due) => Some((due - now).min(MAX_WAIT_MS)),
            None => Some(MAX_WAIT_MS),
        }
    }

    fn schedule(&self, started_ms: u64) {
        let mut inner = self.lock();
        loop {
            let Some(wait_ms) = self.next_wait_ms(&inner, now_ms(), started_ms) else {
                drop(inner);
                self.check_auto();
                inner = self.lock();
                continue;
            };
            inner = match self
                .wake
                .wait_timeout(inner, Duration::from_millis(wait_ms))
            {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }
}

/// The stored state; a missing, unreadable or malformed file is empty.
pub fn load_state(path: Option<&Path>) -> UpdateState {
    let Some(path) = path else {
        return UpdateState::default();
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return UpdateState::default();
        }
        Err(err) => {
            tracing::warn!(%err, "cannot read the update state: starting empty");
            return UpdateState::default();
        }
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|err| {
        tracing::warn!(%err, "malformed update state: starting empty");
        UpdateState::default()
    })
}

/// `update-state.json` in the settings folder.
pub fn state_path() -> Option<PathBuf> {
    let settings = crate::settings::settings_path()?;
    Some(settings.parent()?.join(STATE_FILE))
}

/// The installed version; in debug builds a valid `fake`
/// (`OMA_UPDATE_FAKE_CURRENT`) replaces it. Release builds ignore `fake`.
pub fn current_version(package: &str, fake: Option<&str>) -> Version {
    #[cfg(debug_assertions)]
    if let Some(version) = fake.and_then(Version::parse) {
        return version;
    }
    #[cfg(not(debug_assertions))]
    let _ = fake;
    Version::parse(package).unwrap_or_else(|| {
        tracing::warn!(package, "the package version is not X.Y.Z");
        Version {
            major: 0,
            minor: 0,
            patch: 0,
        }
    })
}

/// `OMA_UPDATE_FAKE_CURRENT`, read only in debug builds.
fn fake_current() -> Option<String> {
    #[cfg(debug_assertions)]
    {
        std::env::var("OMA_UPDATE_FAKE_CURRENT").ok()
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// Builds the service, follows `updates.checkAutomatically` (like the
/// autostart and the tray), emits [`EVENT_UPDATE_STATUS`] and starts the
/// scheduler.
pub fn install(
    app: &AppHandle,
    store: &Arc<SettingsStore>,
    toasts: Arc<dyn ToastSink + Sync>,
) -> Arc<UpdateService> {
    let current = current_version(
        &app.package_info().version.to_string(),
        fake_current().as_deref(),
    );
    let lang_store = Arc::clone(store);
    let service = UpdateService::new(
        Box::new(WinHttpFetch),
        state_path(),
        current,
        toasts,
        Box::new(move || crate::tray::language_for(lang_store.snapshot().general.language)),
    );
    let handle = app.clone();
    service.on_status(Box::new(move |status| {
        let _ = handle.emit(EVENT_UPDATE_STATUS, status);
    }));
    service.set_auto(store.snapshot().updates.check_automatically);
    let listener = Arc::clone(&service);
    store.subscribe(Box::new(move |settings, _| {
        listener.set_auto(settings.updates.check_automatically)
    }));
    service.spawn_scheduler(now_ms());
    service
}

/// A check on request; waits for its result (spec §2.4) on a blocking
/// thread, not on an async worker.
#[tauri::command]
pub async fn check_updates(service: State<'_, Arc<UpdateService>>) -> Result<UpdateStatus, String> {
    let service = Arc::clone(&service);
    tauri::async_runtime::spawn_blocking(move || service.check_now())
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command(async)]
pub fn get_update_status(service: State<'_, Arc<UpdateService>>) -> UpdateStatus {
    service.status()
}

/// Opens the page of the available release, and nothing else.
#[tauri::command(async)]
pub fn open_release_page(service: State<'_, Arc<UpdateService>>) -> Result<(), String> {
    let url = service
        .release_page()
        .ok_or_else(|| "no update available".to_owned())?;
    crate::commands::shell_open(Path::new(&url)).map_err(|err| {
        tracing::warn!(%err, "cannot open the release page");
        err.to_string()
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use oma_core::updates::RELEASE_PAGE_PREFIX;

    use super::*;

    type Answer = Result<(u16, Vec<u8>), CheckError>;
    type SharedAnswer = Arc<Mutex<Answer>>;
    /// The test's ends of a [`Gate`]: "fetch entered" and "let it go".
    type GateEnds = (mpsc::Receiver<()>, mpsc::Sender<()>);

    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<(String, String, String)>>>);

    impl Recorder {
        fn toasts(&self) -> Vec<(String, String, String)> {
            self.0.lock().unwrap().clone()
        }
    }

    impl ToastSink for Recorder {
        fn show(&self, title: String, body: String, launch: String) {
            self.0.lock().unwrap().push((title, body, launch));
        }
    }

    /// Blocks each fetch until the test lets it go.
    struct Gate {
        entered: Mutex<mpsc::Sender<()>>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    struct FakeFetch {
        calls: Arc<AtomicUsize>,
        result: SharedAnswer,
        gate: Option<Gate>,
    }

    impl Fetch for FakeFetch {
        fn fetch(&self, user_agent: &str) -> Result<(u16, Vec<u8>), CheckError> {
            assert!(user_agent.starts_with("OpenMonitorAdvanced/"));
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(gate) = &self.gate {
                let _ = gate.entered.lock().unwrap().send(());
                let _ = gate.release.lock().unwrap().recv();
            }
            self.result.lock().unwrap().clone()
        }
    }

    struct Harness {
        calls: Arc<AtomicUsize>,
        result: SharedAnswer,
        toasts: Recorder,
    }

    fn release_body(version: &str) -> Vec<u8> {
        serde_json::json!({
            "tag_name": format!("v{version}"),
            "html_url": format!("{RELEASE_PAGE_PREFIX}tag/v{version}"),
            "draft": false,
            "prerelease": false,
        })
        .to_string()
        .into_bytes()
    }

    fn temp_dir() -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "oma-updates-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn v040() -> Version {
        Version::parse("0.4.0").unwrap()
    }

    /// A service on 0.4.0 whose fetch answers `result`; with `gated`, the
    /// gate's ends for the test.
    fn service(
        state_path: Option<PathBuf>,
        result: Answer,
        gated: bool,
    ) -> (Arc<UpdateService>, Harness, Option<GateEnds>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let result = Arc::new(Mutex::new(result));
        let toasts = Recorder::default();
        let (gate, ends) = if gated {
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            (
                Some(Gate {
                    entered: Mutex::new(entered_tx),
                    release: Mutex::new(release_rx),
                }),
                Some((entered_rx, release_tx)),
            )
        } else {
            (None, None)
        };
        let fetch = FakeFetch {
            calls: calls.clone(),
            result: result.clone(),
            gate,
        };
        let svc = UpdateService::new(
            Box::new(fetch),
            state_path,
            v040(),
            Arc::new(toasts.clone()),
            Box::new(|| Lang::En),
        );
        (
            svc,
            Harness {
                calls,
                result,
                toasts,
            },
            ends,
        )
    }

    fn saved(path: &Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn state_file_missing_or_garbage_is_empty() {
        let dir = temp_dir();
        let path = dir.join("update-state.json");
        assert_eq!(load_state(Some(&path)), UpdateState::default());
        assert_eq!(load_state(None), UpdateState::default());
        std::fs::write(&path, b"not json {").unwrap();
        assert_eq!(load_state(Some(&path)), UpdateState::default());
        std::fs::write(&path, br#"{"lastAttemptMs":"yesterday"}"#).unwrap();
        assert_eq!(load_state(Some(&path)), UpdateState::default());
        std::fs::write(&path, br#"{"lastAttemptMs":5}"#).unwrap();
        assert_eq!(load_state(Some(&path)).last_attempt_ms, Some(5));
    }

    #[test]
    fn manual_check_reports_available_without_toast() {
        let path = temp_dir().join("update-state.json");
        let (svc, h, _) = service(Some(path.clone()), Ok((200, release_body("0.5.0"))), false);
        svc.set_auto(true);
        let status = svc.check_now();
        assert_eq!(status.state, UpdateStateKind::Available);
        assert_eq!(status.current, "0.4.0");
        assert_eq!(status.latest.unwrap().version, "0.5.0");
        assert!(status.checked_at_ms.is_some());
        assert_eq!(status.error, None);
        assert!(h.toasts.toasts().is_empty());
        assert_eq!(saved(&path)["notifiedVersion"], "0.5.0");
    }

    #[test]
    fn up_to_date_when_latest_is_not_newer() {
        let (svc, _h, _) = service(None, Ok((200, release_body("0.4.0"))), false);
        let status = svc.check_now();
        assert_eq!(status.state, UpdateStateKind::UpToDate);
        assert_eq!(status.latest, None);
    }

    #[test]
    fn auto_check_toasts_once() {
        let path = temp_dir().join("update-state.json");
        let (svc, h, _) = service(Some(path), Ok((200, release_body("0.5.0"))), false);
        svc.set_auto(true);
        svc.check_auto();
        svc.check_auto();
        let toasts = h.toasts.toasts();
        assert_eq!(toasts.len(), 1);
        let (title, body, launch) = &toasts[0];
        assert_eq!(title, "OpenMonitor Advanced 0.5.0 available");
        assert_eq!(body, "Open About to download it");
        assert_eq!(launch, r#"{"open":"about"}"#);
        assert_eq!(h.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn auto_result_after_disable_shows_no_toast() {
        let path = temp_dir().join("update-state.json");
        let (svc, h, ends) = service(Some(path.clone()), Ok((200, release_body("0.5.0"))), true);
        let (entered, release) = ends.unwrap();
        svc.set_auto(true);
        let worker = {
            let svc = svc.clone();
            std::thread::spawn(move || svc.check_auto())
        };
        entered.recv().unwrap();
        assert_eq!(svc.status().state, UpdateStateKind::Checking);
        svc.set_auto(false);
        release.send(()).unwrap();
        let status = worker.join().unwrap();
        assert_eq!(status.state, UpdateStateKind::Available);
        assert_eq!(svc.status().state, UpdateStateKind::Available);
        assert!(h.toasts.toasts().is_empty());
        assert!(saved(&path)["notifiedVersion"].is_null());
    }

    #[test]
    fn failure_after_a_future_success_does_not_spin() {
        // The clock went back past the stored success; the check then fails.
        let path = temp_dir().join("update-state.json");
        let future = now_ms() + 50 * 3_600_000;
        std::fs::write(&path, format!(r#"{{"lastSuccessMs":{future}}}"#)).unwrap();
        let (svc, _h, _) = service(Some(path), Err(CheckError::Offline), false);
        svc.set_auto(true);
        let started = now_ms() - 3_600_000;
        assert_eq!(svc.next_wait_ms(&svc.lock(), now_ms(), started), None);
        svc.check_auto();
        let now = now_ms();
        assert_eq!(
            svc.next_wait_ms(&svc.lock(), now, started),
            Some(MAX_WAIT_MS),
            "a failed check must wait for the retry"
        );
    }

    #[test]
    fn unwritable_state_still_notifies_once_per_session() {
        // The parent "folder" is a file: the state can never be written.
        let blocker = temp_dir().join("blocker");
        std::fs::write(&blocker, b"").unwrap();
        let path = blocker.join("missing").join("update-state.json");
        let (svc, h, _) = service(Some(path.clone()), Ok((200, release_body("0.5.0"))), false);
        svc.set_auto(true);
        svc.check_auto();
        svc.check_auto();
        assert_eq!(h.toasts.toasts().len(), 1);
        assert!(!path.exists());
        let status = svc.status();
        assert_eq!(status.state, UpdateStateKind::Available);
        assert_eq!(status.latest.unwrap().version, "0.5.0");
    }

    #[test]
    fn error_maps_category() {
        let path = temp_dir().join("update-state.json");
        let (svc, h, _) = service(Some(path.clone()), Err(CheckError::Timeout), false);
        svc.set_auto(true);
        let status = svc.check_auto();
        assert_eq!(status.state, UpdateStateKind::Error);
        assert_eq!(status.error, Some("timeout"));
        assert!(status.checked_at_ms.is_some());
        assert!(h.toasts.toasts().is_empty());
        let state = saved(&path);
        assert!(state["lastAttemptMs"].is_u64());
        assert!(state["lastSuccessMs"].is_null());
    }

    #[test]
    fn concurrent_manual_checks_share_one_request() {
        let (svc, h, ends) = service(None, Ok((200, release_body("0.5.0"))), true);
        let (entered, release) = ends.unwrap();
        let first = {
            let svc = svc.clone();
            std::thread::spawn(move || svc.check_now())
        };
        entered.recv().unwrap();
        let second = {
            let svc = svc.clone();
            std::thread::spawn(move || svc.check_now())
        };
        // The second caller must be waiting before the first one finishes.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while svc.lock().waiting == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "second caller never waited"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        release.send(()).unwrap();
        // A second fetch, if any, must not block.
        drop(release);
        let (a, b) = (first.join().unwrap(), second.join().unwrap());
        assert_eq!(h.calls.load(Ordering::SeqCst), 1);
        assert_eq!(a, b);
        assert_eq!(a.state, UpdateStateKind::Available);
    }

    #[test]
    fn error_status_keeps_latest() {
        let (svc, h, _) = service(None, Ok((200, release_body("0.5.0"))), false);
        svc.check_now();
        *h.result.lock().unwrap() = Err(CheckError::Offline);
        let status = svc.check_now();
        assert_eq!(status.state, UpdateStateKind::Error);
        assert_eq!(status.error, Some("offline"));
        assert_eq!(status.latest.unwrap().version, "0.5.0");
        assert_eq!(svc.status().latest.unwrap().version, "0.5.0");
    }

    #[test]
    fn stored_release_is_available_at_start() {
        let path = temp_dir().join("update-state.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "lastSuccessMs": 7,
                "latest": {"version": "0.5.0", "url": format!("{RELEASE_PAGE_PREFIX}tag/v0.5.0")},
            })
            .to_string(),
        )
        .unwrap();
        let (svc, _h, _) = service(Some(path), Err(CheckError::Offline), false);
        let status = svc.status();
        assert_eq!(status.state, UpdateStateKind::Available);
        assert_eq!(status.latest.unwrap().version, "0.5.0");
        assert_eq!(
            svc.release_page().as_deref(),
            Some(format!("{RELEASE_PAGE_PREFIX}tag/v0.5.0").as_str())
        );
    }

    #[test]
    fn no_release_page_without_an_update() {
        let (svc, _h, _) = service(None, Ok((200, release_body("0.4.0"))), false);
        assert_eq!(svc.status().state, UpdateStateKind::Idle);
        assert_eq!(svc.release_page(), None);
        svc.check_now();
        assert_eq!(svc.release_page(), None);
    }

    #[test]
    fn status_serializes_in_camel_case() {
        let status = UpdateStatus {
            state: UpdateStateKind::UpToDate,
            current: "0.4.0".into(),
            latest: Some(LatestVersion {
                version: "0.5.0".into(),
            }),
            checked_at_ms: Some(3),
            error: None,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["state"], "upToDate");
        assert_eq!(json["latest"]["version"], "0.5.0");
        assert_eq!(json["checkedAtMs"], 3);
        assert!(json["error"].is_null());
    }

    #[test]
    fn emits_checking_then_result() {
        let (svc, _h, _) = service(None, Ok((200, release_body("0.5.0"))), false);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        svc.on_status(Box::new(move |status: &UpdateStatus| {
            sink.lock().unwrap().push(status.state)
        }));
        svc.check_now();
        assert_eq!(
            *seen.lock().unwrap(),
            [UpdateStateKind::Checking, UpdateStateKind::Available]
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    fn fake_current_only_in_debug() {
        assert_eq!(current_version("0.4.0", Some("0.2.0")).to_string(), "0.2.0");
        assert_eq!(current_version("0.4.0", Some("v0.2")).to_string(), "0.4.0");
        assert_eq!(current_version("0.4.0", None).to_string(), "0.4.0");
    }
}
