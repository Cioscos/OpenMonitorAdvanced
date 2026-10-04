//! Overlapped client for the sensor named pipe (spec §6, spike S3 §2).
//!
//! The handle is opened with `FILE_FLAG_OVERLAPPED`, so a reader thread can
//! wait on its pending `ReadFile` together with a stop event while another
//! thread writes. Every overlapped operation that is abandoned (stop, write
//! timeout) is cancelled with `CancelIoEx` and then waited for with
//! `GetOverlappedResult(bWait = TRUE)` before its buffer, event and
//! `OVERLAPPED` are released: until then the kernel still owns them.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oma_ipc::{encode_frame, FrameDecoder, IpcError, Message};
use windows::core::HSTRING;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_BUSY, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_MODE, OPEN_EXISTING,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows::Win32::System::Threading::{
    CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use super::win32_code;

/// How long [`PipeClient::send`] waits for the server to take a frame.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// Size of one overlapped read (the `FrameDecoder` contract asks for at most 64 KiB).
const READ_CHUNK: usize = 64 * 1024;

/// Why [`PipeClient::connect`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectError {
    /// No pipe with that name: the service is not running (2).
    NotFound,
    /// Every instance is taken (231). The caller retries on its own schedule;
    /// `connect` never waits with `WaitNamedPipeW`.
    Busy,
    /// The pipe's DACL refuses this user (5).
    AccessDenied,
    /// Any other Win32 error.
    Other(u32),
}

impl ConnectError {
    /// Maps a `CreateFileW` error code.
    pub fn from_code(code: u32) -> Self {
        match code {
            2 => Self::NotFound,
            231 => Self::Busy,
            5 => Self::AccessDenied,
            other => Self::Other(other),
        }
    }
}

/// Why the reader stopped: the payload of the one [`PipeEvent::Closed`].
#[derive(Debug)]
pub enum CloseReason {
    /// The server closed or disconnected the pipe (109, 232, 233).
    Disconnected,
    /// The byte stream broke the protocol: an oversized or undecodable frame,
    /// or a frame left truncated when the server went away.
    Protocol(IpcError),
    /// Any other Win32 error from the read.
    Io(u32),
    /// [`PipeReader::stop`] was called.
    Stopped,
}

/// Maps the error code of a failed read.
pub(crate) fn close_reason_for(code: u32) -> CloseReason {
    match code {
        // ERROR_BROKEN_PIPE, ERROR_BAD_PIPE, ERROR_PIPE_NOT_CONNECTED
        109 | 232 | 233 => CloseReason::Disconnected,
        other => CloseReason::Io(other),
    }
}

/// What the reader thread delivers on the caller's channel.
#[derive(Debug)]
pub enum PipeEvent {
    Message(Message),
    /// The last event of a reader, sent at most once.
    Closed(CloseReason),
}

/// A kernel handle closed on drop.
pub(crate) struct OwnedHandle(pub(crate) HANDLE);

// SAFETY: a kernel handle value may be used from any thread; the owner closes it exactly once.
unsafe impl Send for OwnedHandle {}
// SAFETY: every API used on a shared handle here (overlapped ReadFile/WriteFile with distinct
// OVERLAPPEDs, CancelIoEx, SetEvent, WaitForMultipleObjects) is documented thread-safe.
unsafe impl Sync for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: this value is the sole owner of the handle, never used after drop.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// An unnamed manual-reset event, initially not signalled.
fn new_event() -> Result<OwnedHandle, u32> {
    // SAFETY: no security attributes and no name; the handle is owned by the result.
    unsafe { CreateEventW(None, true, false, None) }
        .map(OwnedHandle)
        .map_err(|e| win32_code(&e))
}

fn is_signalled(event: &OwnedHandle) -> bool {
    // SAFETY: `event` is a live event handle; a zero timeout only polls it.
    unsafe { WaitForSingleObject(event.0, 0) == WAIT_OBJECT_0 }
}

