//! The `oma-log-writer` thread: file parts, buffering, flushes and errors.
//!
//! It owns at most one open session. `Start` creates the first part
//! exclusively (with a `-2`…`-99` suffix on a name collision, L7) and writes
//! BOM and header before answering; rows are buffered and written when the
//! buffer reaches [`FLUSH_BYTES`] or [`FLUSH_EVERY`] after the last flush; a
//! row that does not fit, or has another layout, opens the next part. The
//! first failure of a session is reported once and its rows are dropped until
//! its `Stop`.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oma_core::csv::{self, Layout, PartDecision, BOM};

use super::fs::{LogFile, LogFs};
use super::queue::{Control, Item, LogQueue, Popped, Reply, Row};

/// The buffer is written to the file once it holds this many bytes.
pub const FLUSH_BYTES: usize = 64 * 1024;
/// The file is written and flushed at least this often while data is pending.
pub const FLUSH_EVERY: Duration = Duration::from_secs(5);

/// Highest session suffix tried when the file name is taken (L7).
const MAX_SUFFIX: u32 = 99;
/// Part numbers tried when the next part's name is taken.
const PART_ATTEMPTS: u32 = 99;

/// Why a session failed (L14).
#[derive(Debug, Clone, PartialEq)]
pub enum WriteFailure {
    DiskFull,
    Unavailable,
    Denied,
    HeaderTooLarge,
    TooManyCollisions,
    /// Any other error, with the system message.
    Other(String),
}

impl WriteFailure {
    /// Maps the Win32 error codes of L14: disk full, device gone or not
    /// ready, access denied; anything else keeps its message.
    pub fn from_io(error: &io::Error) -> Self {
        match error.raw_os_error() {
            // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL
            Some(39 | 112) => Self::DiskFull,
            // ERROR_PATH_NOT_FOUND, ERROR_INVALID_DRIVE, ERROR_NOT_READY,
            // ERROR_DEV_NOT_EXIST, ERROR_NO_SUCH_DEVICE, ERROR_FILE_INVALID
            // (an open handle on a dismounted volume), ERROR_IO_DEVICE,
            // ERROR_DEVICE_NOT_CONNECTED
            Some(3 | 15 | 21 | 55 | 433 | 1006 | 1117 | 1167) => Self::Unavailable,
            // ERROR_ACCESS_DENIED
            Some(5) => Self::Denied,
            _ => Self::Other(error.to_string()),
        }
    }

    /// i18n key of the reason (`TooManyCollisions` is an "other" error, L7).
    pub fn key(&self) -> &'static str {
        match self {
            Self::DiskFull => "log.error.diskFull",
            Self::Unavailable => "log.error.unavailable",
            Self::Denied => "log.error.denied",
            Self::HeaderTooLarge => "log.error.headerTooLarge",
            Self::TooManyCollisions | Self::Other(_) => "log.error.other",
        }
    }

    /// The `{detail}` of `log.error.other`.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Other(detail) => Some(detail.clone()),
            Self::TooManyCollisions => Some(format!(
                "more than {MAX_SUFFIX} log files with the same name"
            )),
            _ => None,
        }
    }
}

/// What the writer reports to the session coordinator.
#[derive(Debug, Clone, PartialEq)]
pub enum WriterEvent {
    /// After each write to the file and with each answer. The counters are
    /// cumulative for the session and count only completed writes; `bytes`
    /// includes BOM and headers, `part_bytes` is what the current part holds.
    Progress {
        session: u64,
        path: PathBuf,
        part: u32,
        part_bytes: u64,
        rows: u64,
        bytes: u64,
    },
    /// The first failure of a session, sent once (also when the command that
    /// met it answers with the same error).
    Failed { session: u64, failure: WriteFailure },
}

/// Starts the `oma-log-writer` thread; it exits once `queue` is closed and
/// drained, flushing the open session.
pub fn spawn_writer(
    queue: Arc<LogQueue>,
    fs: Arc<dyn LogFs>,
    events: Box<dyn Fn(WriterEvent) + Send>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("oma-log-writer".into())
        .spawn(move || run(&queue, Writer::new(fs, events), Instant::now))
        .expect("spawn the oma-log-writer thread")
}

fn run(queue: &LogQueue, mut writer: Writer, clock: impl Fn() -> Instant) {
    loop {
        match queue.pop(writer.next_wait(clock())) {
            Popped::Item(item) => writer.handle(item, clock()),
            Popped::TimedOut => {}
            Popped::Closed => {
                writer.finish(clock());
                return;
            }
        }
        // Also after every item, so a steady stream cannot postpone it.
        writer.on_timer(clock());
    }
}

/// `<stem>[-<suffix>][-part<part>].csv`, the shape of `csv::file_name`.
fn part_file_name(stem: &str, suffix: Option<u32>, part: u32) -> String {
    let mut name = String::from(stem);
    if let Some(suffix) = suffix {
        name.push_str(&format!("-{suffix}"));
    }
    if part > 1 {
        name.push_str(&format!("-part{part}"));
    }
    name.push_str(".csv");
    name
}

fn io_failure(error: io::Error) -> WriteFailure {
    WriteFailure::from_io(&error)
}

/// Answers a command; the coordinator may have stopped waiting.
fn answer(reply: Reply, result: Result<(), WriteFailure>) {
    let _ = reply.try_send(result);
}

fn not_open(session: u64) -> WriteFailure {
    WriteFailure::Other(format!("log session {session} is not open"))
}

type Events = Box<dyn Fn(WriterEvent) + Send>;

/// The writer's state machine; [`run`] feeds it from the queue and the clock.
pub(crate) struct Writer {
    fs: Arc<dyn LogFs>,
    events: Events,
    session: Option<Session>,
    /// Scratch for formatting a row.
    line: String,
}

struct Session {
    id: u64,
    dir: PathBuf,
    stem: String,
    suffix: Option<u32>,
    limit_bytes: u64,
    layout: Arc<Layout>,
    /// Header line of `layout`, without BOM.
    header: String,
    /// The open part; `None` once the session failed.
    file: Option<Box<dyn LogFile>>,
    /// Path and number of the current (or last) part.
    path: Option<PathBuf>,
    part: u32,
    /// BOM, header and rows of the current part, buffered ones included:
    /// what the rotation decision counts.
    part_bytes: u64,
    /// Completed writes to the current part.
    part_written: u64,
    /// Rows not written yet; at most `FLUSH_BYTES` plus one row.
    buffer: Vec<u8>,
    buffered_rows: u64,
    /// Written since the last flush.
    unflushed: bool,
    last_flush: Instant,
    rows: u64,
    bytes: u64,
    failure: Option<WriteFailure>,
}

