//! Stress session history, crash journal and recovery (M8a1, spec §8.1, §8.3).
//!
//! Sessions live in `<root>\stress\AAAAMMGG-HHMMSS-<uuid>.json`, the journal in
//! `<root>\journal.json`. Ids and file names are checked before any path is built,
//! and a file read back is capped, so a hostile or huge file cannot escape the
//! folder or blow up memory.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use oma_core::load::{
    decide, is_session_file_name, is_session_id, parse_journal, parse_session, prune,
    session_file_name, summary, Component, FormatError, Journal, Objective, OutcomeDetail,
    OutcomeFacts, Preset, Session, SessionEvent, SessionSummary, StartRequest, KEEP_SESSIONS,
    MAX_ERRORS,
};
use oma_ipc::load::Plan;
use oma_win::eventlog::{EventProvider, SystemEvent};

use crate::overlay::store::write_file;

const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SAMPLES: usize = 20_000;
const MAX_EVENTS: usize = 1_000;
/// `crash_evidence` returns at most this many events.
const EVIDENCE_LIMIT: usize = 100;

pub struct PerformanceStore {
    root: PathBuf,
    /// File names already warned about in this run.
    warned: Mutex<BTreeSet<String>>,
}

impl PerformanceStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            warned: Mutex::new(BTreeSet::new()),
        }
    }

    fn dir(&self) -> PathBuf {
        self.root.join("stress")
    }

    fn journal_path(&self) -> PathBuf {
        self.root.join("journal.json")
    }

    /// Sidecar with the number of starts that failed to save the recovered session.
    fn attempts_path(&self) -> PathBuf {
        self.root.join("journal.attempts")
    }

    /// Valid session file names in the folder, newest first.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| is_session_file_name(n))
            .collect();
        names.sort_unstable_by(|a, b| b.cmp(a));
        names
    }

    fn read(&self, name: &str) -> Result<Session, String> {
        let path = self.dir().join(name);
        let len = fs::metadata(&path).map_err(|e| e.to_string())?.len();
        if len > MAX_FILE_BYTES {
            return Err(format!("file too large ({len} bytes)"));
        }
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;
        let mut s = parse_session(&bytes).map_err(|e| e.to_string())?;
        // `AAAAMMGG-HHMMSS-` is 16 bytes; `is_session_file_name` checked the form.
        if name.get(16..name.len() - 5) != Some(s.id.as_str()) {
            return Err("session id does not match the file name".into());
        }
        cap(&mut s);
        Ok(s)
    }

    /// Atomic write, then the history is pruned to `KEEP_SESSIONS`.
    pub fn save(&self, session: &Session) -> io::Result<()> {
        let name = session_file_name(&session.started_at, &session.id);
        if !is_session_file_name(&name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "session id or start time out of form",
            ));
        }
        let json = serde_json::to_vec(session).map_err(io::Error::other)?;
        write_file(&self.dir().join(&name), &json)?;
        self.prune_keeping(Some(&name));
        Ok(())
    }

    /// Removes the oldest sessions beyond `KEEP_SESSIONS`.
    pub fn prune_now(&self) {
        self.prune_keeping(None);
    }

    /// Never removes `keep`, the file just written.
    fn prune_keeping(&self, keep: Option<&str>) {
        for name in prune(&self.names(), KEEP_SESSIONS) {
            if keep == Some(name.as_str()) {
                continue;
            }
            if let Err(err) = fs::remove_file(self.dir().join(&name)) {
                tracing::warn!(%err, %name, "cannot prune a stress session");
            }
        }
    }

    /// Newest first. Unreadable and future-format files are skipped, with one
    /// warning per file and run.
    pub fn list(&self) -> Vec<SessionSummary> {
        let mut out = Vec::new();
        for name in self.names() {
            match self.read(&name) {
                Ok(s) => out.push(summary(&s)),
                Err(err) => {
                    let first = self.warned.lock().is_ok_and(|mut w| w.insert(name.clone()));
                    if first {
                        tracing::warn!(%err, %name, "skipping a stress session");
                    }
                }
            }
        }
        out
    }

    /// The file for `id`, found by listing the folder (no path from the id).
    fn find(&self, id: &str) -> io::Result<Option<String>> {
        if !is_session_id(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a session id",
            ));
        }
        let suffix = format!("-{id}.json");
        Ok(self.names().into_iter().find(|n| n.ends_with(&suffix)))
    }

    pub fn load(&self, id: &str) -> io::Result<Option<Session>> {
        match self.find(id)? {
            Some(name) => self.read(&name).map(Some).map_err(io::Error::other),
            None => Ok(None),
        }
    }

    pub fn delete(&self, id: &str) -> io::Result<()> {
        match self.find(id)? {
            Some(name) => fs::remove_file(self.dir().join(name)),
            None => Ok(()),
        }
    }

    pub fn write_journal(&self, j: &Journal) -> io::Result<()> {
        write_file(
            &self.journal_path(),
            &serde_json::to_vec(j).map_err(io::Error::other)?,
        )
    }

    /// Also removes the retry counter.
    pub fn delete_journal(&self) -> io::Result<()> {
        let _ = fs::remove_file(self.attempts_path());
        match fs::remove_file(self.journal_path()) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }

    /// `None` when there is no journal.
    pub fn read_journal(&self) -> Option<Result<Journal, FormatError>> {
        let path = self.journal_path();
        match fs::metadata(&path) {
            Err(_) => return None,
            Ok(m) if m.len() > MAX_FILE_BYTES => {
                return Some(Err(FormatError::Invalid(serde::de::Error::custom(
                    "journal too large",
                ))))
            }
            Ok(_) => {}
        }
        let bytes = fs::read(&path).ok()?;
        Some(parse_journal(&bytes))
    }

    /// Turns a journal left by a test that did not end into a saved session
    /// (DA14). `boot_ms` is the system boot time (Unix ms). Returns the
    /// recovered session's summary; a broken journal is removed and gives `None`.
    ///
    /// `OutcomeDetail.phase` is the journal's `phaseIndex` and `params["phase"]`
    /// the same index plus one (the number a person counts).
    pub fn recover(
        &self,
        boot_ms: i64,
        evidence: impl Fn(&str) -> io::Result<Vec<SystemEvent>>,
        app_version: &str,
    ) -> Option<SessionSummary> {
        let journal = match self.read_journal()? {
            Ok(j) => j,
            Err(err) => {
                tracing::warn!(%err, "removing a broken stress journal");
                let _ = self.delete_journal();
                return None;
            }
        };
        if !is_session_id(&journal.session_id) {
            tracing::warn!("removing a stress journal with a bad session id");
            let _ = self.delete_journal();
            return None;
        }
        // A bad time is treated as an app close, without evidence.
        let updated_ms = parse_rfc3339_ms(&journal.updated_at);

        let mut s = match self.load(&journal.session_id) {
            Ok(Some(s)) => s,
            Ok(None) => minimal_session(&journal, app_version),
            Err(err) => {
                // Never a second session under the same id.
                tracing::warn!(%err, "saved stress session unreadable, dropping the journal");
                let _ = self.delete_journal();
                return None;
            }
        };
        if s.outcome.is_some() {
            // Already closed: only the journal was left behind.
            let _ = self.delete_journal();
            return Some(summary(&s));
        }

        let system_crash = updated_ms.is_some_and(|u| boot_ms > u);
        let facts = OutcomeFacts {
            system_crash,
            app_closed: !system_crash,
            errors: s.errors.len() as u64 + s.errors_dropped,
            ..Default::default()
        };
        let (outcome, key) = decide(&facts);
        let mut params = key.params;
        params.insert("phase".into(), (journal.phase_index + 1).to_string());
        s.outcome = Some(outcome);
        s.outcome_detail = Some(OutcomeDetail {
            verdict: key.key.to_string(),
            params,
            phase: Some(journal.phase_index),
            kernel: journal.kernel,
            core: journal.core,
            temp_c: None,
            clock_mhz: None,
            at_ms: None,
        });
        if updated_ms.is_some() {
            s.ended_at = Some(journal.updated_at.clone());
        }

        let started_ms = parse_rfc3339_ms(&s.started_at);
        if let Some(updated_ms) = updated_ms {
            // From a minute before the last update, but not before the test began.
            let since = (updated_ms - 60_000).max(started_ms.unwrap_or(i64::MIN));
            match evidence(&to_rfc3339(since)) {
                Ok(events) => add_evidence(&mut s, &events, started_ms.unwrap_or(updated_ms)),
                Err(err) => {
                    tracing::warn!(%err, "crash evidence unreadable");
                    s.whea.unreadable = true;
                }
            }
        }

        if let Err(err) = self.save(&s) {
            self.save_failed(&err);
            return None;
        }
        let _ = self.delete_journal();
        Some(summary(&s))
    }
}