/// Why an overlapped operation did not complete.
enum IoFailure {
    Win32(u32),
    Stopped,
    TimedOut,
}

/// Starts one overlapped operation with `op` and waits for it, for `stop`
/// or for `timeout_ms`. When it gives up (stop, timeout, failed wait) the
/// operation is cancelled and waited for before returning, so on return
/// the kernel no longer uses the `OVERLAPPED`, `event` or the buffer.
///
/// # Safety
///
/// `op` must start at most one overlapped I/O, on `h`, with the
/// `OVERLAPPED` it receives, and its buffer must outlive this call.
/// `event` (manual reset) and `stop` must be live event handles.
unsafe fn overlapped_io(
    h: HANDLE,
    event: HANDLE,
    stop: Option<HANDLE>,
    timeout_ms: u32,
    op: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>,
) -> Result<u32, IoFailure> {
    let mut ov = OVERLAPPED {
        hEvent: event,
        ..Default::default()
    };
    if let Err(e) = op(&mut ov) {
        let code = win32_code(&e);
        if code != ERROR_IO_PENDING.0 {
            // Failed synchronously: nothing is pending on `ov`.
            return Err(IoFailure::Win32(code));
        }
        let waits = [event, stop.unwrap_or_default()];
        let waits = if stop.is_some() {
            &waits[..]
        } else {
            &waits[..1]
        };
        // SAFETY: every handle in `waits` is a live event (caller contract).
        let w = unsafe { WaitForMultipleObjects(waits, false, timeout_ms) };
        if w != WAIT_OBJECT_0 {
            let failure = if w == WAIT_TIMEOUT {
                IoFailure::TimedOut
            } else if stop.is_some() && w.0 == WAIT_OBJECT_0.0 + 1 {
                IoFailure::Stopped
            } else {
                // SAFETY: reads this thread's last error right after the failed wait.
                IoFailure::Win32(unsafe { GetLastError() }.0)
            };
            let mut n = 0u32;
            // SAFETY: cancels only this OVERLAPPED on `h`, then blocks until the kernel has
            // finished with it (completed or aborted), so `ov` and the buffer can be released.
            unsafe {
                let _ = CancelIoEx(h, Some(&ov));
                let _ = GetOverlappedResult(h, &ov, &mut n, true);
            }
            return Err(failure);
        }
    }
    let mut n = 0u32;
    // SAFETY: the operation has completed (synchronously, or its event is signalled).
    unsafe { GetOverlappedResult(h, &ov, &mut n, false) }
        .map_err(|e| IoFailure::Win32(win32_code(&e)))?;
    Ok(n)
}

struct Shared {
    pipe: OwnedHandle,
    /// Set once the connection is unusable: the reader gave up on it, or a
    /// write failed or timed out part-way through a frame.
    broken: AtomicBool,
    /// Event of the write `OVERLAPPED`; the mutex also serialises `send`.
    write_event: Mutex<OwnedHandle>,
    reader_started: AtomicBool,
}

/// A connection to the sensor pipe.
pub struct PipeClient {
    shared: Arc<Shared>,
}