impl Writer {
    pub(crate) fn new(fs: Arc<dyn LogFs>, events: Events) -> Self {
        Self {
            fs,
            events,
            session: None,
            line: String::new(),
        }
    }

    pub(crate) fn handle(&mut self, item: Item, now: Instant) {
        match item {
            Item::Row(row) => self.row(row, now),
            Item::Control(control) => self.control(control, now),
        }
    }

    /// Writes and flushes pending data [`FLUSH_EVERY`] after the last flush.
    pub(crate) fn on_timer(&mut self, now: Instant) {
        let events = &*self.events;
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if session.pending() && now >= session.deadline() {
            if let Err(failure) = session.flush(events, now) {
                session.fail(failure, events);
            }
        }
    }

    /// How long the thread may sleep: until the flush deadline while data is
    /// pending, otherwise without limit.
    pub(crate) fn next_wait(&self, now: Instant) -> Option<Duration> {
        let session = self.session.as_ref()?;
        session
            .pending()
            .then(|| session.deadline().saturating_duration_since(now))
    }

    /// The queue closed: flush and close the open session.
    pub(crate) fn finish(&mut self, now: Instant) {
        self.close_session(now);
    }

    fn close_session(&mut self, now: Instant) {
        let events = &*self.events;
        if let Some(mut session) = self.session.take() {
            if session.failure.is_none() {
                if let Err(failure) = session.flush(events, now) {
                    session.fail(failure, events);
                }
            }
        }
    }

    fn row(&mut self, row: Row, now: Instant) {
        let events = &*self.events;
        let Some(session) = self
            .session
            .as_mut()
            .filter(|s| s.id == row.session && s.failure.is_none())
        else {
            return;
        };
        if let Err(failure) = session.append(row, &*self.fs, events, &mut self.line, now) {
            session.fail(failure, events);
        }
    }

    fn control(&mut self, control: Control, now: Instant) {
        match control {
            Control::Start {
                session,
                layout,
                dir,
                stem,
                limit_bytes,
                reply,
            } => {
                // A start without the stop of the previous session closes it.
                self.close_session(now);
                let events = &*self.events;
                let mut opened = Session::new(session, layout, dir, stem, limit_bytes, now);
                let result = opened.open_first(&*self.fs, now);
                if let Err(failure) = &result {
                    opened.fail(failure.clone(), events);
                }
                opened.progress(events);
                self.session = Some(opened);
                answer(reply, result);
            }
            Control::Pause { session, reply } => {
                let events = &*self.events;
                let result = match self.session.as_mut().filter(|s| s.id == session) {
                    Some(open) => {
                        let result = match open.failure.clone() {
                            Some(failure) => Err(failure),
                            None => open.flush(events, now),
                        };
                        if let Err(failure) = &result {
                            open.fail(failure.clone(), events);
                        }
                        open.progress(events);
                        result
                    }
                    None => Err(not_open(session)),
                };
                answer(reply, result);
            }
            Control::Resume { session, reply } => {
                let events = &*self.events;
                let result = match self.session.as_ref().filter(|s| s.id == session) {
                    Some(open) => {
                        open.progress(events);
                        open.failure.clone().map_or(Ok(()), Err)
                    }
                    None => Err(not_open(session)),
                };
                answer(reply, result);
            }
            Control::Stop { session, reply } => {
                let events = &*self.events;
                // Stopping a session that is not open (or no longer) only
                // confirms that nothing is held for it; after a failure the
                // stop only confirms the release.
                let mut result = Ok(());
                if self.session.as_ref().is_some_and(|s| s.id == session) {
                    if let Some(mut open) = self.session.take() {
                        if open.failure.is_none() {
                            result = open.flush(events, now);
                            if let Err(failure) = &result {
                                open.fail(failure.clone(), events);
                            }
                        }
                        open.progress(events);
                    }
                }
                answer(reply, result);
            }
        }
    }
}

impl Session {
    fn new(
        id: u64,
        layout: Arc<Layout>,
        dir: PathBuf,
        stem: String,
        limit_bytes: u64,
        now: Instant,
    ) -> Self {
        let mut header = String::new();
        csv::header_line(&layout, &mut header);
        Self {
            id,
            dir,
            stem,
            suffix: None,
            limit_bytes,
            layout,
            header,
            file: None,
            path: None,
            part: 1,
            part_bytes: 0,
            part_written: 0,
            buffer: Vec::new(),
            buffered_rows: 0,
            unflushed: false,
            last_flush: now,
            rows: 0,
            bytes: 0,
            failure: None,
        }
    }