/// Starts that may fail to save a recovered session before the journal is dropped.
const MAX_SAVE_ATTEMPTS: u32 = 3;

impl PerformanceStore {
    /// `InvalidInput` can never succeed: the journal goes at once. Other I/O
    /// errors keep it for the next start, counted in `journal.attempts`.
    fn save_failed(&self, err: &io::Error) {
        let attempts = fs::read_to_string(self.attempts_path())
            .ok()
            .and_then(|t| t.trim().parse::<u32>().ok())
            .unwrap_or(0)
            + 1;
        if err.kind() == io::ErrorKind::InvalidInput || attempts >= MAX_SAVE_ATTEMPTS {
            tracing::warn!(%err, attempts, "cannot save the recovered stress session, dropping the journal");
            let _ = self.delete_journal();
        } else {
            tracing::warn!(%err, attempts, "cannot save the recovered stress session, will retry");
            let _ = fs::write(self.attempts_path(), attempts.to_string());
        }
    }
}

/// Bounds a session read from disk.
fn cap(s: &mut Session) {
    if s.errors.len() > MAX_ERRORS {
        s.errors_dropped += (s.errors.len() - MAX_ERRORS) as u64;
        s.errors.truncate(MAX_ERRORS);
    }
    s.samples.truncate(MAX_SAMPLES);
    if s.events.len() > MAX_EVENTS {
        let extra = s.events.len() - MAX_EVENTS;
        s.events.drain(..extra);
        s.events_dropped += extra as u64;
    }
}

