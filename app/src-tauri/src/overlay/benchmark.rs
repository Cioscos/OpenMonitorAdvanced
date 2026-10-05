//! Benchmark recording (M7d, §8): the pure [`Recorder`] that turns the
//! followed game's frames into CSV rows and a [`SessionSummary`], and the
//! files it writes (per-frame CSV plus a JSON summary) in the `benchmarks`
//! folder of the CSV log, with the history list and delete.
//!
//! The recorder keeps only the displayed frametimes (`f32`) and counters, up
//! to `MAX_SESSION_FRAMES`; rows go to disk through a 64 KiB buffer.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oma_core::csv::{self, LocalTime, BOM};
use oma_core::frames::{FrameKind, FrameSample, SessionAccumulator, SessionSummary};
use serde::{Deserialize, Serialize};

use crate::log::fs::{LogFile, LogFs};
use crate::log::writer::WriteFailure;

/// A capture stops after one hour (§8).
pub const MAX_DURATION_MS: u64 = 3_600_000;
/// A capture stops once the game has been gone this long.
pub const NO_TARGET_STOP_MS: u64 = 10_000;
/// How long the overlay shows the final summary.
pub const SUMMARY_SHOW_MS: u64 = 10_000;
/// History entries listed.
pub const MAX_HISTORY: usize = 500;
/// Largest summary file read back.
pub const MAX_SUMMARY_BYTES: u64 = 65_536;
/// The CSV buffer is written once it holds this many bytes.
pub const FLUSH_BYTES: usize = 64 * 1024;

const CSV_HEADER: &str = "qpc_ms,frametime_displayed_ms,frametime_app_ms,frame_type,displayed,pc_latency_ms,gpu_busy_ms\r\n";
/// Longest id the UI may name.
const MAX_ID_LEN: usize = 96;
/// Collision suffixes tried: `-2` to `-99`.
const MAX_SUFFIX: u32 = 99;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EndReason {
    User,
    NoTarget,
    Limit,
    Error,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    NoTarget,
    AlreadyRecording,
}

struct Active {
    game: String,
    acc: SessionAccumulator,
    start_ms: u64,
    first_t: Option<f64>,
    capped: bool,
}

/// The capture state machine; no clock, no files.
pub struct Recorder {
    cap: usize,
    target_present: bool,
    absent_since: Option<u64>,
    active: Option<Active>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder {
    pub fn new() -> Self {
        Self::with_cap(oma_core::frames::MAX_SESSION_FRAMES)
    }

    pub fn with_cap(cap: usize) -> Self {
        Self {
            cap,
            target_present: false,
            absent_since: None,
            active: None,
        }
    }

    pub fn is_recording(&self) -> bool {
        self.active.is_some()
    }

    /// The game being recorded.
    pub fn game(&self) -> Option<&str> {
        self.active.as_ref().map(|a| a.game.as_str())
    }

    pub fn start(&mut self, exe: &str, now_ms: u64) -> Result<(), StartError> {
        if self.active.is_some() {
            return Err(StartError::AlreadyRecording);
        }
        if !self.target_present {
            return Err(StartError::NoTarget);
        }
        self.active = Some(Active {
            game: exe.to_owned(),
            acc: SessionAccumulator::with_cap(self.cap),
            start_ms: now_ms,
            first_t: None,
            capped: false,
        });
        Ok(())
    }

    /// The CSV rows (CRLF included) of the accepted frames.
    pub fn on_frames(&mut self, frames: &[FrameSample], _now_ms: u64) -> Vec<String> {
        let Some(a) = self.active.as_mut().filter(|a| !a.capped) else {
            return Vec::new();
        };
        let mut rows = Vec::with_capacity(frames.len());
        for f in frames {
            if !a.acc.push(f) {
                a.capped = true;
                break;
            }
            let first = *a.first_t.get_or_insert(f.t_s);
            rows.push(csv_row(f, (f.t_s - first) * 1000.0));
        }
        rows
    }

    pub fn on_target(&mut self, present: bool, now_ms: u64) {
        self.target_present = present;
        if present {
            self.absent_since = None;
        } else {
            self.absent_since.get_or_insert(now_ms);
        }
    }

    /// Why the capture must end now, if it must.
    pub fn tick(&mut self, now_ms: u64) -> Option<EndReason> {
        let a = self.active.as_ref()?;
        if a.capped || now_ms.saturating_sub(a.start_ms) >= MAX_DURATION_MS {
            return Some(EndReason::Limit);
        }
        match self.absent_since {
            Some(t) if now_ms.saturating_sub(t) >= NO_TARGET_STOP_MS => Some(EndReason::NoTarget),
            _ => None,
        }
    }

    /// Ends the capture; the summary is `None` without enough frames.
    pub fn stop(&mut self, _reason: EndReason) -> Option<SessionSummary> {
        self.active.take().and_then(|a| a.acc.summary())
    }

    pub fn elapsed_s(&self, now_ms: u64) -> Option<u32> {
        let a = self.active.as_ref()?;
        Some(
            (now_ms.saturating_sub(a.start_ms) / 1000)
                .try_into()
                .unwrap_or(u32::MAX),
        )
    }
}

fn kind_wire(kind: FrameKind) -> &'static str {
    match kind {
        FrameKind::App => "app",
        FrameKind::GeneratedIntelXefg => "generated_intel_xefg",
        FrameKind::GeneratedAmdAfmf => "generated_amd_afmf",
        FrameKind::GeneratedOther => "generated_other",
        FrameKind::Unknown => "unknown",
    }
}