    /// Folder and first part, trying the session suffixes on a collision;
    /// the header is checked against the limit before anything is created.
    fn open_first(&mut self, fs: &dyn LogFs, now: Instant) -> Result<(), WriteFailure> {
        if (BOM.len() + self.header.len()) as u64 > self.limit_bytes {
            return Err(WriteFailure::HeaderTooLarge);
        }
        fs.create_dir_all(&self.dir).map_err(io_failure)?;
        for suffix in std::iter::once(None).chain((2..=MAX_SUFFIX).map(Some)) {
            let path = self.dir.join(part_file_name(&self.stem, suffix, 1));
            match fs.create_new(&path) {
                Ok(file) => {
                    self.suffix = suffix;
                    return self.begin_part(file, path, 1, now);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(io_failure(error)),
            }
        }
        Err(WriteFailure::TooManyCollisions)
    }

    /// Flushes and closes the current part, then opens the first free part
    /// number after it (an existing file is never overwritten).
    fn next_part(
        &mut self,
        fs: &dyn LogFs,
        events: &dyn Fn(WriterEvent),
        now: Instant,
    ) -> Result<(), WriteFailure> {
        self.flush(events, now)?;
        self.file = None;
        let first = self.part + 1;
        for part in first..first.saturating_add(PART_ATTEMPTS) {
            let path = self.dir.join(part_file_name(&self.stem, self.suffix, part));
            match fs.create_new(&path) {
                Ok(file) => {
                    self.begin_part(file, path, part, now)?;
                    self.progress(events);
                    return Ok(());
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(io_failure(error)),
            }
        }
        Err(WriteFailure::TooManyCollisions)
    }

    /// Writes BOM and header to a freshly created part and flushes it.
    fn begin_part(
        &mut self,
        file: Box<dyn LogFile>,
        path: PathBuf,
        part: u32,
        now: Instant,
    ) -> Result<(), WriteFailure> {
        self.path = Some(path);
        self.part = part;
        self.part_written = 0;
        let mut head = Vec::with_capacity(BOM.len() + self.header.len());
        head.extend_from_slice(BOM);
        head.extend_from_slice(self.header.as_bytes());
        self.part_bytes = head.len() as u64;
        let file = self.file.insert(file);
        file.write_all(&head).map_err(io_failure)?;
        self.part_written = head.len() as u64;
        self.bytes += head.len() as u64;
        file.flush().map_err(io_failure)?;
        self.unflushed = false;
        self.last_flush = now;
        Ok(())
    }

    fn append(
        &mut self,
        row: Row,
        fs: &dyn LogFs,
        events: &dyn Fn(WriterEvent),
        line: &mut String,
        now: Instant,
    ) -> Result<(), WriteFailure> {
        line.clear();
        csv::row_line(
            &row.layout,
            row.timestamp_ms,
            row.offset_minutes,
            &row.values,
            line,
        );
        let row_bytes = line.len() as u64;
        if !Arc::ptr_eq(&row.layout, &self.layout) {
            if row.layout.same_output(&self.layout) {
                // Same columns (maybe at other schema indexes): same part.
                self.layout = row.layout;
            } else {
                let mut header = String::new();
                csv::header_line(&row.layout, &mut header);
                let empty = (BOM.len() + header.len()) as u64;
                if empty.saturating_add(row_bytes) > self.limit_bytes {
                    // Checked before a part is created; keep what was accepted.
                    self.flush(events, now)?;
                    return Err(WriteFailure::HeaderTooLarge);
                }
                self.layout = row.layout;
                self.header = header;
                self.next_part(fs, events, now)?;
            }
        }
        match csv::part_decision(
            self.part_bytes,
            row_bytes,
            self.header.len() as u64,
            self.limit_bytes,
        ) {
            PartDecision::Append => {}
            PartDecision::NewPart => self.next_part(fs, events, now)?,
            PartDecision::TooLarge => {
                self.flush(events, now)?;
                return Err(WriteFailure::HeaderTooLarge);
            }
        }
        self.buffer.extend_from_slice(line.as_bytes());
        self.part_bytes += row_bytes;
        self.buffered_rows += 1;
        if self.buffer.len() >= FLUSH_BYTES {
            self.write_out(events)?;
        }
        Ok(())
    }

    /// Hands the buffer to the file; only a completed write counts.
    fn write_out(&mut self, events: &dyn Fn(WriterEvent)) -> Result<(), WriteFailure> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        let result = file.write_all(&self.buffer);
        let len = self.buffer.len() as u64;
        let rows = std::mem::take(&mut self.buffered_rows);
        self.buffer.clear();
        result.map_err(io_failure)?;
        self.unflushed = true;
        self.part_written += len;
        self.bytes += len;
        self.rows += rows;
        self.progress(events);
        Ok(())
    }

    fn flush(&mut self, events: &dyn Fn(WriterEvent), now: Instant) -> Result<(), WriteFailure> {
        self.write_out(events)?;
        if let Some(file) = self.file.as_mut() {
            file.flush().map_err(io_failure)?;
        }
        self.unflushed = false;
        self.last_flush = now;
        Ok(())
    }

    fn pending(&self) -> bool {
        self.file.is_some() && (!self.buffer.is_empty() || self.unflushed)
    }

    fn deadline(&self) -> Instant {
        self.last_flush + FLUSH_EVERY
    }

    /// Records the first failure (reported once), closes the file and frees
    /// the buffer; later rows of the session are dropped.
    fn fail(&mut self, failure: WriteFailure, events: &dyn Fn(WriterEvent)) {
        self.file = None;
        self.buffer = Vec::new();
        self.buffered_rows = 0;
        self.unflushed = false;
        if self.failure.is_none() {
            self.failure = Some(failure.clone());
            events(WriterEvent::Failed {
                session: self.id,
                failure,
            });
        }
    }

    fn progress(&self, events: &dyn Fn(WriterEvent)) {
        if let Some(path) = &self.path {
            events(WriterEvent::Progress {
                session: self.id,
                path: path.clone(),
                part: self.part,
                part_bytes: self.part_written,
                rows: self.rows,
                bytes: self.bytes,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::fs::fake::{Fault, MemFs};
    use crate::log::queue::{Control, Reply, Row, MAX_QUEUE_BYTES, MAX_ROWS};
    use oma_core::csv::{self, Column, Conversion, Layout, BOM};
    use std::sync::mpsc::{sync_channel, Receiver};
    use std::sync::Mutex;

    const STEM: &str = "oma-2026-09-29_14-03-12";
    const T0: u64 = 1_790_683_392_000; // 2026-09-29T12:03:12Z
    const OFFSET: i32 = 120;
    const BIG: u64 = 100 * 1024 * 1024;

    type Answer = Receiver<Result<(), WriteFailure>>;

    fn dir() -> PathBuf {
        PathBuf::from("logs")
    }

    fn path(name: &str) -> PathBuf {
        dir().join(name)
    }

    fn base() -> PathBuf {
        path(&format!("{STEM}.csv"))
    }

    fn layout_of(columns: &[(&str, &str, usize)]) -> Arc<Layout> {
        Arc::new(Layout {
            columns: columns
                .iter()
                .map(|&(id, label, index)| Column {
                    sensor_id: id.into(),
                    index,
                    device: "CPU".into(),
                    label: label.into(),
                    unit: "%",
                    conversion: Conversion::None,
                })
                .collect(),
        })
    }

    fn layout_a() -> Arc<Layout> {
        layout_of(&[("cpu/load/total", "Load", 0)])
    }

    fn layout_b() -> Arc<Layout> {
        layout_of(&[
            ("cpu/load/total", "Load", 0),
            ("cpu/temp/pkg", "Package", 1),
        ])
    }

    fn head(layout: &Layout) -> Vec<u8> {
        let mut header = String::new();
        csv::header_line(layout, &mut header);
        let mut bytes = BOM.to_vec();
        bytes.extend_from_slice(header.as_bytes());
        bytes
    }

    fn line(layout: &Layout, n: u64) -> Vec<u8> {
        let values = vec![Some(n as f64); layout.columns.len()];
        let mut out = String::new();
        csv::row_line(layout, T0 + n * 1000, OFFSET, &values, &mut out);
        out.into_bytes()
    }

    fn row(session: u64, layout: &Arc<Layout>, n: u64) -> Row {
        Row {
            session,
            layout: layout.clone(),
            timestamp_ms: T0 + n * 1000,
            offset_minutes: OFFSET,
            values: vec![Some(n as f64); layout.columns.len()].into_boxed_slice(),
        }
    }

    /// `MemFs::files` lists paths sorted.
    fn sorted(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
        paths.sort();
        paths
    }

    fn concat(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    fn start_control(session: u64, layout: &Arc<Layout>, limit: u64) -> (Control, Answer) {
        let (tx, rx) = sync_channel(1);
        let control = Control::Start {
            session,
            layout: layout.clone(),
            dir: dir(),
            stem: STEM.into(),
            limit_bytes: limit,
            reply: tx,
        };
        (control, rx)
    }

    fn command(make: fn(u64, Reply) -> Control, session: u64) -> (Control, Answer) {
        let (tx, rx) = sync_channel(1);
        (make(session, tx), rx)
    }

    fn pause(session: u64, reply: Reply) -> Control {
        Control::Pause { session, reply }
    }

    fn resume(session: u64, reply: Reply) -> Control {
        Control::Resume { session, reply }
    }

    fn stop(session: u64, reply: Reply) -> Control {
        Control::Stop { session, reply }
    }

    type Events = Arc<Mutex<Vec<WriterEvent>>>;

    fn recorder() -> (Events, Box<dyn Fn(WriterEvent) + Send>) {
        let events: Events = Arc::default();
        let sink = events.clone();
        (events, Box::new(move |e| sink.lock().unwrap().push(e)))
    }

    fn failures(events: &Events) -> Vec<WriteFailure> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                WriterEvent::Failed { failure, .. } => Some(failure.clone()),
                WriterEvent::Progress { .. } => None,
            })
            .collect()
    }

    /// `(part, part_bytes, rows, bytes)` of the last `Progress`.
    fn last_progress(events: &Events) -> Option<(PathBuf, u32, u64, u64, u64)> {
        events.lock().unwrap().iter().rev().find_map(|e| match e {
            WriterEvent::Progress {
                path,
                part,
                part_bytes,
                rows,
                bytes,
                ..
            } => Some((path.clone(), *part, *part_bytes, *rows, *bytes)),
            WriterEvent::Failed { .. } => None,
        })
    }

    /// The writer state machine driven by hand, with a fake clock.
    struct Harness {
        fs: Arc<MemFs>,
        events: Events,
        writer: Writer,
        t0: Instant,
    }

    impl Harness {
        fn new() -> Self {
            Self::with_fs(MemFs::new())
        }

        fn with_fs(fs: Arc<MemFs>) -> Self {
            let (events, sink) = recorder();
            Self {
                writer: Writer::new(fs.clone(), sink),
                fs,
                events,
                t0: Instant::now(),
            }
        }

        fn at(&self, secs: f64) -> Instant {
            self.t0 + Duration::from_secs_f64(secs)
        }

        fn control(
            &mut self,
            (control, rx): (Control, Answer),
            secs: f64,
        ) -> Result<(), WriteFailure> {
            let now = self.at(secs);
            self.writer.handle(Item::Control(control), now);
            self.writer.on_timer(now);
            rx.try_recv().expect("answered before handle returns")
        }

        fn start(
            &mut self,
            session: u64,
            layout: &Arc<Layout>,
            limit: u64,
        ) -> Result<(), WriteFailure> {
            self.control(start_control(session, layout, limit), 0.0)
        }

        fn row(&mut self, row: Row, secs: f64) {
            let now = self.at(secs);
            self.writer.handle(Item::Row(row), now);
            self.writer.on_timer(now);
        }
    }

    /// Runs the real thread over a queue filled beforehand, so that every
    /// row the test pushes is queued in front of the writer.
    fn run_thread(fs: &Arc<MemFs>, queue: &Arc<LogQueue>) -> (Events, JoinHandle<()>) {
        let (events, sink) = recorder();
        let handle = spawn_writer(queue.clone(), fs.clone(), sink);
        (events, handle)
    }

    fn wait(rx: &Answer) -> Result<(), WriteFailure> {
        rx.recv_timeout(Duration::from_secs(10))
            .expect("writer answers")
    }

    #[test]
    fn part_names_match_the_core_file_name() {
        let start = csv::local_time(T0, OFFSET);
        for (suffix, part) in [
            (None, 1),
            (None, 2),
            (Some(2), 1),
            (Some(2), 3),
            (Some(99), 12),
        ] {
            assert_eq!(
                part_file_name(STEM, suffix, part),
                csv::file_name(start, suffix, part)
            );
        }
    }

    #[test]
    fn start_creates_the_file_exclusively_with_bom_and_header() {
        let mut h = Harness::new();
        let a = layout_a();
        assert_eq!(h.start(1, &a, BIG), Ok(()));

        assert_eq!(h.fs.dirs(), vec![dir()]);
        assert_eq!(h.fs.files(), vec![base()]);
        let expected = concat(&[BOM, b"Timestamp,CPU / Load [%] {cpu/load/total}\r\n"]);
        assert_eq!(head(&a), expected);
        // Written and flushed before the answer, before any row.
        assert_eq!(h.fs.flushed(&base()), Some(expected.clone()));
        assert_eq!(
            last_progress(&h.events),
            Some((base(), 1, expected.len() as u64, 0, expected.len() as u64))
        );
        assert!(failures(&h.events).is_empty());
    }

    #[test]
    fn name_collision_uses_a_session_suffix() {
        let fs = MemFs::new();
        fs.insert(base(), b"old");
        let mut h = Harness::with_fs(fs);
        let a = layout_a();
        let limit = (head(&a).len() + line(&a, 1).len()) as u64;
        assert_eq!(h.start(1, &a, limit), Ok(()));
        h.row(row(1, &a, 1), 1.0);
        h.row(row(1, &a, 2), 2.0);
        assert_eq!(h.control(command(stop, 1), 3.0), Ok(()));

        let second = path(&format!("{STEM}-2.csv"));
        let second_part = path(&format!("{STEM}-2-part2.csv"));
        assert_eq!(
            h.fs.files(),
            sorted(vec![base(), second.clone(), second_part.clone()])
        );
        assert_eq!(h.fs.content(&base()), Some(b"old".to_vec()));
        assert_eq!(
            h.fs.content(&second),
            Some(concat(&[&head(&a), &line(&a, 1)]))
        );
        assert_eq!(
            h.fs.content(&second_part),
            Some(concat(&[&head(&a), &line(&a, 2)]))
        );
    }

    #[test]
    fn too_many_collisions_fail() {
        let fs = MemFs::new();
        fs.insert(base(), b"old");
        for suffix in 2..=99 {
            fs.insert(path(&format!("{STEM}-{suffix}.csv")), b"old");
        }
        let mut h = Harness::with_fs(fs);
        let a = layout_a();
        assert_eq!(h.start(1, &a, BIG), Err(WriteFailure::TooManyCollisions));
        assert_eq!(failures(&h.events), vec![WriteFailure::TooManyCollisions]);
        assert_eq!(WriteFailure::TooManyCollisions.key(), "log.error.other");
        assert_eq!(h.fs.files().len(), 99);
        // The failed session drops its rows and its Stop only confirms.
        h.row(row(1, &a, 1), 1.0);
        assert_eq!(h.control(command(stop, 1), 2.0), Ok(()));
        assert_eq!(h.fs.files().len(), 99);
        assert_eq!(failures(&h.events).len(), 1);
    }

    #[test]
    fn part_collision_preserves_existing_file() {
        let fs = MemFs::new();
        let part2 = path(&format!("{STEM}-part2.csv"));
        fs.insert(part2.clone(), b"keep");
        let mut h = Harness::with_fs(fs);
        let a = layout_a();
        let limit = (head(&a).len() + line(&a, 1).len()) as u64;
        assert_eq!(h.start(1, &a, limit), Ok(()));
        h.row(row(1, &a, 1), 1.0);
        h.row(row(1, &a, 2), 2.0);
        assert_eq!(h.control(command(stop, 1), 3.0), Ok(()));

        let part3 = path(&format!("{STEM}-part3.csv"));
        assert_eq!(h.fs.content(&part2), Some(b"keep".to_vec()));
        assert_eq!(
            h.fs.content(&part3),
            Some(concat(&[&head(&a), &line(&a, 2)]))
        );
        let (last, part, ..) = last_progress(&h.events).unwrap();
        assert_eq!((last, part), (part3, 3));
    }

    #[test]
    fn new_part_on_size_and_on_layout_change() {
        let mut h = Harness::new();
        let a = layout_a();
        let b = layout_b();
        let limit = (head(&b).len() + 2 * line(&b, 1).len()) as u64;
        assert_eq!(h.start(1, &a, limit), Ok(()));
        h.row(row(1, &a, 1), 1.0);
        h.row(row(1, &a, 2), 2.0);
        // Same output with another schema index: no new part.
        let a_moved = layout_of(&[("cpu/load/total", "Load", 5)]);
        h.row(row(1, &a_moved, 3), 3.0);
        // Too big for part 1 now.
        let small = (head(&a).len() + 3 * line(&a, 1).len()) as u64;
        assert!(small <= limit && small + line(&a, 4).len() as u64 > limit);
        h.row(row(1, &a, 4), 4.0);
        // Layout change: part 3 with the new header.
        h.row(row(1, &b, 5), 5.0);
        assert_eq!(h.control(command(stop, 1), 6.0), Ok(()));

        let part2 = path(&format!("{STEM}-part2.csv"));
        let part3 = path(&format!("{STEM}-part3.csv"));
        assert_eq!(
            h.fs.files(),
            sorted(vec![base(), part2.clone(), part3.clone()])
        );
        assert_eq!(
            h.fs.content(&base()),
            Some(concat(&[
                &head(&a),
                &line(&a, 1),
                &line(&a, 2),
                &line(&a, 3)
            ]))
        );
        assert_eq!(
            h.fs.content(&part2),
            Some(concat(&[&head(&a), &line(&a, 4)]))
        );
        assert_eq!(
            h.fs.content(&part3),
            Some(concat(&[&head(&b), &line(&b, 5)]))
        );
        let (_, part, part_bytes, rows, bytes) = last_progress(&h.events).unwrap();
        assert_eq!(part, 3);
        assert_eq!(part_bytes, (head(&b).len() + line(&b, 5).len()) as u64);
        assert_eq!(rows, 5);
        let total: usize = [&base(), &part2, &part3]
            .iter()
            .map(|p| h.fs.content(p).unwrap().len())
            .sum();
        assert_eq!(bytes, total as u64);
    }

    #[test]
    fn oversized_row_fails_without_rotating() {
        let mut h = Harness::new();
        let a = layout_a();
        let b = layout_b();
        // Room for header + one row of `a`, not for header + a row of `b`.
        let limit = (head(&a).len() + line(&a, 1).len()) as u64;
        assert!((head(&b).len() + line(&b, 1).len()) as u64 > limit);
        assert_eq!(h.start(1, &a, limit), Ok(()));
        h.row(row(1, &a, 1), 1.0);
        h.row(row(1, &b, 2), 2.0);
        assert_eq!(failures(&h.events), vec![WriteFailure::HeaderTooLarge]);
        // No new part; the row accepted before is on disk.
        assert_eq!(h.fs.files(), vec![base()]);
        assert_eq!(
            h.fs.flushed(&base()),
            Some(concat(&[&head(&a), &line(&a, 1)]))
        );
        assert_eq!(h.control(command(stop, 1), 3.0), Ok(()));

        // A header alone over the limit fails the start without a file.
        let mut h = Harness::new();
        assert_eq!(
            h.start(1, &a, head(&a).len() as u64 - 1),
            Err(WriteFailure::HeaderTooLarge)
        );
        assert!(h.fs.files().is_empty());

        // A row too big for a fresh part fails as well.
        let mut h = Harness::new();
        let wide = layout_of(&[("cpu/load/total", "L", 0)]);
        assert_eq!(h.start(1, &wide, head(&wide).len() as u64 + 3), Ok(()));
        h.row(row(1, &wide, 1), 1.0);
        assert_eq!(failures(&h.events), vec![WriteFailure::HeaderTooLarge]);
        assert_eq!(h.fs.files(), vec![base()]);
    }

    #[test]
    fn buffered_bytes_count_toward_rotation() {
        let mut h = Harness::new();
        let a = layout_a();
        let limit = (head(&a).len() + 2 * line(&a, 1).len()) as u64;
        assert_eq!(h.start(1, &a, limit), Ok(()));
        h.row(row(1, &a, 1), 0.1);
        h.row(row(1, &a, 2), 0.2);
        // Nothing written yet, but part 1 is full.
        assert_eq!(h.fs.content(&base()), Some(head(&a)));
        h.row(row(1, &a, 3), 0.3);
        let part2 = path(&format!("{STEM}-part2.csv"));
        assert_eq!(
            h.fs.flushed(&base()),
            Some(concat(&[&head(&a), &line(&a, 1), &line(&a, 2)]))
        );
        // Row 3 waits in the buffer of part 2.
        assert_eq!(h.fs.content(&part2), Some(head(&a)));
    }

    #[test]
    fn large_buffer_is_written_without_waiting() {
        let mut h = Harness::new();
        let a = layout_a();
        assert_eq!(h.start(1, &a, BIG), Ok(()));
        let per_row = line(&a, 1).len();
        let rows = FLUSH_BYTES.div_ceil(per_row) as u64;
        for _ in 1..rows {
            h.row(row(1, &a, 1), 0.1);
        }
        assert_eq!(h.fs.content(&base()), Some(head(&a)));
        h.row(row(1, &a, 1), 0.1);
        let written = h.fs.content(&base()).unwrap().len() - head(&a).len();
        assert_eq!(written, rows as usize * per_row);
        assert!(written >= FLUSH_BYTES && written < FLUSH_BYTES + per_row);
        let (_, _, _, counted, _) = last_progress(&h.events).unwrap();
        assert_eq!(counted, rows);
    }

    #[test]
    fn continuous_rows_do_not_postpone_flush() {
        let mut h = Harness::new();
        let a = layout_a();
        assert_eq!(h.start(1, &a, BIG), Ok(()));
        assert_eq!(h.fs.flush_count(&base()), 1);
        for n in 1..=4 {
            h.row(row(1, &a, n), n as f64);
            assert_eq!(h.fs.flushed(&base()), Some(head(&a)), "row {n}");
        }
        // 5 s after the last flush, though a row arrived 1 s ago.
        h.row(row(1, &a, 5), 5.0);
        assert_eq!(h.fs.flush_count(&base()), 2);
        let mut expected = head(&a);
        for n in 1..=5 {
            expected.extend_from_slice(&line(&a, n));
        }
        assert_eq!(h.fs.flushed(&base()), Some(expected));
        h.row(row(1, &a, 6), 6.0);
        assert_eq!(h.writer.next_wait(h.at(6.0)), Some(Duration::from_secs(4)));
    }

    #[test]
    fn stop_drains_accepted_rows_before_confirming() {
        let fs = MemFs::new();
        let queue = LogQueue::new(MAX_ROWS, MAX_QUEUE_BYTES);
        let a = layout_a();
        let (start, started) = start_control(1, &a, BIG);
        queue.push_control(start).unwrap();
        for n in 1..=10 {
            queue.try_push_row(row(1, &a, n)).unwrap();
        }
        let (first, stopped) = command(stop, 1);
        queue.push_control(first).unwrap();
        let (events, handle) = run_thread(&fs, &queue);

        assert_eq!(wait(&started), Ok(()));
        assert_eq!(wait(&stopped), Ok(()));
        let mut expected = head(&a);
        for n in 1..=10 {
            expected.extend_from_slice(&line(&a, n));
        }
        assert_eq!(fs.flushed(&base()), Some(expected.clone()));
        let (_, _, _, rows, bytes) = last_progress(&events).unwrap();
        assert_eq!((rows, bytes), (10, expected.len() as u64));

        // Rows pushed while the writer runs: each one accepted is written.
        let (start, started) = start_control(2, &a, BIG);
        queue.push_control(start).unwrap();
        assert_eq!(wait(&started), Ok(()));
        let second = path(&format!("{STEM}-2.csv"));
        let mut expected = head(&a);
        for n in 1..=20 {
            if queue.try_push_row(row(2, &a, n)).is_ok() {
                expected.extend_from_slice(&line(&a, n));
            }
        }
        let (last, stopped) = command(stop, 2);
        queue.push_control(last).unwrap();
        assert_eq!(wait(&stopped), Ok(()));
        assert_eq!(fs.flushed(&second), Some(expected));

        queue.close();
        handle.join().unwrap();
    }

    #[test]
    fn pause_flushes_before_confirming() {
        let mut h = Harness::new();
        let a = layout_a();
        assert_eq!(h.start(1, &a, BIG), Ok(()));
        h.row(row(1, &a, 1), 0.5);
        h.row(row(1, &a, 2), 1.0);
        assert_eq!(h.fs.flushed(&base()), Some(head(&a)));
        assert_eq!(h.control(command(pause, 1), 1.5), Ok(()));
        let expected = concat(&[&head(&a), &line(&a, 1), &line(&a, 2)]);
        assert_eq!(h.fs.flushed(&base()), Some(expected.clone()));
        let (_, _, _, rows, bytes) = last_progress(&h.events).unwrap();
        assert_eq!((rows, bytes), (2, expected.len() as u64));
        // Nothing pending while paused: the writer sleeps without a deadline.
        assert_eq!(h.writer.next_wait(h.at(1.5)), None);
        assert_eq!(h.control(command(resume, 1), 10.0), Ok(()));
    }

    #[test]
    fn buffer_is_written_every_five_seconds_even_when_paused() {
        let mut h = Harness::new();
        let a = layout_a();
        assert_eq!(h.start(1, &a, BIG), Ok(()));
        h.row(row(1, &a, 1), 1.0);
        // Then no item arrives (as while paused): the timer alone writes.
        assert_eq!(h.writer.next_wait(h.at(1.0)), Some(Duration::from_secs(4)));
        h.writer.on_timer(h.at(4.9));
        assert_eq!(h.fs.content(&base()), Some(head(&a)));
        assert_eq!(
            h.writer.next_wait(h.at(4.9)).map(|d| d.as_millis()),
            Some(100)
        );
        h.writer.on_timer(h.at(5.0));
        let expected = concat(&[&head(&a), &line(&a, 1)]);
        assert_eq!(h.fs.flushed(&base()), Some(expected));
        assert_eq!(h.writer.next_wait(h.at(5.0)), None);

        // A row accepted before the pause barrier but still buffered after
        // it cannot happen (pause flushes), and pause keeps the timer: a row
        // buffered right after resuming is written 5 s after the pause flush.
        assert_eq!(h.control(command(pause, 1), 6.0), Ok(()));
        assert_eq!(h.control(command(resume, 1), 7.0), Ok(()));
        h.row(row(1, &a, 2), 7.0);
        assert_eq!(h.writer.next_wait(h.at(7.0)), Some(Duration::from_secs(4)));
        h.writer.on_timer(h.at(11.0));
        assert_eq!(
            h.fs.flushed(&base()),
            Some(concat(&[&head(&a), &line(&a, 1), &line(&a, 2)]))
        );
    }

    #[test]
    fn write_error_fails_the_session_and_keeps_draining() {
        let fs = MemFs::new();
        // Write 1 is the header, write 2 the first buffered rows.
        fs.fail_write(2, Fault::Error(112));
        let queue = LogQueue::new(MAX_ROWS, MAX_QUEUE_BYTES);
        let a = layout_a();
        let (start, started) = start_control(1, &a, BIG);
        queue.push_control(start).unwrap();
        queue.try_push_row(row(1, &a, 1)).unwrap();
        let (pause, paused) = command(pause, 1);
        queue.push_control(pause).unwrap();
        queue.try_push_row(row(1, &a, 2)).unwrap();
        queue.try_push_row(row(1, &a, 3)).unwrap();
        let (resume, resumed) = command(resume, 1);
        queue.push_control(resume).unwrap();
        let (stop, stopped) = command(stop, 1);
        queue.push_control(stop).unwrap();
        let (events, handle) = run_thread(&fs, &queue);

        assert_eq!(wait(&started), Ok(()));
        assert_eq!(wait(&paused), Err(WriteFailure::DiskFull));
        assert_eq!(wait(&resumed), Err(WriteFailure::DiskFull));
        assert_eq!(wait(&stopped), Ok(()));
        assert_eq!(failures(&events), vec![WriteFailure::DiskFull]);
        assert_eq!(fs.content(&base()), Some(head(&a)));

        // The writer is still alive for the next session.
        let (start, started) = start_control(2, &a, BIG);
        queue.push_control(start).unwrap();
        assert_eq!(wait(&started), Ok(()));
        queue.close();
        handle.join().unwrap();
        assert!(fs.content(&path(&format!("{STEM}-2.csv"))).is_some());
    }

    #[test]
    fn partial_write_does_not_count_a_complete_row() {
        let fs = MemFs::new();
        fs.fail_write(2, Fault::Partial(112));
        let mut h = Harness::with_fs(fs);
        let a = layout_a();
        assert_eq!(h.start(1, &a, BIG), Ok(()));
        h.row(row(1, &a, 1), 0.5);
        h.row(row(1, &a, 2), 1.0);
        assert_eq!(
            h.control(command(pause, 1), 1.5),
            Err(WriteFailure::DiskFull)
        );
        assert_eq!(h.control(command(stop, 1), 2.0), Ok(()));
        // Half a buffer reached the file; none of it counts.
        assert!(h.fs.content(&base()).unwrap().len() > head(&a).len());
        let (_, _, part_bytes, rows, bytes) = last_progress(&h.events).unwrap();
        let header = head(&a).len() as u64;
        assert_eq!((part_bytes, rows, bytes), (header, 0, header));
        assert_eq!(failures(&h.events), vec![WriteFailure::DiskFull]);
    }

    #[test]
    fn flush_error_fails_the_session() {
        // The flush of the header at the start.
        let fs = MemFs::new();
        fs.fail_flush(1, 1167);
        let mut h = Harness::with_fs(fs);
        let a = layout_a();
        assert_eq!(h.start(1, &a, BIG), Err(WriteFailure::Unavailable));
        assert_eq!(failures(&h.events), vec![WriteFailure::Unavailable]);
        h.row(row(1, &a, 1), 1.0);
        assert_eq!(
            h.control(command(pause, 1), 2.0),
            Err(WriteFailure::Unavailable)
        );
        assert_eq!(h.control(command(stop, 1), 3.0), Ok(()));
        assert_eq!(h.fs.content(&base()), Some(head(&a)));

        // The flush of the stop: stop reports it, once.
        let fs = MemFs::new();
        fs.fail_flush(2, 5);
        let mut h = Harness::with_fs(fs);
        assert_eq!(h.start(1, &a, BIG), Ok(()));
        h.row(row(1, &a, 1), 1.0);
        assert_eq!(h.control(command(stop, 1), 2.0), Err(WriteFailure::Denied));
        assert_eq!(failures(&h.events), vec![WriteFailure::Denied]);
        // A late second stop only confirms.
        assert_eq!(h.control(command(stop, 1), 3.0), Ok(()));
        assert_eq!(failures(&h.events).len(), 1);
    }

    #[test]
    fn rows_of_an_old_session_are_ignored() {
        let mut h = Harness::new();
        let a = layout_a();
        assert_eq!(h.start(1, &a, BIG), Ok(()));
        h.row(row(1, &a, 1), 0.5);
        // A new start closes session 1 (its rows are kept).
        assert_eq!(h.control(start_control(2, &a, BIG), 1.0), Ok(()));
        h.row(row(1, &a, 2), 1.5);
        h.row(row(2, &a, 3), 2.0);
        assert_eq!(h.control(command(stop, 1), 2.5), Ok(()));
        assert_eq!(h.control(command(stop, 2), 3.0), Ok(()));
        h.row(row(2, &a, 4), 3.5);

        let second = path(&format!("{STEM}-2.csv"));
        assert_eq!(
            h.fs.content(&base()),
            Some(concat(&[&head(&a), &line(&a, 1)]))
        );
        assert_eq!(
            h.fs.content(&second),
            Some(concat(&[&head(&a), &line(&a, 3)]))
        );
        // Pause of a session that is not open is refused.
        assert!(h.control(command(pause, 1), 4.0).is_err());
    }

    #[test]
    fn full_queue_during_pause_and_stop_keeps_the_barriers() {
        let fs = MemFs::new();
        let queue = LogQueue::new(3, MAX_QUEUE_BYTES);
        let a = layout_a();
        let (start, started) = start_control(1, &a, BIG);
        queue.push_control(start).unwrap();
        for n in 1..=3 {
            queue.try_push_row(row(1, &a, n)).unwrap();
        }
        assert!(queue.try_push_row(row(1, &a, 4)).is_err());
        let (pause, paused) = command(pause, 1);
        queue.push_control(pause).unwrap();
        assert!(queue.try_push_row(row(1, &a, 5)).is_err());
        let (stop, stopped) = command(stop, 1);
        queue.push_control(stop).unwrap();
        assert!(queue.try_push_row(row(1, &a, 6)).is_err());
        let (_events, handle) = run_thread(&fs, &queue);

        assert_eq!(wait(&started), Ok(()));
        assert_eq!(wait(&paused), Ok(()));
        assert_eq!(wait(&stopped), Ok(()));
        assert_eq!(
            fs.flushed(&base()),
            Some(concat(&[
                &head(&a),
                &line(&a, 1),
                &line(&a, 2),
                &line(&a, 3)
            ]))
        );
        queue.close();
        handle.join().unwrap();
    }

    #[test]
    fn layout_change_with_backlogged_rows() {
        let fs = MemFs::new();
        let queue = LogQueue::new(MAX_ROWS, MAX_QUEUE_BYTES);
        let a = layout_a();
        let b = layout_b();
        let (start, started) = start_control(1, &a, BIG);
        queue.push_control(start).unwrap();
        for (n, l) in [(1, &a), (2, &a), (3, &b), (4, &b), (5, &a)] {
            queue.try_push_row(row(1, l, n)).unwrap();
        }
        let (stop, stopped) = command(stop, 1);
        queue.push_control(stop).unwrap();
        let (_events, handle) = run_thread(&fs, &queue);
        assert_eq!(wait(&started), Ok(()));
        assert_eq!(wait(&stopped), Ok(()));

        let part = |k: u32| path(&format!("{STEM}-part{k}.csv"));
        assert_eq!(
            fs.content(&base()),
            Some(concat(&[&head(&a), &line(&a, 1), &line(&a, 2)]))
        );
        assert_eq!(
            fs.content(&part(2)),
            Some(concat(&[&head(&b), &line(&b, 3), &line(&b, 4)]))
        );
        assert_eq!(
            fs.content(&part(3)),
            Some(concat(&[&head(&a), &line(&a, 5)]))
        );
        queue.close();
        handle.join().unwrap();
    }

    #[test]
    fn closing_the_queue_flushes_and_ends_the_thread() {
        let fs = MemFs::new();
        let queue = LogQueue::new(MAX_ROWS, MAX_QUEUE_BYTES);
        let a = layout_a();
        let (start, started) = start_control(1, &a, BIG);
        queue.push_control(start).unwrap();
        queue.try_push_row(row(1, &a, 1)).unwrap();
        queue.close();
        let (_events, handle) = run_thread(&fs, &queue);
        assert_eq!(wait(&started), Ok(()));
        handle.join().unwrap();
        assert_eq!(
            fs.flushed(&base()),
            Some(concat(&[&head(&a), &line(&a, 1)]))
        );
    }

    #[test]
    fn a_dropped_reply_receiver_is_not_an_error() {
        let mut h = Harness::new();
        let a = layout_a();
        let (start, rx) = start_control(1, &a, BIG);
        drop(rx);
        h.writer.handle(Item::Control(start), h.at(0.0));
        h.row(row(1, &a, 1), 1.0);
        assert_eq!(h.control(command(stop, 1), 2.0), Ok(()));
        assert_eq!(
            h.fs.flushed(&base()),
            Some(concat(&[&head(&a), &line(&a, 1)]))
        );
    }

    #[test]
    fn failure_keys() {
        let key = |code: i32| WriteFailure::from_io(&io::Error::from_raw_os_error(code));
        assert_eq!(key(39), WriteFailure::DiskFull);
        assert_eq!(key(112), WriteFailure::DiskFull);
        for code in [3, 15, 21, 55, 433, 1006, 1117, 1167] {
            assert_eq!(key(code), WriteFailure::Unavailable, "code {code}");
        }
        assert_eq!(key(5), WriteFailure::Denied);
        let other = io::Error::other("boom");
        assert_eq!(
            WriteFailure::from_io(&other),
            WriteFailure::Other("boom".into())
        );
        assert!(matches!(key(87), WriteFailure::Other(_)));

        let cases = [
            (WriteFailure::DiskFull, "log.error.diskFull"),
            (WriteFailure::Unavailable, "log.error.unavailable"),
            (WriteFailure::Denied, "log.error.denied"),
            (WriteFailure::HeaderTooLarge, "log.error.headerTooLarge"),
            (WriteFailure::TooManyCollisions, "log.error.other"),
            (WriteFailure::Other("x".into()), "log.error.other"),
        ];
        for (failure, expected) in cases {
            assert_eq!(failure.key(), expected, "{failure:?}");
        }
        assert_eq!(
            WriteFailure::Other("boom".into()).detail().as_deref(),
            Some("boom")
        );
        assert!(WriteFailure::TooManyCollisions.detail().is_some());
        assert_eq!(WriteFailure::DiskFull.detail(), None);
    }
}