fn push_event(s: &mut Session, at_ms: u64, code: &str, params: &[(&str, String)]) {
    if s.events.len() >= MAX_EVENTS {
        s.events.remove(0);
        s.events_dropped += 1;
    }
    s.events.push(SessionEvent {
        at_ms,
        code: code.to_string(),
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    });
}

fn add_evidence(s: &mut Session, events: &[SystemEvent], started_ms: i64) {
    for e in events {
        let at_ms = parse_rfc3339_ms(&e.time_utc).map_or(0, |t| (t - started_ms).max(0) as u64);
        let rec = ("record", e.record_id.to_string());
        match e.provider {
            EventProvider::Whea => {
                // Already counted live while the test ran.
                if s.whea.last_record.is_some_and(|last| e.record_id <= last) {
                    continue;
                }
                *s.whea.by_id.entry(e.event_id).or_default() += 1;
                let mut p = vec![rec];
                if let Some(apic) = e.apic_id {
                    *s.whea.by_apic.entry(apic).or_default() += 1;
                    p.push(("apic", apic.to_string()));
                }
                push_event(s, at_ms, &format!("whea{}", e.event_id), &p);
            }
            EventProvider::BugCheck => push_event(s, at_ms, "bugcheck", &[rec]),
            EventProvider::KernelPower => push_event(s, at_ms, "kernelPower41", &[rec]),
        }
    }
    if events.len() == EVIDENCE_LIMIT {
        let at = s.events.last().map_or(0, |e| e.at_ms);
        push_event(s, at, "events_truncated", &[]);
    }
}

/// A session with only what the journal knows (no intermediate save was made).
fn minimal_session(j: &Journal, app_version: &str) -> Session {
    // ponytail: the journal has no start time, so `updatedAt` stands in; it only names the file.
    let request = StartRequest {
        component: Component::Cpu,
        objective: Objective::Normal,
        preset: Preset::Standard,
        custom: None,
        retry_core: None,
    };
    Session {
        format: oma_core::load::FORMAT,
        id: j.session_id.clone(),
        started_at: j.updated_at.clone(),
        ended_at: None,
        component: request.component,
        device: String::new(),
        objective: request.objective,
        preset: request.preset,
        request,
        plan: Plan {
            seed: 0,
            ram_bytes: 0,
            phases: vec![],
        },
        outcome: None,
        outcome_detail: None,
        phases: vec![],
        cores: vec![],
        errors: vec![],
        errors_dropped: 0,
        events_dropped: 0,
        whea: Default::default(),
        stats: Default::default(),
        samples: vec![],
        events: vec![],
        app_version: app_version.to_string(),
        load_version: None,
    }
}

/// `YYYY-MM-DDTHH:MM:SS[.fff]Z` to Unix ms.
fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let fraction_ok = b.len() == 20
        || (b.len() > 21 && b[19] == b'.' && b[20..b.len() - 1].iter().all(u8::is_ascii_digit));
    if b.len() < 20
        || !fraction_ok
        || !s.ends_with('Z')
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (h, mi, sec) = (n(11..13)?, n(14..16)?, n(17..19)?);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dim = match mo {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=12).contains(&mo) || !(1..=dim).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // Days from civil (proleptic Gregorian).
    let y = y - i64::from(mo <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((((days * 24 + h) * 60 + mi) * 60 + sec) * 1000)
}