fn csv_row(f: &FrameSample, qpc_ms: f64) -> String {
    fn num(v: Option<f64>, out: &mut String) {
        if let Some(v) = v {
            csv::format_number(v, out);
        }
    }
    let mut out = String::with_capacity(64);
    num(Some(qpc_ms), &mut out);
    out.push(',');
    num(f.ms_between_display_change, &mut out);
    out.push(',');
    num(f.ms_app_frametime, &mut out);
    out.push(',');
    csv::escape_field(kind_wire(f.kind), &mut out);
    out.push_str(if f.displayed { ",1," } else { ",0," });
    num(f.ms_pc_latency, &mut out);
    out.push(',');
    num(f.ms_gpu_busy, &mut out);
    out.push_str("\r\n");
    out
}

/// `<exe>-<AAAAMMGG-hhmmss>`, safe to use as a file name.
pub fn file_stem(exe: &str, start: LocalTime) -> String {
    let base = match exe.len().checked_sub(4) {
        Some(n) if exe.is_char_boundary(n) && exe[n..].eq_ignore_ascii_case(".exe") => &exe[..n],
        _ => exe,
    };
    let mut name: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    // `..` is not an id: break every run of dots.
    while name.contains("..") {
        name = name.replace("..", "._");
    }
    if name.is_empty() {
        name.push_str("game");
    }
    format!(
        "{name}-{:04}{:02}{:02}-{:02}{:02}{:02}",
        start.year, start.month, start.day, start.hour, start.minute, start.second
    )
}

/// The `AAAAMMGG-hhmmss` tail of a stem that ends with `-AAAAMMGG-hhmmss`
/// after a non-empty name.
fn ends_with_stamp(s: &str) -> Option<&str> {
    let n = s.len().checked_sub(16).filter(|&n| n > 0)?;
    let tail = s.get(n..)?.as_bytes();
    let ok = tail.iter().enumerate().all(|(i, b)| {
        if i == 0 || i == 9 {
            *b == b'-'
        } else {
            b.is_ascii_digit()
        }
    });
    ok.then(|| &s[n + 1..])
}

/// The time stamp of a valid id.
fn stamp_of(id: &str) -> Option<&str> {
    if let Some((head, n)) = id.rsplit_once('-') {
        if (1..=2).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit()) {
            if let Some(stamp) = ends_with_stamp(head) {
                return Some(stamp);
            }
        }
    }
    ends_with_stamp(id)
}

/// Sort key: time stamp, then the collision suffix as a number.
fn rank(id: &str) -> (Option<&str>, u32) {
    let n = id
        .rsplit_once('-')
        .filter(|(head, _)| ends_with_stamp(head).is_some())
        .and_then(|(_, n)| n.parse().ok())
        .unwrap_or(1);
    (stamp_of(id), n)
}

/// Trust boundary for ids coming from the UI: only what [`file_stem`] and the
/// collision suffix can produce, so no path leaves the benchmarks folder.
pub fn is_benchmark_id(id: &str) -> bool {
    id.len() <= MAX_ID_LEN
        && !id.contains("..")
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && stamp_of(id).is_some()
}

