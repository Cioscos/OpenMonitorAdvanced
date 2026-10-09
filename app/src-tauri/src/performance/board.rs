//! The leaderboard table (M8d2, DZ7-DZ10): the author rows bundled in the app plus the
//! community copy downloaded from the Worker, cached next to the Performance data.
//! One request at a time; the good copy survives every failed download.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use oma_core::scores::{
    author_rows, normalize_model, parse_table, plausible, share_block, submission_bytes,
    submit_outcome, validate_submission, Board, ErrorCode, HostFacts, ScoreFile, Source, TableRow,
    DISK_SCORE_VERSION, GPU_SCORE_VERSION, MAX_TABLE_BYTES, SCORE_VERSION,
};
use oma_core::updates::{user_agent, CheckError};
use serde::{Deserialize, Serialize};

use super::store::PerformanceStore;
use crate::overlay::store::write_file;

const TABLE_FILE: &str = "reference-scores.json";
const STATE_FILE: &str = "reference-scores.state.json";
const HOUR_MS: u64 = 3_600_000;
const FAILURE_WAIT_MS: u64 = 6 * HOUR_MS;
const CHECK_EVERY_MS: u64 = 24 * HOUR_MS;

/// What a request returned.
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub etag: Option<String>,
}

/// The HTTP side, replaced by a fake in the tests.
pub trait BoardTransport: Send + Sync {
    fn get_table(&self, user_agent: &str, etag: Option<&str>) -> Result<Reply, CheckError>;
    fn submit(&self, user_agent: &str, body: &[u8]) -> Result<Reply, CheckError>;
}

/// The real transport, over WinHTTP.
pub struct WinHttpBoard;

#[cfg(windows)]
impl BoardTransport for WinHttpBoard {
    fn get_table(&self, user_agent: &str, etag: Option<&str>) -> Result<Reply, CheckError> {
        use oma_core::scores::TABLE_URL;
        let mut headers = vec![("Accept", "application/json")];
        if let Some(etag) = etag {
            headers.push(("If-None-Match", etag));
        }
        let r = oma_win::http::get(
            TABLE_URL,
            user_agent,
            &headers,
            std::time::Duration::from_secs(10),
            MAX_TABLE_BYTES,
        )?;
        Ok(Reply {
            status: r.status,
            body: r.body,
            etag: r.etag,
        })
    }

    fn submit(&self, user_agent: &str, body: &[u8]) -> Result<Reply, CheckError> {
        use oma_core::scores::SUBMIT_URL;
        let r = oma_win::http::post(
            SUBMIT_URL,
            user_agent,
            &[("Content-Type", "application/json")],
            body,
            std::time::Duration::from_secs(10),
            4096,
        )?;
        Ok(Reply {
            status: r.status,
            body: r.body,
            etag: r.etag,
        })
    }
}

#[cfg(not(windows))]
impl BoardTransport for WinHttpBoard {
    fn get_table(&self, _: &str, _: Option<&str>) -> Result<Reply, CheckError> {
        Err(CheckError::Offline)
    }
    fn submit(&self, _: &str, _: &[u8]) -> Result<Reply, CheckError> {
        Err(CheckError::Offline)
    }
}

/// `reference-scores.state.json` (DZ7).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FetchState {
    pub format: u32,
    pub etag: Option<String>,
    pub checked_at_ms: Option<u64>,
    pub failed_at_ms: Option<u64>,
    pub error: Option<String>,
}