fn to_rfc3339(unix_ms: i64) -> String {
    let t = oma_core::csv::local_time(unix_ms.max(0) as u64, 0);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const ID: &str = "0b9f6c1e-7d2a-4c53-9a1e-3f5d8e2b7a10";
    const UPDATED: &str = "2026-10-06T14:10:00Z";

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "oma-perf-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn journal(id: &str, updated: &str) -> Journal {
        Journal {
            format: 1,
            session_id: id.into(),
            plan_summary: "cpu".into(),
            phase_index: 2,
            kernel: None,
            core: None,
            updated_at: updated.into(),
            clean_end: false,
        }
    }

    fn session(id: &str, started: &str) -> Session {
        minimal_session(&journal(id, started), "0.0.0")
    }

    fn uuid(n: u32) -> String {
        format!("00000000-0000-4000-8000-{n:012x}")
    }

    fn ev(provider: EventProvider, id: u32, rec: u64, apic: Option<u32>) -> SystemEvent {
        SystemEvent {
            record_id: rec,
            provider,
            event_id: id,
            time_utc: UPDATED.into(),
            apic_id: apic,
        }
    }

    fn none(_: &str) -> io::Result<Vec<SystemEvent>> {
        Ok(vec![])
    }

    #[test]
    fn rfc3339_round_trips() {
        let ms = parse_rfc3339_ms(UPDATED).unwrap();
        assert_eq!(to_rfc3339(ms), UPDATED);
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_ms("garbage"), None);
    }

    #[test]
    fn save_list_load_delete_round_trip() {
        let store = PerformanceStore::new(temp_dir("rt"));
        let a = session(ID, "2026-10-06T14:03:09Z");
        let b = session(&uuid(1), "2026-10-07T10:00:00Z");
        store.save(&a).unwrap();
        store.save(&b).unwrap();
        let ids: Vec<String> = store.list().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, [uuid(1), ID.to_string()]);
        assert_eq!(store.load(ID).unwrap().unwrap(), a);
        assert!(store.load(&uuid(9)).unwrap().is_none());
        store.delete(ID).unwrap();
        assert!(store.load(ID).unwrap().is_none());
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn prune_keeps_500() {
        let store = PerformanceStore::new(temp_dir("prune"));
        for i in 0..503u32 {
            let started = format!("2026-01-01T00:{:02}:{:02}Z", i / 60, i % 60);
            let s = session(&uuid(i), &started);
            // Written directly: `save` would prune on every call.
            let name = session_file_name(&s.started_at, &s.id);
            write_file(&store.dir().join(name), &serde_json::to_vec(&s).unwrap()).unwrap();
        }
        assert_eq!(store.names().len(), 503);
        store.prune_now();
        let names = store.names();
        assert_eq!(names.len(), 500);
        // The three oldest are gone.
        assert!(!names
            .iter()
            .any(|n| n.ends_with(&format!("{}.json", uuid(0)))));
        assert!(names
            .iter()
            .any(|n| n.ends_with(&format!("{}.json", uuid(502)))));
    }

    #[test]
    fn ids_outside_the_session_form_are_rejected() {
        let store = PerformanceStore::new(temp_dir("ids"));
        store.save(&session(ID, "2026-10-06T14:03:09Z")).unwrap();
        let before = store.names();
        for bad in [
            r"..\..\x",
            r"C:\x",
            "a/b",
            "0B9F6C1E-7D2A-4C53-9A1E-3F5D8E2B7A10",
        ] {
            assert_eq!(
                store.load(bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(
                store.delete(bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert_eq!(store.names(), before);
        let mut bad = session(ID, "2026-10-06T14:03:09Z");
        bad.id = "../x".into();
        assert_eq!(
            store.save(&bad).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(store.names(), before);
    }

    #[test]
    fn future_format_sessions_are_skipped() {
        let store = PerformanceStore::new(temp_dir("future"));
        store.save(&session(ID, "2026-10-06T14:03:09Z")).unwrap();
        let mut v = serde_json::to_value(session(&uuid(2), "2026-10-07T10:00:00Z")).unwrap();
        v["format"] = 99.into();
        let name = session_file_name("2026-10-07T10:00:00Z", &uuid(2));
        write_file(&store.dir().join(name), v.to_string().as_bytes()).unwrap();
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.warned.lock().unwrap().len(), 1);
        store.list();
        assert_eq!(store.warned.lock().unwrap().len(), 1);
    }

    #[test]
    fn journal_older_than_boot_is_system_crash() {
        let store = PerformanceStore::new(temp_dir("sys"));
        store.save(&session(ID, "2026-10-06T14:03:09Z")).unwrap();
        store.write_journal(&journal(ID, UPDATED)).unwrap();
        let boot = parse_rfc3339_ms(UPDATED).unwrap() + 5_000;
        let sum = store.recover(boot, none, "1.0").unwrap();
        assert_eq!(sum.verdict.as_deref(), Some("system_crash"));
        assert_eq!(sum.params["phase"], "3");
        let s = store.load(ID).unwrap().unwrap();
        assert_eq!(s.outcome_detail.unwrap().phase, Some(2));
        assert!(store.read_journal().is_none());
    }

    #[test]
    fn journal_newer_than_boot_is_app_closed() {
        let store = PerformanceStore::new(temp_dir("app"));
        store.save(&session(ID, "2026-10-06T14:03:09Z")).unwrap();
        store.write_journal(&journal(ID, UPDATED)).unwrap();
        let boot = parse_rfc3339_ms(UPDATED).unwrap() - 3_600_000;
        let sum = store.recover(boot, none, "1.0").unwrap();
        assert_eq!(sum.verdict.as_deref(), Some("crashed_app"));
        assert!(store.read_journal().is_none());
    }

    #[test]
    fn recovery_adds_whea_and_bugcheck_events() {
        let store = PerformanceStore::new(temp_dir("ev"));
        store.save(&session(ID, "2026-10-06T14:03:09Z")).unwrap();
        store.write_journal(&journal(ID, UPDATED)).unwrap();
        let seen = std::cell::RefCell::new(String::new());
        let evidence = |since: &str| {
            *seen.borrow_mut() = since.to_string();
            Ok(vec![
                ev(EventProvider::Whea, 18, 7, Some(4)),
                ev(EventProvider::BugCheck, 1001, 8, None),
                ev(EventProvider::KernelPower, 41, 9, None),
            ])
        };
        store.recover(i64::MAX, evidence, "1.0").unwrap();
        assert_eq!(*seen.borrow(), "2026-10-06T14:09:00Z");
        let s = store.load(ID).unwrap().unwrap();
        let codes: Vec<_> = s.events.iter().map(|e| e.code.as_str()).collect();
        assert_eq!(codes, ["whea18", "bugcheck", "kernelPower41"]);
        assert_eq!(s.whea.by_id[&18], 1);
        assert_eq!(s.whea.by_apic[&4], 1);
    }

    #[test]
    fn full_evidence_is_marked_truncated() {
        let store = PerformanceStore::new(temp_dir("trunc"));
        store.write_journal(&journal(ID, UPDATED)).unwrap();
        let many = |_: &str| {
            Ok((0..100)
                .map(|i| ev(EventProvider::Whea, 17, i, None))
                .collect())
        };
        store.recover(0, many, "1.0").unwrap();
        let s = store.load(ID).unwrap().unwrap();
        assert_eq!(s.events.last().unwrap().code, "events_truncated");
    }

    #[test]
    fn corrupt_journal_is_removed_and_logged() {
        let store = PerformanceStore::new(temp_dir("bad"));
        fs::write(store.journal_path(), b"{\"format\":1,\"sess").unwrap();
        assert!(store.read_journal().unwrap().is_err());
        assert!(store.recover(0, none, "1.0").is_none());
        assert!(store.read_journal().is_none());
        assert!(!store.journal_path().exists());
    }

    #[test]
    fn recovery_without_saved_session_builds_one_from_the_journal() {
        let store = PerformanceStore::new(temp_dir("min"));
        store.write_journal(&journal(ID, UPDATED)).unwrap();
        let sum = store.recover(0, none, "1.2.3").unwrap();
        assert_eq!(sum.id, ID);
        let s = store.load(ID).unwrap().unwrap();
        assert_eq!(s.app_version, "1.2.3");
        assert_eq!(s.ended_at.as_deref(), Some(UPDATED));
        assert!(store.read_journal().is_none());
    }

    #[test]
    fn unreadable_saved_session_drops_the_journal_without_a_second_session() {
        let store = PerformanceStore::new(temp_dir("unreadable"));
        let mut v = serde_json::to_value(session(ID, "2026-10-06T14:03:09Z")).unwrap();
        v["format"] = 99.into();
        let name = session_file_name("2026-10-06T14:03:09Z", ID);
        write_file(&store.dir().join(&name), v.to_string().as_bytes()).unwrap();
        store.write_journal(&journal(ID, UPDATED)).unwrap();
        assert!(store.recover(0, none, "1.0").is_none());
        assert!(store.read_journal().is_none());
        assert_eq!(store.names(), [name]);
    }

    #[test]
    fn save_failure_drops_or_retries_the_journal() {
        let dir = temp_dir("savefail");
        let store = PerformanceStore::new(dir.clone());
        // Journal time out of form: the minimal session cannot be named.
        let mut j = journal(ID, "x");
        store.write_journal(&j).unwrap();
        assert!(store.recover(0, none, "1.0").is_none());
        assert!(store.read_journal().is_none());
        // A real I/O error (`stress` is a file): kept twice, dropped at the third start.
        j.updated_at = UPDATED.into();
        fs::write(dir.join("stress"), b"x").unwrap();
        for n in 1..=3 {
            store.write_journal(&j).unwrap_or(());
            assert!(store.recover(0, none, "1.0").is_none());
            assert_eq!(store.read_journal().is_some(), n < 3, "start {n}");
        }
    }

    #[test]
    fn evidence_starts_at_the_session_start_and_bad_times_are_rejected() {
        let store = PerformanceStore::new(temp_dir("since"));
        store.save(&session(ID, "2026-10-06T14:09:30Z")).unwrap();
        store.write_journal(&journal(ID, UPDATED)).unwrap();
        let seen = std::cell::RefCell::new(String::new());
        let evidence = |since: &str| {
            *seen.borrow_mut() = since.to_string();
            Ok(vec![])
        };
        store.recover(0, evidence, "1.0").unwrap();
        assert_eq!(*seen.borrow(), "2026-10-06T14:09:30Z");
        assert_eq!(parse_rfc3339_ms("2026-02-30T00:00:00Z"), None);
        assert_eq!(parse_rfc3339_ms("2026-02-28T00:00:00+01:00"), None);
        assert_eq!(parse_rfc3339_ms("2026-02-28T00:00:00"), None);
        assert!(parse_rfc3339_ms("2024-02-29T00:00:00.5Z").is_some());
    }

    #[test]
    fn mismatched_content_id_is_skipped_and_bad_time_marks_the_session_crashed() {
        let store = PerformanceStore::new(temp_dir("mismatch"));
        let other = session(&uuid(5), "2026-10-06T14:03:09Z");
        let name = session_file_name("2026-10-06T14:03:09Z", ID);
        write_file(
            &store.dir().join(name),
            &serde_json::to_vec(&other).unwrap(),
        )
        .unwrap();
        assert!(store.list().is_empty());
        store
            .save(&session(&uuid(6), "2026-10-06T14:03:09Z"))
            .unwrap();
        store.write_journal(&journal(&uuid(6), "bad")).unwrap();
        let sum = store.recover(0, none, "1.0").unwrap();
        assert_eq!(sum.verdict.as_deref(), Some("crashed_app"));
    }

    #[test]
    fn saving_never_prunes_the_file_just_written() {
        let store = PerformanceStore::new(temp_dir("keepnew"));
        for i in 1..=KEEP_SESSIONS as u32 {
            let s = session(&uuid(i), "2026-06-01T00:00:00Z");
            let name = session_file_name(&s.started_at, &s.id);
            write_file(&store.dir().join(name), &serde_json::to_vec(&s).unwrap()).unwrap();
        }
        // Older than all the others.
        store.save(&session(ID, "2020-01-01T00:00:00Z")).unwrap();
        assert!(store.load(ID).unwrap().is_some());
    }

    #[test]
    fn loaded_sessions_are_capped() {
        let store = PerformanceStore::new(temp_dir("cap"));
        let mut s = session(ID, "2026-10-06T14:03:09Z");
        s.events = (0..1_005)
            .map(|i| SessionEvent {
                at_ms: i,
                code: "x".into(),
                params: BTreeMap::new(),
            })
            .collect();
        store.save(&s).unwrap();
        let l = store.load(ID).unwrap().unwrap();
        assert_eq!(l.events.len(), 1_000);
        assert_eq!(l.events_dropped, 5);
        assert_eq!(l.events[0].at_ms, 5);
    }
}