/// The `.json` next to each CSV.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkRecord {
    pub format: u32,
    pub game: String,
    pub started_at: String,
    pub end_reason: EndReason,
    pub summary: SessionSummary,
}

impl BenchmarkRecord {
    pub fn new(
        game: &str,
        start: LocalTime,
        end_reason: EndReason,
        summary: SessionSummary,
    ) -> Self {
        Self {
            format: 1,
            game: game.to_owned(),
            started_at: format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                start.year, start.month, start.day, start.hour, start.minute, start.second
            ),
            end_reason,
            summary,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BenchmarkEntry {
    pub id: String,
    pub record: BenchmarkRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchmarkError {
    InvalidId,
    NotFound,
    Io(String),
}

pub struct BenchmarkFiles {
    dir: PathBuf,
    fs: Arc<dyn LogFs>,
}

impl BenchmarkFiles {
    pub fn new(dir: PathBuf, fs: Arc<dyn LogFs>) -> Self {
        Self { dir, fs }
    }

    /// Creates the folder and the CSV (`-2`…`-99` on a name collision) with
    /// BOM and header.
    pub fn begin(&self, stem: &str) -> Result<BenchmarkWriter, WriteFailure> {
        if !is_benchmark_id(stem) {
            return Err(WriteFailure::Other(format!(
                "invalid benchmark name {stem}"
            )));
        }
        self.fs
            .create_dir_all(&self.dir)
            .map_err(|e| WriteFailure::from_io(&e))?;
        for n in 1..=MAX_SUFFIX {
            let name = if n == 1 {
                stem.to_owned()
            } else {
                format!("{stem}-{n}")
            };
            let csv = self.dir.join(format!("{name}.csv"));
            let mut file = match self.fs.create_new(&csv) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(WriteFailure::from_io(&e)),
            };
            let mut head = BOM.to_vec();
            head.extend_from_slice(CSV_HEADER.as_bytes());
            if let Err(e) = file.write_all(&head) {
                drop(file);
                let _ = self.fs.remove_file(&csv);
                return Err(WriteFailure::from_io(&e));
            }
            return Ok(BenchmarkWriter {
                file,
                json: self.dir.join(format!("{name}.json")),
                csv,
                fs: self.fs.clone(),
                buf: Vec::with_capacity(FLUSH_BYTES + 1024),
                rows: 0,
                failed: false,
            });
        }
        Err(WriteFailure::TooManyCollisions)
    }

    /// Summaries, newest first (by their time stamp), at most [`MAX_HISTORY`].
    /// Unreadable, oversized and malformed files are skipped.
    pub fn list(&self) -> Vec<BenchmarkEntry> {
        let Ok(dir) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = dir
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let id = name.strip_suffix(".json")?;
                is_benchmark_id(id).then(|| id.to_owned())
            })
            .collect();
        ids.sort_by(|a, b| rank(b).cmp(&rank(a)));
        ids.truncate(MAX_HISTORY);
        ids.into_iter()
            .filter_map(|id| {
                let record = read_record(&self.dir.join(format!("{id}.json")))?;
                Some(BenchmarkEntry { id, record })
            })
            .collect()
    }

    /// The CSV of `id`, for export.
    pub fn csv_path(&self, id: &str) -> Result<PathBuf, BenchmarkError> {
        if !is_benchmark_id(id) {
            return Err(BenchmarkError::InvalidId);
        }
        let path = self.dir.join(format!("{id}.csv"));
        if path.is_file() {
            Ok(path)
        } else {
            Err(BenchmarkError::NotFound)
        }
    }

    /// Removes the CSV and the summary of `id`.
    pub fn delete(&self, id: &str) -> Result<(), BenchmarkError> {
        if !is_benchmark_id(id) {
            return Err(BenchmarkError::InvalidId);
        }
        let mut found = false;
        for ext in ["csv", "json"] {
            match std::fs::remove_file(self.dir.join(format!("{id}.{ext}"))) {
                Ok(()) => found = true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(BenchmarkError::Io(e.to_string())),
            }
        }
        if found {
            Ok(())
        } else {
            Err(BenchmarkError::NotFound)
        }
    }
}