impl PipeClient {
    /// Opens `\\.\pipe\<pipe_name>` for overlapped reads and writes, with an
    /// identification-level impersonation token for the server. Never waits
    /// for a busy pipe.
    pub fn connect(pipe_name: &str) -> Result<PipeClient, ConnectError> {
        let path = HSTRING::from(format!(r"\\.\pipe\{pipe_name}"));
        // SAFETY: `path` is a valid NUL-terminated wide string for the call; the returned
        // handle is owned by an `OwnedHandle` right away.
        let h = unsafe {
            CreateFileW(
                &path,
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        }
        .map_err(|e| ConnectError::from_code(win32_code(&e)))?;
        let pipe = OwnedHandle(h);
        let write_event = new_event().map_err(ConnectError::Other)?;
        Ok(PipeClient {
            shared: Arc::new(Shared {
                pipe,
                broken: AtomicBool::new(false),
                write_event: Mutex::new(write_event),
                reader_started: AtomicBool::new(false),
            }),
        })
    }

    /// PID of the process serving the other end (for the service identity check).
    pub fn server_pid(&self) -> Option<u32> {
        let mut pid = 0u32;
        // SAFETY: live client pipe handle and a valid out pointer.
        unsafe { GetNamedPipeServerProcessId(self.shared.pipe.0, &mut pid) }.ok()?;
        Some(pid)
    }

    /// Writes one frame, waiting at most 2 s for the server to take it.
    /// Concurrent calls are serialised. Any failure after the connection
    /// was opened leaves it unusable (a frame may be half written): later
    /// calls fail with `BrokenPipe`, and the caller should drop the client.
    pub fn send(&self, msg: &Message) -> io::Result<()> {
        let frame = encode_frame(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let event = self
            .shared
            .write_event
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.shared.broken.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the sensor pipe connection is closed",
            ));
        }
        let h = self.shared.pipe.0;
        let deadline = Instant::now() + WRITE_TIMEOUT;
        let mut off = 0;
        while off < frame.len() {
            let left = deadline.saturating_duration_since(Instant::now());
            let chunk = &frame[off..];
            // SAFETY: one overlapped WriteFile on `h` with the given OVERLAPPED; `chunk` lives in
            // `frame`, which outlives the call, and `overlapped_io` waits for the write (or its
            // cancellation) to finish. The write event is live and ours while the lock is held.
            let r = unsafe {
                overlapped_io(h, event.0, None, left.as_millis() as u32, |ov| {
                    WriteFile(h, Some(chunk), None, Some(ov))
                })
            };
            match r {
                Ok(n) => off += n as usize,
                Err(failure) => {
                    self.shared.broken.store(true, Ordering::Release);
                    return Err(match failure {
                        IoFailure::Win32(code) => io::Error::from_raw_os_error(code as i32),
                        IoFailure::TimedOut | IoFailure::Stopped => io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the sensor service did not read the frame within 2 s",
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Starts the reader thread, which decodes frames and delivers them on
    /// `events` until the connection closes or [`PipeReader::stop`].
    ///
    /// `events` is meant to be a `sync_channel(8)`. The reader never blocks
    /// on it: when it is full, the reader closes the connection (overflow is
    /// a close, not an unbounded backlog) without a `Closed` event. Whatever
    /// the reason, the reader drops its sender when it exits, so the
    /// receiver always ends with `Disconnected`, after at most one `Closed`.
    ///
    /// One reader per connection: a second call only reports `Closed(Io(170))`.
    pub fn start_reader(&self, events: SyncSender<PipeEvent>) -> PipeReader {
        self.start_reader_with(move |event| match events.try_send(event) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                tracing::warn!("sensor pipe consumer is not keeping up; closing");
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        })
    }

    /// Like [`start_reader`](Self::start_reader), but each event goes to
    /// `deliver`, which must not block for good: it returns `false` when
    /// nobody can take the event (the consumer is gone or overwhelmed), and
    /// the reader then closes the connection without a `Closed` event. A
    /// `deliver` that waits for its consumer must stop waiting before
    /// [`PipeReader::stop`], which joins this thread. `deliver` is dropped
    /// when the reader exits.
    pub fn start_reader_with(
        &self,
        deliver: impl Fn(PipeEvent) -> bool + Send + 'static,
    ) -> PipeReader {
        let inert = PipeReader {
            stop: None,
            thread: None,
        };
        if self.shared.reader_started.swap(true, Ordering::AcqRel) {
            let _ = deliver(PipeEvent::Closed(CloseReason::Io(ERROR_BUSY.0)));
            return inert;
        }
        let (stop, read_event) = match (new_event(), new_event()) {
            (Ok(stop), Ok(read_event)) => (Arc::new(stop), read_event),
            (Err(code), _) | (_, Err(code)) => {
                self.shared.broken.store(true, Ordering::Release);
                let _ = deliver(PipeEvent::Closed(CloseReason::Io(code)));
                return inert;
            }
        };
        let shared = Arc::clone(&self.shared);
        let thread_stop = Arc::clone(&stop);
        let spawned = std::thread::Builder::new()
            .name("oma-pipe-reader".to_owned())
            .spawn(move || run_reader(&shared, &thread_stop, &read_event, &deliver));
        match spawned {
            Ok(thread) => PipeReader {
                stop: Some(stop),
                thread: Some(thread),
            },
            Err(e) => {
                // The closure, and with it `deliver`, is gone: a receiver behind it sees Disconnected.
                tracing::warn!("cannot start the sensor pipe reader: {e}");
                self.shared.broken.store(true, Ordering::Release);
                inert
            }
        }
    }
}

/// The reader thread's body: reads until the connection ends, then reports
/// the reason (if the channel can take it) and drops the sender.
fn run_reader(
    shared: &Shared,
    stop: &OwnedHandle,
    read_event: &OwnedHandle,
    deliver: &impl Fn(PipeEvent) -> bool,
) {
    let reason = read_until_closed(shared.pipe.0, stop, read_event, deliver);
    if !matches!(reason, Some(CloseReason::Stopped)) {
        shared.broken.store(true, Ordering::Release);
    }
    if let Some(reason) = reason {
        let _ = deliver(PipeEvent::Closed(reason));
    }
}

/// `None` means the consumer is full or gone: nobody can be told.
fn read_until_closed(
    h: HANDLE,
    stop: &OwnedHandle,
    read_event: &OwnedHandle,
    deliver: &impl Fn(PipeEvent) -> bool,
) -> Option<CloseReason> {
    let mut buf = vec![0u8; READ_CHUNK];
    let mut decoder = FrameDecoder::new();
    loop {
        // Reads that complete synchronously never wait on `stop`: check it here too.
        if is_signalled(stop) {
            return Some(CloseReason::Stopped);
        }
        // SAFETY: one overlapped ReadFile on `h` with the given OVERLAPPED; `buf` outlives the
        // call and `overlapped_io` waits for the read (or its cancellation) to finish. Both
        // events are live for the whole thread.
        let r = unsafe {
            overlapped_io(h, read_event.0, Some(stop.0), INFINITE, |ov| {
                ReadFile(h, Some(&mut buf), None, Some(ov))
            })
        };
        let n = match r {
            Ok(n) => n as usize,
            Err(IoFailure::Stopped) => return Some(CloseReason::Stopped),
            Err(IoFailure::TimedOut) => return Some(CloseReason::Io(WAIT_TIMEOUT.0)),
            Err(IoFailure::Win32(code)) => {
                return Some(match close_reason_for(code) {
                    // End of stream: a frame left half read is a protocol error (ruling R3).
                    CloseReason::Disconnected => match decoder.finish() {
                        Ok(()) => CloseReason::Disconnected,
                        Err(e) => CloseReason::Protocol(e),
                    },
                    other => other,
                });
            }
        };
        if let Err(e) = decoder.push(&buf[..n]) {
            return Some(CloseReason::Protocol(e));
        }
        // Drain to Ok(None) after every push (ruling R8).
        loop {
            match decoder.next_message() {
                Ok(Some(msg)) => {
                    if !deliver(PipeEvent::Message(msg)) {
                        return None;
                    }
                }
                Ok(None) => break,
                Err(e) => return Some(CloseReason::Protocol(e)),
            }
        }
    }
}

/// The running reader thread of a [`PipeClient`]. Dropping it stops it too.
pub struct PipeReader {
    stop: Option<Arc<OwnedHandle>>,
    thread: Option<JoinHandle<()>>,
}

impl PipeReader {
    /// Signals the stop event and joins the thread. The thread cancels its
    /// pending read (`CancelIoEx` on its own `OVERLAPPED`), waits for the
    /// cancellation with `GetOverlappedResult(TRUE)`, reports
    /// `Closed(Stopped)` if the channel has room, and exits.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(stop) = self.stop.take() {
            // SAFETY: `stop` is a live event handle, kept alive by this Arc.
            unsafe {
                let _ = SetEvent(stop.0);
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError};
    use std::time::{Duration, Instant};

    use oma_ipc::{Hello, Subscribe, WireSnapshot, MAX_FRAME_BYTES};

    use super::*;
    use crate::svc::fake_server::{unique_name, FakeServer};

    const WAIT: Duration = Duration::from_secs(5);

    fn hello() -> Message {
        Message::Hello(Hello {
            protocol_version: 1,
            service_version: "test".to_owned(),
            pawn_io: "ok".to_owned(),
        })
    }

    fn connected() -> (FakeServer, PipeClient) {
        let server = FakeServer::new();
        let client = PipeClient::connect(&server.name).expect("connect to the fake server");
        server.accept();
        (server, client)
    }

    /// The reader has exited and dropped its sender: nothing else will arrive.
    fn assert_channel_closed(rx: &Receiver<PipeEvent>) {
        match rx.recv_timeout(WAIT) {
            Err(RecvTimeoutError::Disconnected) => {}
            other => panic!("expected the channel to close, got {other:?}"),
        }
    }

    #[test]
    fn connect_error_codes_map_to_variants() {
        assert_eq!(ConnectError::from_code(2), ConnectError::NotFound);
        assert_eq!(ConnectError::from_code(231), ConnectError::Busy);
        assert_eq!(ConnectError::from_code(5), ConnectError::AccessDenied);
        assert_eq!(ConnectError::from_code(3), ConnectError::Other(3));
    }

    #[test]
    fn broken_pipe_codes_are_disconnects() {
        for code in [109, 232, 233] {
            assert!(matches!(close_reason_for(code), CloseReason::Disconnected));
        }
        assert!(matches!(close_reason_for(995), CloseReason::Io(995)));
    }

    #[test]
    fn connect_to_a_missing_pipe_is_not_found() {
        let name = format!("{}-missing", unique_name());
        assert_eq!(
            PipeClient::connect(&name).err(),
            Some(ConnectError::NotFound)
        );
    }

    #[test]
    fn round_trip_with_the_fake_server() {
        let (server, client) = connected();
        let (tx, rx) = sync_channel(8);
        let reader = client.start_reader(tx);

        server.send(&hello());
        match rx.recv_timeout(WAIT).expect("hello") {
            PipeEvent::Message(Message::Hello(h)) => assert_eq!(h.protocol_version, 1),
            other => panic!("expected Hello, got {other:?}"),
        }

        let subscribe = Message::Subscribe(Subscribe {
            interval_ms: 1000,
            disabled_modules: Vec::new(),
            smart_disabled_drives: Vec::new(),
            smart_enabled_drives: Vec::new(),
        });
        client.send(&subscribe).expect("send Subscribe");
        assert_eq!(server.recv(), subscribe);

        reader.stop();
    }

    #[test]
    fn server_pid_is_this_process() {
        let (_server, client) = connected();
        assert_eq!(client.server_pid(), Some(std::process::id()));
    }

    #[test]
    fn server_disconnect_reports_closed() {
        let (server, client) = connected();
        let (tx, rx) = sync_channel(8);
        let reader = client.start_reader(tx);
        server.send(&hello());
        assert!(matches!(
            rx.recv_timeout(WAIT),
            Ok(PipeEvent::Message(Message::Hello(_)))
        ));

        server.disconnect();
        match rx.recv_timeout(WAIT) {
            Ok(PipeEvent::Closed(CloseReason::Disconnected))
            | Ok(PipeEvent::Closed(CloseReason::Io(233))) => {}
            other => panic!("expected Closed(Disconnected), got {other:?}"),
        }
        assert_channel_closed(&rx);
        reader.stop();
    }

    #[test]
    fn oversized_frame_closes_with_a_protocol_error() {
        let (server, client) = connected();
        let (tx, rx) = sync_channel(8);
        let reader = client.start_reader(tx);

        let mut bytes = (MAX_FRAME_BYTES as u32 + 1).to_le_bytes().to_vec();
        bytes.extend_from_slice(&[0u8; 16]);
        server.send_raw(&bytes);

        match rx.recv_timeout(WAIT) {
            Ok(PipeEvent::Closed(CloseReason::Protocol(IpcError::FrameTooLarge(n)))) => {
                assert_eq!(n as usize, MAX_FRAME_BYTES + 1)
            }
            other => panic!("expected Closed(Protocol(FrameTooLarge)), got {other:?}"),
        }
        assert_channel_closed(&rx);
        reader.stop();
    }

    #[test]
    fn truncated_frame_at_disconnect_is_a_protocol_error() {
        let (server, client) = connected();
        let (tx, rx) = sync_channel(8);
        let reader = client.start_reader(tx);

        let frame = oma_ipc::encode_frame(&hello()).unwrap();
        server.send_raw(&frame[..frame.len() - 1]);
        // The disconnect would discard bytes the client has not read yet.
        server.flush();
        server.disconnect();

        match rx.recv_timeout(WAIT) {
            Ok(PipeEvent::Closed(CloseReason::Protocol(_))) => {}
            other => panic!("expected Closed(Protocol), got {other:?}"),
        }
        assert_channel_closed(&rx);
        reader.stop();
    }

    #[test]
    fn stopping_an_idle_reader_is_quick() {
        let (_server, client) = connected();
        let (tx, rx) = sync_channel(8);
        let reader = client.start_reader(tx);
        // Let the reader block in its overlapped read.
        std::thread::sleep(Duration::from_millis(50));

        let t = Instant::now();
        reader.stop();
        let took = t.elapsed();
        assert!(took < Duration::from_millis(200), "stop took {took:?}");

        match rx.recv_timeout(WAIT) {
            Ok(PipeEvent::Closed(CloseReason::Stopped)) => {}
            other => panic!("expected Closed(Stopped), got {other:?}"),
        }
        assert_channel_closed(&rx);
    }

    #[test]
    fn a_full_channel_closes_the_connection() {
        let (server, client) = connected();
        for _ in 0..12 {
            server.send(&hello());
        }
        let (tx, rx) = sync_channel(8);
        let reader = client.start_reader(tx);
        // Nobody drains the channel until the reader has given up.
        std::thread::sleep(Duration::from_millis(300));

        for i in 0..8 {
            match rx.recv_timeout(WAIT) {
                Ok(PipeEvent::Message(Message::Hello(_))) => {}
                other => panic!("event {i}: expected Hello, got {other:?}"),
            }
        }
        assert_channel_closed(&rx);
        let err = client
            .send(&Message::Subscribe(Subscribe {
                interval_ms: 1000,
                disabled_modules: Vec::new(),
                smart_disabled_drives: Vec::new(),
                smart_enabled_drives: Vec::new(),
            }))
            .expect_err("a closed connection refuses to send");
        assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
        reader.stop();
    }

    #[test]
    fn a_write_the_server_never_reads_times_out() {
        let (_server, client) = connected();
        // Far more than the 64 KiB pipe buffer, well under the 4 MiB frame limit.
        let big = Message::Snapshot(WireSnapshot {
            seq: 1,
            timestamp_ms: 0,
            values: vec![Some(1.5); 200_000],
            held: vec![false; 200_000],
        });

        let t = Instant::now();
        let err = client.send(&big).expect_err("the server never reads");
        let took = t.elapsed();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            took >= Duration::from_millis(1900) && took < Duration::from_secs(4),
            "send gave up after {took:?}"
        );
    }
}