/// Whether a download is due (DZ8). A timestamp in the future (clock moved) counts as expired.
pub fn fetch_due(state: &FetchState, now_ms: u64, manual: bool) -> bool {
    let within = |at: Option<u64>, span: u64| at.is_some_and(|t| t <= now_ms && now_ms - t < span);
    manual
        || !(within(state.failed_at_ms, FAILURE_WAIT_MS)
            || within(state.checked_at_ms, CHECK_EVERY_MS))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardRow {
    pub board: Board,
    pub score_version: String,
    pub model: String,
    pub key: String,
    pub value: f64,
    pub n: u32,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoardVersions {
    pub cpu: &'static str,
    pub gpu: &'static str,
    pub disk: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardTable {
    pub rows: Vec<BoardRow>,
    pub community_at: Option<String>,
    pub checked_at_ms: Option<u64>,
    pub error: Option<String>,
    pub enabled: bool,
    pub versions: BoardVersions,
}

fn board_index(b: Board) -> u8 {
    match b {
        Board::CpuSingle => 0,
        Board::CpuMulti => 1,
        Board::GpuCompute => 2,
        Board::GpuGraphics => 3,
        Board::Disk => 4,
    }
}

pub struct BoardService {
    root: PathBuf,
    transport: Arc<dyn BoardTransport>,
    app_version: String,
    /// One request at a time; whoever waits finds the state already updated.
    refreshing: Mutex<()>,
}

impl BoardService {
    pub fn new(root: PathBuf, transport: Arc<dyn BoardTransport>, app_version: String) -> Self {
        Self {
            root,
            transport,
            app_version,
            refreshing: Mutex::new(()),
        }
    }

    pub fn user_agent(&self) -> String {
        user_agent(&self.app_version)
    }

    fn state(&self) -> FetchState {
        std::fs::read(self.root.join(STATE_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save_state(&self, state: &FetchState) {
        let mut state = state.clone();
        state.format = 1;
        if let Ok(bytes) = serde_json::to_vec_pretty(&state) {
            if let Err(e) = write_file(&self.root.join(STATE_FILE), &bytes) {
                tracing::warn!("leaderboard: cannot save the state: {e}");
            }
        }
    }

    /// The saved community copy: only `source: community` rows (DZ3), with its date.
    fn community(&self) -> Option<(Vec<TableRow>, Option<String>)> {
        let bytes = std::fs::read(self.root.join(TABLE_FILE)).ok()?;
        let table = parse_table(&bytes)?;
        let rows = table
            .rows
            .into_iter()
            .filter(|r| r.source == Source::Community)
            .collect();
        Some((rows, table.generated_at))
    }

    /// The author rows and the saved community rows, for the plausibility check.
    pub fn merged_rows(&self) -> Vec<TableRow> {
        let mut rows = author_rows().to_vec();
        rows.extend(self.community().map(|c| c.0).unwrap_or_default());
        rows
    }

    /// The checked submission bytes: exactly what the preview shows and the send posts (DZ4).
    fn prepare(
        &self,
        file: &ScoreFile,
        facts: &HostFacts,
        overclock: bool,
    ) -> Result<Vec<u8>, String> {
        if let Some(code) = share_block(file) {
            return Err(code.to_owned());
        }
        submission_bytes(file, facts, overclock).map_err(|c| c.as_str().to_owned())
    }

    /// The text the user previews before sending.
    pub fn preview(
        &self,
        file: &ScoreFile,
        facts: &HostFacts,
        overclock: bool,
    ) -> Result<String, String> {
        let bytes = self.prepare(file, facts, overclock)?;
        String::from_utf8(bytes).map_err(|_| "invalid".to_owned())
    }

    /// Posts the submission (DZ11): the same checks as the server, locally first.
    pub fn share(
        &self,
        file: &ScoreFile,
        facts: &HostFacts,
        overclock: bool,
    ) -> Result<(), String> {
        let bytes = self.prepare(file, facts, overclock)?;
        let value = serde_json::from_slice(&bytes).map_err(|_| "invalid".to_owned())?;
        let submission = validate_submission(&value).map_err(|c| c.as_str().to_owned())?;
        let rows = self.merged_rows();
        let key = &submission.model.key;
        if !submission
            .values
            .iter()
            .all(|&(b, v)| plausible(b, &submission.score_version, key, v, &rows))
        {
            return Err(ErrorCode::Implausible.as_str().to_owned());
        }
        let reply = self
            .transport
            .submit(&self.user_agent(), &bytes)
            .map_err(|e| e.category().to_owned())?;
        submit_outcome(reply.status, &reply.body)
    }

    /// Sends the stored score and, only if that succeeded, marks it shared. A failed
    /// mark is logged: the data is already out.
    pub fn share_stored(
        &self,
        store: &PerformanceStore,
        id: &str,
        facts: &HostFacts,
        overclock: bool,
    ) -> Result<(), String> {
        let file = store
            .load_score(id)
            .map_err(|_| "invalid".to_owned())?
            .ok_or_else(|| "not_found".to_owned())?;
        self.share(&file, facts, overclock)?;
        if let Err(e) = store.mark_shared(id) {
            tracing::warn!("leaderboard: cannot mark the score shared: {e}");
        }
        Ok(())
    }

    /// The table from what is on disk; never touches the network.
    pub fn table(&self, enabled: bool) -> BoardTable {
        let state = self.state();
        let (community, community_at) = match self.community() {
            Some((rows, at)) => (rows, at),
            None => (Vec::new(), None),
        };
        let mut rows: Vec<BoardRow> = author_rows()
            .iter()
            .cloned()
            .chain(community)
            .map(|r| BoardRow {
                key: normalize_model(&r.model).key,
                board: r.category,
                score_version: r.score_version,
                model: r.model,
                value: r.value,
                n: r.n,
                source: r.source,
            })
            .collect();
        rows.sort_by(|a, b| {
            board_index(a.board)
                .cmp(&board_index(b.board))
                .then(b.value.total_cmp(&a.value))
        });
        BoardTable {
            rows,
            community_at,
            checked_at_ms: state.checked_at_ms,
            error: state.error,
            enabled,
            versions: BoardVersions {
                cpu: SCORE_VERSION,
                gpu: GPU_SCORE_VERSION,
                disk: DISK_SCORE_VERSION,
            },
        }
    }

    /// Downloads when due (DZ8); with the setting off nothing is requested (DZ9).
    pub fn refresh(&self, enabled: bool, manual: bool, now_ms: u64) -> BoardTable {
        if enabled {
            let _one = self
                .refreshing
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let mut state = self.state();
            if fetch_due(&state, now_ms, manual) {
                self.download(&mut state, now_ms);
            }
        }
        self.table(enabled)
    }

    fn download(&self, state: &mut FetchState, now_ms: u64) {
        // Without a readable copy an old ETag would earn a 304 forever.
        let have_copy = self.community().is_some();
        let etag = state.etag.clone().filter(|_| have_copy);
        let result = match self
            .transport
            .get_table(&self.user_agent(), etag.as_deref())
        {
            Ok(r) if r.status == 304 && have_copy => Ok(state.etag.clone()),
            Ok(r) if r.status == 200 => self.accept(&r),
            Ok(_) => Err("http"),
            Err(e) => Err(e.category()),
        };
        match result {
            Ok(new_etag) => {
                state.etag = new_etag;
                state.checked_at_ms = Some(now_ms);
                state.failed_at_ms = None;
                state.error = None;
            }
            Err(code) => {
                tracing::warn!("leaderboard: download failed ({code})");
                state.failed_at_ms = Some(now_ms);
                state.error = Some(code.to_owned());
            }
        }
        self.save_state(state);
    }

    /// Saves a valid `200` body and returns its ETag.
    fn accept(&self, r: &Reply) -> Result<Option<String>, &'static str> {
        if r.body.len() > MAX_TABLE_BYTES || parse_table(&r.body).is_none() {
            return Err("invalid");
        }
        write_file(&self.root.join(TABLE_FILE), &r.body).map_err(|_| "invalid")?;
        Ok(r.etag.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const NOW: u64 = 1_800_000_000_000;

    fn table_json(rows: &str) -> Vec<u8> {
        format!(r#"{{"format":1,"generatedAt":"2026-10-09T10:00:00Z","rows":[{rows}]}}"#)
            .into_bytes()
    }

    fn row(model: &str, value: f64, source: &str) -> String {
        format!(
            r#"{{"category":"cpu-single","scoreVersion":"cpu-1","model":"{model}","value":{value},"n":5,"source":"{source}"}}"#
        )
    }

    fn good() -> Vec<u8> {
        table_json(&row("Test CPU", 1234.0, "community"))
    }

    type Canned = Result<(u16, Vec<u8>, Option<String>), CheckError>;

    #[derive(Default)]
    struct Fake {
        calls: AtomicUsize,
        etags: Mutex<Vec<Option<String>>>,
        replies: Mutex<Vec<Canned>>,
        delay: bool,
        submits: Mutex<Vec<Vec<u8>>>,
        submit_reply: Mutex<Option<Canned>>,
    }

    impl Fake {
        fn with(replies: Vec<Canned>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies),
                ..Self::default()
            })
        }
    }

    impl BoardTransport for Fake {
        fn get_table(&self, _: &str, etag: Option<&str>) -> Result<Reply, CheckError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.etags.lock().unwrap().push(etag.map(str::to_owned));
            if self.delay {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let mut replies = self.replies.lock().unwrap();
            let next = if replies.len() > 1 {
                replies.remove(0)
            } else {
                replies[0].clone()
            };
            next.map(|(status, body, etag)| Reply { status, body, etag })
        }
        fn submit(&self, _: &str, body: &[u8]) -> Result<Reply, CheckError> {
            self.submits.lock().unwrap().push(body.to_vec());
            match self.submit_reply.lock().unwrap().clone() {
                Some(r) => r.map(|(status, body, etag)| Reply { status, body, etag }),
                None => Err(CheckError::Offline),
            }
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "oma-board-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn service(name: &str, fake: &Arc<Fake>) -> BoardService {
        BoardService::new(temp_dir(name), fake.clone(), "0.0.0".into())
    }

    fn community_count(t: &BoardTable) -> usize {
        t.rows
            .iter()
            .filter(|r| r.source == Source::Community)
            .count()
    }

    #[test]
    fn first_refresh_downloads_and_saves() {
        let fake = Fake::with(vec![Ok((200, good(), Some("\"e1\"".into())))]);
        let s = service("first", &fake);
        let t = s.refresh(true, false, NOW);
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
        assert_eq!(community_count(&t), 1);
        assert_eq!(t.checked_at_ms, Some(NOW));
        assert_eq!(t.community_at.as_deref(), Some("2026-10-09T10:00:00Z"));
        assert_eq!(s.state().etag.as_deref(), Some("\"e1\""));
        assert_eq!(std::fs::read(s.root.join(TABLE_FILE)).unwrap(), good());
    }

    #[test]
    fn refresh_within_24h_does_not_request() {
        let fake = Fake::with(vec![Ok((200, good(), None))]);
        let s = service("within", &fake);
        s.refresh(true, false, NOW);
        s.refresh(true, false, NOW + 23 * HOUR_MS);
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
        s.refresh(true, false, NOW + 24 * HOUR_MS);
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn failure_waits_6h_and_keeps_the_good_copy() {
        let fake = Fake::with(vec![
            Ok((200, good(), None)),
            Err(CheckError::Offline),
            Ok((200, good(), None)),
        ]);
        let s = service("fail", &fake);
        s.refresh(true, false, NOW);
        let t = s.refresh(true, false, NOW + 25 * HOUR_MS);
        assert_eq!(t.error.as_deref(), Some("offline"));
        assert_eq!(community_count(&t), 1);
        s.refresh(true, false, NOW + 30 * HOUR_MS);
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
        let t = s.refresh(true, false, NOW + 31 * HOUR_MS);
        assert_eq!(fake.calls.load(Ordering::SeqCst), 3);
        assert_eq!(t.error, None);
    }

    #[test]
    fn manual_refresh_always_requests() {
        let fake = Fake::with(vec![Ok((200, good(), None))]);
        let s = service("manual", &fake);
        s.refresh(true, true, NOW);
        s.refresh(true, true, NOW + 1);
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn disabled_never_requests_but_uses_the_saved_copy() {
        let fake = Fake::with(vec![Ok((200, good(), None))]);
        let s = service("disabled", &fake);
        s.refresh(true, false, NOW);
        let t = s.refresh(false, true, NOW + 100 * HOUR_MS);
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
        assert!(!t.enabled);
        assert_eq!(community_count(&t), 1);
    }

    #[test]
    fn not_modified_updates_checked_at() {
        let fake = Fake::with(vec![
            Ok((200, good(), Some("\"e1\"".into()))),
            Ok((304, Vec::new(), None)),
        ]);
        let s = service("304", &fake);
        s.refresh(true, false, NOW);
        let later = NOW + 25 * HOUR_MS;
        let t = s.refresh(true, false, later);
        assert_eq!(t.checked_at_ms, Some(later));
        assert_eq!(t.error, None);
        assert_eq!(s.state().etag.as_deref(), Some("\"e1\""));
        assert_eq!(
            fake.etags.lock().unwrap().as_slice(),
            [None, Some("\"e1\"".to_owned())]
        );
    }

    #[test]
    fn etag_is_sent_only_with_a_readable_cache() {
        let fake = Fake::with(vec![
            Ok((200, good(), Some("\"e1\"".into()))),
            Ok((200, good(), Some("\"e1\"".into()))),
            Ok((200, good(), Some("\"e1\"".into()))),
        ]);
        let s = service("etag", &fake);
        s.refresh(true, false, NOW);
        // The copy disappears: the ETag stays in the state but must not be sent.
        std::fs::remove_file(s.root.join(TABLE_FILE)).unwrap();
        s.refresh(true, true, NOW + 1);
        // The copy is garbage.
        std::fs::write(s.root.join(TABLE_FILE), b"not json").unwrap();
        s.refresh(true, true, NOW + 2);
        assert_eq!(fake.etags.lock().unwrap().as_slice(), [None, None, None]);
        // Restored: the next request sends it.
        s.refresh(true, true, NOW + 3);
        assert_eq!(
            fake.etags.lock().unwrap().last().unwrap().as_deref(),
            Some("\"e1\"")
        );
    }

    #[test]
    fn future_timestamps_make_the_fetch_due() {
        let state = FetchState {
            checked_at_ms: Some(NOW + 1000 * HOUR_MS),
            failed_at_ms: Some(NOW + 1000 * HOUR_MS),
            ..FetchState::default()
        };
        assert!(fetch_due(&state, NOW, false));
        let fresh = FetchState {
            checked_at_ms: Some(NOW - HOUR_MS),
            ..FetchState::default()
        };
        assert!(!fetch_due(&fresh, NOW, false));
    }

    #[test]
    fn bad_download_keeps_the_good_copy() {
        let bad: Vec<Vec<u8>> = vec![
            vec![b' '; 2 * 1024 * 1024],
            br#"{"format":2,"rows":[]}"#.to_vec(),
            b"<html>not json</html>".to_vec(),
        ];
        for (i, body) in bad.into_iter().enumerate() {
            let fake = Fake::with(vec![
                Ok((200, good(), Some("\"e1\"".into()))),
                Ok((200, body, Some("\"e2\"".into()))),
            ]);
            let s = service(&format!("bad{i}"), &fake);
            s.refresh(true, false, NOW);
            let t = s.refresh(true, true, NOW + 1);
            assert_eq!(t.error.as_deref(), Some("invalid"));
            assert_eq!(community_count(&t), 1);
            assert_eq!(std::fs::read(s.root.join(TABLE_FILE)).unwrap(), good());
            assert_eq!(s.state().etag.as_deref(), Some("\"e1\""));
        }
    }

    #[test]
    fn downloaded_author_rows_are_dropped() {
        let body = table_json(&format!(
            "{},{}",
            row("Fake Author CPU", 99999.0, "author"),
            row("Real Community CPU", 10.0, "community")
        ));
        let fake = Fake::with(vec![Ok((200, body, None))]);
        let s = service("author", &fake);
        let t = s.refresh(true, false, NOW);
        assert!(t.rows.iter().all(|r| r.model != "Fake Author CPU"));
        assert!(t.rows.iter().any(|r| r.model == "Real Community CPU"));
        assert!(s.merged_rows().iter().all(|r| r.model != "Fake Author CPU"));
    }

    #[test]
    fn rows_carry_the_normalized_key_and_are_sorted() {
        let body = table_json(&format!(
            "{},{}",
            row("Intel(R) Core(TM)  Zeta", 10.0, "community"),
            row("Big CPU", 90000.0, "community")
        ));
        let fake = Fake::with(vec![Ok((200, body, None))]);
        let s = service("sorted", &fake);
        let t = s.refresh(true, false, NOW);
        for r in &t.rows {
            assert_eq!(r.key, normalize_model(&r.model).key);
        }
        let zeta = t.rows.iter().find(|r| r.model.contains("Zeta")).unwrap();
        assert_eq!(zeta.key, normalize_model("Intel(R) Core(TM)  Zeta").key);
        assert!(!zeta.key.contains("(r)"));
        for pair in t.rows.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let (ia, ib) = (board_index(a.board), board_index(b.board));
            assert!(ia < ib || (ia == ib && a.value >= b.value));
        }
    }

    #[test]
    fn concurrent_refreshes_make_one_request() {
        let fake = Arc::new(Fake {
            replies: Mutex::new(vec![Ok((200, good(), None))]),
            delay: true,
            ..Fake::default()
        });
        let s = Arc::new(service("concurrent", &fake));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let s = s.clone();
                std::thread::spawn(move || s.refresh(true, false, NOW))
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    }

    fn facts() -> HostFacts {
        HostFacts {
            ram_gb: 32,
            os_build: "26300".into(),
        }
    }

    const ID: &str = "0b9c5a2e-1d3f-4a6b-8c7d-9e0f1a2b3c4d";

    fn cpu_score(single: u32) -> ScoreFile {
        let json = format!(
            r#"{{"format":1,"id":"{ID}","at":"2026-10-07T10:00:00Z","category":"cpu","scoreVersion":"cpu-1","provisional":false,"isa":null,"shaderDigest":null,"scores":{{"single":{single},"multi":1495,"compute":null,"graphics":null}},"kernels":[],"device":{{"model":"AMD Ryzen 7 7800X3D","cores":8,"logical":16}},"flags":[],"valid":true,"scaling":null,"samples":[],"appVersion":"0.5.0"}}"#
        );
        oma_core::scores::parse_score(json.as_bytes()).expect("a valid score file")
    }

    fn sharer(name: &str, reply: Option<Canned>) -> (BoardService, Arc<Fake>, PerformanceStore) {
        let fake = Fake::with(vec![Ok((200, good(), None))]);
        *fake.submit_reply.lock().unwrap() = reply;
        let dir = temp_dir(name);
        let s = BoardService::new(dir.clone(), fake.clone(), "0.0.0".into());
        (s, fake, PerformanceStore::new(dir))
    }

    fn created() -> Option<Canned> {
        Some(Ok((201, b"{}".to_vec(), None)))
    }

    #[test]
    fn share_posts_the_previewed_bytes() {
        let (s, fake, _) = sharer("post", created());
        let f = cpu_score(1491);
        let preview = s.preview(&f, &facts(), true).unwrap();
        s.share(&f, &facts(), true).unwrap();
        let sent = fake.submits.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], preview.as_bytes());
        assert_eq!(sent[0], submission_bytes(&f, &facts(), true).unwrap());
    }

    #[test]
    fn implausible_share_is_refused_without_a_request() {
        let (s, fake, _) = sharer("implausible", created());
        assert_eq!(
            s.share(&cpu_score(50000), &facts(), false),
            Err("implausible".to_owned())
        );
        assert!(fake.submits.lock().unwrap().is_empty());
    }

    #[test]
    fn failed_send_is_not_marked_shared() {
        let (s, fake, store) = sharer("failed", None);
        store.save_score(&cpu_score(1491)).unwrap();
        assert_eq!(
            s.share_stored(&store, ID, &facts(), false),
            Err("offline".to_owned())
        );
        assert!(!store.load_score(ID).unwrap().unwrap().shared);
        *fake.submit_reply.lock().unwrap() =
            Some(Ok((429, br#"{"error":"rate_limited"}"#.to_vec(), None)));
        assert_eq!(
            s.share_stored(&store, ID, &facts(), false),
            Err("rate_limited".to_owned())
        );
        assert!(!store.load_score(ID).unwrap().unwrap().shared);
        assert_eq!(fake.submits.lock().unwrap().len(), 2);
    }

    #[test]
    fn successful_send_marks_the_score_shared() {
        let (s, fake, store) = sharer("ok", created());
        store.save_score(&cpu_score(1491)).unwrap();
        s.share_stored(&store, ID, &facts(), false).unwrap();
        assert!(store.load_score(ID).unwrap().unwrap().shared);
        // Already shared: refused, nothing more is sent.
        assert_eq!(
            s.share_stored(&store, ID, &facts(), false),
            Err("shared".to_owned())
        );
        assert_eq!(fake.submits.lock().unwrap().len(), 1);
    }

    #[test]
    fn provisional_and_shared_scores_are_refused() {
        let (s, fake, _) = sharer("refused", created());
        let mut f = cpu_score(1491);
        f.provisional = true;
        assert_eq!(s.share(&f, &facts(), false), Err("provisional".to_owned()));
        f.provisional = false;
        f.shared = true;
        assert_eq!(s.share(&f, &facts(), false), Err("shared".to_owned()));
        assert!(s.preview(&f, &facts(), false).is_err());
        assert!(fake.submits.lock().unwrap().is_empty());
    }
}
