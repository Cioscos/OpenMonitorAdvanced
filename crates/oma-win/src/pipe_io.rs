//! Overlapped named-pipe I/O shared by the sensor pipe client (`svc::pipe`)
//! and the private overlay pipe (`overlay_pipe`), spec §6 and spike S3 §2.
//!
//! The handle is opened with `FILE_FLAG_OVERLAPPED`, so a reader thread can
//! wait on its pending `ReadFile` together with a stop event while another
//! thread writes. Every overlapped operation that is abandoned (stop, write
//! timeout, accept timeout) is cancelled with `CancelIoEx` and then waited
//! for with `GetOverlappedResult(bWait = TRUE)` before its buffer, event and
//! `OVERLAPPED` are released: until then the kernel still owns them.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oma_ipc::{FrameDecoder, IpcError};
use serde::de::DeserializeOwned;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_BUSY, ERROR_IO_PENDING, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::Threading::{
    CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use crate::svc::win32_code;

/// How long a send waits for the other end to take a frame.
pub(crate) const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// Size of one overlapped read (the `FrameDecoder` contract asks for at most 64 KiB).
const READ_CHUNK: usize = 64 * 1024;

/// Why the reader stopped: the payload of the one [`PipeEvent::Closed`].
#[derive(Debug)]
pub enum CloseReason {
    /// The other end closed or disconnected the pipe (109, 232, 233).
    Disconnected,
    /// The byte stream broke the protocol: an oversized or undecodable frame,
    /// or a frame left truncated when the other end went away.
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

/// What a reader thread delivers to its consumer.
#[derive(Debug)]
pub enum PipeEvent<T> {
    Message(T),
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
pub(crate) fn new_event() -> Result<OwnedHandle, u32> {
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
pub(crate) enum IoFailure {
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
pub(crate) unsafe fn overlapped_io(
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

/// One connected end of a byte-mode pipe opened for overlapped I/O: a
/// serialised framed writer and at most one reader thread.
pub(crate) struct PipeConn {
    pipe: OwnedHandle,
    /// Names the other end in logs and errors ("sensor service", "overlay").
    peer: &'static str,
    /// Set once the connection is unusable: the reader gave up on it, or a
    /// write failed or timed out part-way through a frame.
    broken: AtomicBool,
    /// Event of the write `OVERLAPPED`; the mutex also serialises `send_frame`.
    write_event: Mutex<OwnedHandle>,
    reader_started: AtomicBool,
}

impl PipeConn {
    /// Takes ownership of `pipe`, a pipe handle opened with `FILE_FLAG_OVERLAPPED`.
    pub(crate) fn new(pipe: OwnedHandle, peer: &'static str) -> Result<Arc<Self>, u32> {
        let write_event = new_event()?;
        Ok(Arc::new(Self {
            pipe,
            peer,
            broken: AtomicBool::new(false),
            write_event: Mutex::new(write_event),
            reader_started: AtomicBool::new(false),
        }))
    }

    pub(crate) fn handle(&self) -> HANDLE {
        self.pipe.0
    }

    /// Writes one encoded frame, waiting at most [`WRITE_TIMEOUT`] for the
    /// other end to take it. Concurrent calls are serialised. Any failure
    /// leaves the connection unusable (a frame may be half written): later
    /// calls fail with `BrokenPipe`.
    pub(crate) fn send_frame(&self, frame: &[u8]) -> io::Result<()> {
        let event = self
            .write_event
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.broken.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("the connection to the {} is closed", self.peer),
            ));
        }
        let h = self.pipe.0;
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
                    self.broken.store(true, Ordering::Release);
                    return Err(match failure {
                        IoFailure::Win32(code) => io::Error::from_raw_os_error(code as i32),
                        IoFailure::TimedOut | IoFailure::Stopped => io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!("the {} did not read the frame within 2 s", self.peer),
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    /// Starts the reader thread, which decodes frames of `T` and gives each
    /// to `deliver` until the connection closes or [`PipeReader::stop`].
    ///
    /// `deliver` must not block for good: it returns `false` when nobody can
    /// take the event (the consumer is gone or overwhelmed), and the reader
    /// then closes the connection without a `Closed` event. A `deliver` that
    /// waits for its consumer must stop waiting before [`PipeReader::stop`],
    /// which joins this thread. `deliver` is dropped when the reader exits.
    ///
    /// One reader per connection: a second call only reports `Closed(Io(170))`.
    pub(crate) fn start_reader<T: DeserializeOwned + Send + 'static>(
        self: &Arc<Self>,
        deliver: impl Fn(PipeEvent<T>) -> bool + Send + 'static,
    ) -> PipeReader {
        let inert = PipeReader {
            stop: None,
            thread: None,
        };
        if self.reader_started.swap(true, Ordering::AcqRel) {
            let _ = deliver(PipeEvent::Closed(CloseReason::Io(ERROR_BUSY.0)));
            return inert;
        }
        let (stop, read_event) = match (new_event(), new_event()) {
            (Ok(stop), Ok(read_event)) => (Arc::new(stop), read_event),
            (Err(code), _) | (_, Err(code)) => {
                self.broken.store(true, Ordering::Release);
                let _ = deliver(PipeEvent::Closed(CloseReason::Io(code)));
                return inert;
            }
        };
        let conn = Arc::clone(self);
        let thread_stop = Arc::clone(&stop);
        let spawned = std::thread::Builder::new()
            .name("oma-pipe-reader".to_owned())
            .spawn(move || run_reader(&conn, &thread_stop, &read_event, &deliver));
        match spawned {
            Ok(thread) => PipeReader {
                stop: Some(stop),
                thread: Some(thread),
            },
            Err(e) => {
                // The closure, and with it `deliver`, is gone: a receiver behind it sees Disconnected.
                tracing::warn!("cannot start the {} pipe reader: {e}", self.peer);
                self.broken.store(true, Ordering::Release);
                inert
            }
        }
    }
}

/// The reader thread's body: reads until the connection ends, then reports
/// the reason (if the consumer can take it) and drops `deliver`.
fn run_reader<T: DeserializeOwned>(
    conn: &PipeConn,
    stop: &OwnedHandle,
    read_event: &OwnedHandle,
    deliver: &impl Fn(PipeEvent<T>) -> bool,
) {
    let reason = read_until_closed(conn.pipe.0, stop, read_event, deliver);
    if !matches!(reason, Some(CloseReason::Stopped)) {
        conn.broken.store(true, Ordering::Release);
    }
    if let Some(reason) = reason {
        let _ = deliver(PipeEvent::Closed(reason));
    }
}

/// `None` means the consumer is full or gone: nobody can be told.
fn read_until_closed<T: DeserializeOwned>(
    h: HANDLE,
    stop: &OwnedHandle,
    read_event: &OwnedHandle,
    deliver: &impl Fn(PipeEvent<T>) -> bool,
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
            match decoder.next_of::<T>() {
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

/// The running reader thread of a pipe connection. Dropping it stops it too.
pub struct PipeReader {
    stop: Option<Arc<OwnedHandle>>,
    thread: Option<JoinHandle<()>>,
}

impl PipeReader {
    /// Signals the stop event and joins the thread. The thread cancels its
    /// pending read (`CancelIoEx` on its own `OVERLAPPED`), waits for the
    /// cancellation with `GetOverlappedResult(TRUE)`, reports
    /// `Closed(Stopped)` if the consumer has room, and exits.
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