fn read_record(path: &Path) -> Option<BenchmarkRecord> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_SUMMARY_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_SUMMARY_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

/// An open CSV. After a write error the rest of the rows is dropped.
pub struct BenchmarkWriter {
    file: Box<dyn LogFile>,
    fs: Arc<dyn LogFs>,
    csv: PathBuf,
    json: PathBuf,
    buf: Vec<u8>,
    rows: u64,
    failed: bool,
}

impl BenchmarkWriter {
    /// Buffers the rows. The first write error is returned once and must end
    /// the capture (the caller stops with `EndReason::Error`); later rows are
    /// dropped and return `Ok`.
    pub fn append(&mut self, lines: &[String]) -> Result<(), WriteFailure> {
        if self.failed {
            return Ok(());
        }
        for line in lines {
            self.buf.extend_from_slice(line.as_bytes());
        }
        self.rows += lines.len() as u64;
        if self.buf.len() >= FLUSH_BYTES {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), WriteFailure> {
        let result = self
            .file
            .write_all(&self.buf)
            .and_then(|()| self.file.flush());
        self.buf.clear();
        result.map_err(|e| {
            self.failed = true;
            WriteFailure::from_io(&e)
        })
    }

    /// Flushes and, with a summary, writes the `.json` atomically. Without a
    /// summary the CSV is removed (it would never show in the history).
    /// Returns the first error.
    pub fn finish(mut self, summary: Option<&BenchmarkRecord>) -> Result<(), WriteFailure> {
        let flushed = if self.failed { Ok(()) } else { self.flush() };
        let mut written = Ok(());
        if let Some(record) = summary {
            written = serde_json::to_vec_pretty(record)
                .map_err(|e| WriteFailure::Other(e.to_string()))
                .and_then(|bytes| {
                    super::store::write_file(&self.json, &bytes)
                        .map_err(|e| WriteFailure::from_io(&e))
                });
        }
        let Self { fs, csv, file, .. } = self;
        drop(file);
        if summary.is_none() {
            let _ = fs.remove_file(&csv);
        }
        flushed.and(written)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::fs::fake::{Fault, MemFs};
    use crate::log::fs::RealFs;
    use oma_core::csv::local_time;
    use oma_core::frames::FrameKind;

    fn frame(t_s: f64) -> FrameSample {
        FrameSample {
            t_s,
            swapchain: 1,
            kind: FrameKind::App,
            displayed: true,
            ms_between_presents: 16.0,
            ms_between_display_change: Some(16.0),
            ms_until_displayed: None,
            ms_app_frametime: None,
            ms_pc_latency: None,
            ms_gpu_busy: None,
            pcl_frame_id: None,
        }
    }

    fn started(cap: usize) -> Recorder {
        let mut r = Recorder::with_cap(cap);
        r.on_target(true, 0);
        r.start("game.exe", 0).unwrap();
        r
    }

    fn stamp() -> oma_core::csv::LocalTime {
        // 2026-10-05 21:30:00 UTC
        local_time(1_791_235_800_000, 0)
    }

    fn temp_dir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "oma-bench-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn record(reason: EndReason) -> BenchmarkRecord {
        let mut acc = SessionAccumulator::new();
        for i in 0..20 {
            acc.push(&frame(f64::from(i) * 0.016));
        }
        BenchmarkRecord::new("game.exe", stamp(), reason, acc.summary().unwrap())
    }

    #[test]
    fn start_needs_a_target() {
        let mut r = Recorder::new();
        assert_eq!(r.start("g.exe", 0), Err(StartError::NoTarget));
        r.on_target(true, 0);
        assert_eq!(r.start("g.exe", 0), Ok(()));
        assert_eq!(r.start("g.exe", 1), Err(StartError::AlreadyRecording));
    }

    #[test]
    fn stops_after_ten_seconds_without_target() {
        let mut r = started(100);
        r.on_target(false, 1_000);
        assert_eq!(r.tick(10_999), None);
        assert_eq!(r.tick(11_000), Some(EndReason::NoTarget));
    }

    #[test]
    fn a_return_after_nine_seconds_does_not_stop() {
        let mut r = started(100);
        r.on_target(false, 1_000);
        r.on_target(true, 10_000);
        r.on_target(false, 10_500);
        assert_eq!(r.tick(20_000), None);
        assert_eq!(r.tick(20_500), Some(EndReason::NoTarget));
    }

    #[test]
    fn stops_at_sixty_minutes() {
        let mut r = started(100);
        assert_eq!(r.tick(MAX_DURATION_MS - 1), None);
        assert_eq!(r.tick(MAX_DURATION_MS), Some(EndReason::Limit));
        assert_eq!(r.elapsed_s(61_000), Some(61));
    }

    #[test]
    fn stops_at_the_frame_cap() {
        let mut r = started(3);
        let frames: Vec<_> = (0..4).map(|i| frame(f64::from(i) * 0.016)).collect();
        assert_eq!(r.on_frames(&frames, 100).len(), 3);
        assert_eq!(r.tick(100), Some(EndReason::Limit));
        assert_eq!(r.stop(EndReason::Limit).unwrap().frames_displayed, 3);
        assert_eq!(r.elapsed_s(200), None);
        assert!(r.on_frames(&frames, 300).is_empty());
    }

    #[test]
    fn csv_rows_match_the_spec_columns() {
        assert_eq!(
            CSV_HEADER,
            "qpc_ms,frametime_displayed_ms,frametime_app_ms,frame_type,displayed,pc_latency_ms,gpu_busy_ms\r\n"
        );
        let mut r = started(100);
        let mut second = frame(10.0165);
        second.ms_app_frametime = Some(8.25);
        second.kind = FrameKind::GeneratedOther;
        second.displayed = false;
        second.ms_between_display_change = None;
        second.ms_gpu_busy = Some(7.5);
        let rows = r.on_frames(&[frame(10.0), second], 0);
        assert_eq!(rows[0], "0,16,,app,1,,\r\n");
        assert_eq!(rows[1], "16.5,,8.25,generated_other,0,,7.5\r\n");
    }

    #[test]
    fn formula_guard_on_frame_type() {
        let mut out = String::new();
        csv::escape_field("=cmd", &mut out);
        assert_eq!(out, "'=cmd");
        // Every wire string the recorder writes is a plain word.
        for kind in [
            FrameKind::App,
            FrameKind::GeneratedIntelXefg,
            FrameKind::GeneratedAmdAfmf,
            FrameKind::GeneratedOther,
            FrameKind::Unknown,
        ] {
            assert!(kind_wire(kind)
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }

    #[test]
    fn file_stem_sanitizes_the_exe() {
        assert_eq!(
            file_stem("Control_DX12.exe", stamp()),
            "Control_DX12-20261005-213000"
        );
        let s = file_stem("../a:b.exe", stamp());
        assert!(!s.contains(['/', '\\', ':']));
        assert!(is_benchmark_id(&s), "{s}");
        assert!(file_stem(".exe", stamp()).starts_with("game-"));
        assert!(file_stem(&"x".repeat(200), stamp()).len() <= 64 + 16);
    }

    #[test]
    fn name_collision_adds_a_suffix() {
        let fs = MemFs::new();
        let files = BenchmarkFiles::new("b".into(), fs.clone());
        for _ in 0..3 {
            files.begin("g-20261005-213000").unwrap();
        }
        let names: Vec<_> = fs
            .files()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "g-20261005-213000-2.csv",
                "g-20261005-213000-3.csv",
                "g-20261005-213000.csv"
            ]
        );
        assert!(fs
            .content(&fs.files()[2])
            .unwrap()
            .starts_with(b"\xEF\xBB\xBFqpc_ms"));
    }

    #[test]
    fn write_failure_ends_with_an_error_and_no_summary_if_empty() {
        let fs = MemFs::new();
        let files = BenchmarkFiles::new("b".into(), fs.clone());
        // The header write fails.
        fs.fail_write(1, Fault::Error(112));
        assert_eq!(
            files.begin("g-20261005-213000").err(),
            Some(WriteFailure::DiskFull)
        );
        assert!(fs.files().is_empty());
        // A data write fails: one error, the rest is dropped, no .json.
        let mut w = files.begin("h-20261005-213000").unwrap();
        fs.fail_write(3, Fault::Error(112));
        let big = vec!["x".repeat(FLUSH_BYTES)];
        assert_eq!(w.append(&big), Err(WriteFailure::DiskFull));
        assert_eq!(w.append(&big), Ok(()));
        assert_eq!(w.finish(None), Ok(()));
        assert!(fs.files().is_empty());
    }

    #[test]
    fn empty_capture_leaves_no_files() {
        let dir = temp_dir();
        let files = BenchmarkFiles::new(dir.clone(), Arc::new(RealFs));
        let w = files.begin("g-20261005-213000").unwrap();
        assert_eq!(w.finish(None), Ok(()));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finish_writes_csv_and_summary() {
        let dir = temp_dir();
        let files = BenchmarkFiles::new(dir.clone(), Arc::new(RealFs));
        let mut w = files.begin("g-20261005-213000").unwrap();
        w.append(&["1,2\r\n".to_owned()]).unwrap();
        w.finish(Some(&record(EndReason::User))).unwrap();
        let csv = std::fs::read(dir.join("g-20261005-213000.csv")).unwrap();
        assert!(csv.ends_with(b"1,2\r\n"));
        let list = files.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "g-20261005-213000");
        assert_eq!(list[0].record.end_reason, EndReason::User);
        assert_eq!(list[0].record.started_at, "2026-10-05T21:30:00");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn history_lists_newest_first_and_skips_bad_files() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let files = BenchmarkFiles::new(dir.clone(), Arc::new(RealFs));
        let json = serde_json::to_vec(&record(EndReason::Limit)).unwrap();
        // Name order and time order differ: "a" is newer than "z".
        std::fs::write(dir.join("z-20261001-100000.json"), &json).unwrap();
        std::fs::write(dir.join("a-20261005-100000.json"), &json).unwrap();
        std::fs::write(dir.join("a-20261005-100000-2.json"), &json).unwrap();
        std::fs::write(dir.join("a-20261005-100000-10.json"), &json).unwrap();
        std::fs::write(dir.join("broken-20261006-100000.json"), b"{").unwrap();
        std::fs::write(dir.join("not an id.json"), &json).unwrap();
        std::fs::write(
            dir.join("big-20261007-100000.json"),
            vec![b' '; MAX_SUMMARY_BYTES as usize + 1],
        )
        .unwrap();
        std::fs::write(dir.join("other.txt"), b"x").unwrap();
        let ids: Vec<_> = files.list().into_iter().map(|e| e.id).collect();
        assert_eq!(
            ids,
            [
                "a-20261005-100000-10",
                "a-20261005-100000-2",
                "a-20261005-100000",
                "z-20261001-100000"
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn benchmark_ids_with_separators_are_rejected() {
        assert!(is_benchmark_id("Control-20261005-213000"));
        assert!(is_benchmark_id("Control-20261005-213000-12"));
        for bad in [
            "",
            "..\\..\\Windows\\x",
            "../x-20261005-213000",
            "a/b-20261005-213000",
            "a\\b-20261005-213000",
            "C:\\x-20261005-213000",
            "a:b-20261005-213000",
            "a..b-20261005-213000",
            "x-20261005-213000.csv",
            "x-20261005-2130",
            "x-20261005-213000-123",
            "-20261005-213000",
            &format!("{}-20261005-213000", "x".repeat(96)),
        ] {
            assert!(!is_benchmark_id(bad), "{bad}");
        }
        let files = BenchmarkFiles::new(temp_dir(), Arc::new(RealFs));
        assert_eq!(files.csv_path("../x"), Err(BenchmarkError::InvalidId));
        assert_eq!(files.delete("..\\x"), Err(BenchmarkError::InvalidId));
    }

    #[test]
    fn delete_removes_csv_and_json() {
        let dir = temp_dir();
        let files = BenchmarkFiles::new(dir.clone(), Arc::new(RealFs));
        let mut w = files.begin("g-20261005-213000").unwrap();
        w.append(&["1\r\n".to_owned()]).unwrap();
        w.finish(Some(&record(EndReason::User))).unwrap();
        assert!(files.csv_path("g-20261005-213000").unwrap().exists());
        files.delete("g-20261005-213000").unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        assert_eq!(
            files.delete("g-20261005-213000"),
            Err(BenchmarkError::NotFound)
        );
        assert_eq!(
            files.csv_path("g-20261005-213000"),
            Err(BenchmarkError::NotFound)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
