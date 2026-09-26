//! Named pipe client: overlapped handle, length-prefixed frames, stop event.
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_NO_DATA,
    ERROR_PIPE_BUSY, ERROR_PIPE_NOT_CONNECTED, ERROR_SEM_TIMEOUT, GENERIC_READ, GENERIC_WRITE,
    GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT, WIN32_ERROR,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, OPEN_EXISTING,
    ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::{GetNamedPipeServerProcessId, WaitNamedPipeW};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects};
use windows::core::HSTRING;

pub const MAX_FRAME: u32 = 4 * 1024 * 1024;
/// FILE_GENERIC_READ | FILE_WRITE_DATA: what the client really needs (no FILE_APPEND_DATA,
/// which on a pipe means FILE_CREATE_PIPE_INSTANCE).
pub const CLIENT_ACCESS_MIN: u32 = 0x0012_0089 | 0x0000_0002;
pub const CLIENT_ACCESS_GENERIC: u32 = GENERIC_READ.0 | GENERIC_WRITE.0;

#[derive(Debug, PartialEq)]
pub enum PipeError {
    /// No pipe with that name (server not running).
    NotFound,
    /// All instances busy and WaitNamedPipeW timed out.
    Busy,
    /// Server closed or disconnected the pipe.
    Disconnected(WIN32_ERROR),
    /// Stop event signalled.
    Stopped,
    /// Write did not complete in time.
    TimedOut,
    FrameTooLarge(u32),
    Win32(WIN32_ERROR),
}

fn code(e: &windows::core::Error) -> WIN32_ERROR {
    WIN32_ERROR::from_error(e).unwrap_or(WIN32_ERROR(e.code().0 as u32))
}

fn map_io(c: WIN32_ERROR) -> PipeError {
    match c {
        ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_NO_DATA => PipeError::Disconnected(c),
        other => PipeError::Win32(other),
    }
}

pub struct OwnedHandle(pub HANDLE);
// SAFETY: a kernel handle value may be used from any thread; the owner closes it exactly once.
unsafe impl Send for OwnedHandle {}
// SAFETY: as above; every API used on it (ReadFile/WriteFile with distinct OVERLAPPEDs,
// CancelIoEx, SetEvent, WaitForMultipleObjects) is documented thread-safe.
unsafe impl Sync for OwnedHandle {}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: we own the handle and nobody uses it after drop.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

pub fn new_event(manual_reset: bool) -> OwnedHandle {
    // SAFETY: plain unnamed event, no security attributes.
    OwnedHandle(unsafe { CreateEventW(None, manual_reset, false, None) }.expect("CreateEventW"))
}

pub fn signal(ev: &OwnedHandle) {
    // SAFETY: ev is a live event handle.
    unsafe { SetEvent(ev.0) }.expect("SetEvent");
}

/// Opens `\\.\pipe\<name>`. Retries on ERROR_PIPE_BUSY via WaitNamedPipeW until `busy_wait`.
pub fn open(
    name: &str,
    access: u32,
    overlapped: bool,
    busy_wait: Duration,
) -> Result<OwnedHandle, PipeError> {
    let path = HSTRING::from(format!(r"\\.\pipe\{name}"));
    let deadline = Instant::now() + busy_wait;
    let mut flags = SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION;
    if overlapped {
        flags |= FILE_FLAG_OVERLAPPED;
    }
    loop {
        // SAFETY: path is a valid NUL-terminated wide string for the call's duration.
        let r = unsafe {
            CreateFileW(&path, access, FILE_SHARE_MODE(0), None, OPEN_EXISTING, flags, None)
        };
        match r {
            Ok(h) => return Ok(OwnedHandle(h)),
            Err(e) => match code(&e) {
                ERROR_FILE_NOT_FOUND => return Err(PipeError::NotFound),
                ERROR_PIPE_BUSY => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(PipeError::Busy);
                    }
                    // SAFETY: as above.
                    let ok = unsafe { WaitNamedPipeW(&path, left.as_millis().max(1) as u32) };
                    if !ok.as_bool() {
                        // SAFETY: reads the thread-local last error right after the failing call.
                        match unsafe { GetLastError() } {
                            ERROR_SEM_TIMEOUT => return Err(PipeError::Busy),
                            ERROR_FILE_NOT_FOUND => return Err(PipeError::NotFound),
                            other => return Err(PipeError::Win32(other)),
                        }
                    }
                }
                other => return Err(PipeError::Win32(other)),
            },
        }
    }
}

pub fn wait_named_pipe_raw(name: &str, ms: u32) -> Result<(), WIN32_ERROR> {
    let path = HSTRING::from(format!(r"\\.\pipe\{name}"));
    // SAFETY: valid wide string.
    let ok = unsafe { WaitNamedPipeW(&path, ms) };
    if ok.as_bool() {
        Ok(())
    } else {
        // SAFETY: immediately after the failing call.
        Err(unsafe { GetLastError() })
    }
}

pub fn server_pid(h: &OwnedHandle) -> Result<u32, WIN32_ERROR> {
    let mut pid = 0u32;
    // SAFETY: h is a live client pipe handle, pid a valid out pointer.
    unsafe { GetNamedPipeServerProcessId(h.0, &mut pid) }.map_err(|e| code(&e))?;
    Ok(pid)
}

/// One overlapped operation, waited on together with an optional stop event.
/// On stop/timeout the I/O is cancelled and *waited for* (the kernel owns `ov` and the
/// buffer until GetOverlappedResult(bWait=TRUE) returns).
fn ov_io(
    h: HANDLE,
    io_event: &OwnedHandle,
    stop: Option<&OwnedHandle>,
    timeout_ms: u32,
    op: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>,
) -> Result<u32, PipeError> {
    let mut ov = OVERLAPPED { hEvent: io_event.0, ..Default::default() };
    if let Err(e) = op(&mut ov) {
        let c = code(&e);
        if c != ERROR_IO_PENDING {
            return Err(map_io(c));
        }
        let mut handles = vec![io_event.0];
        if let Some(s) = stop {
            handles.push(s.0);
        }
        // SAFETY: all handles are live events.
        let w = unsafe { WaitForMultipleObjects(&handles, false, timeout_ms) };
        if w != WAIT_OBJECT_0 {
            let mut n = 0u32;
            // SAFETY: cancels only this OVERLAPPED; then wait for the kernel to release it.
            unsafe {
                let _ = CancelIoEx(h, Some(&ov));
                let _ = GetOverlappedResult(h, &ov, &mut n, true);
            }
            return Err(if w == WAIT_TIMEOUT { PipeError::TimedOut } else { PipeError::Stopped });
        }
    }
    let mut n = 0u32;
    // SAFETY: the operation has completed (event signalled or synchronous success).
    unsafe { GetOverlappedResult(h, &ov, &mut n, false) }.map_err(|e| map_io(code(&e)))?;
    Ok(n)
}

pub struct Conn {
    pub h: OwnedHandle,
    read_ev: OwnedHandle,
    write_ev: OwnedHandle,
}

impl Conn {
    pub fn new(h: OwnedHandle) -> Self {
        Self { h, read_ev: new_event(true), write_ev: new_event(true) }
    }

    fn read_exact(&self, buf: &mut [u8], stop: &OwnedHandle) -> Result<(), PipeError> {
        let mut off = 0;
        while off < buf.len() {
            let chunk = &mut buf[off..];
            let n = ov_io(self.h.0, &self.read_ev, Some(stop), INFINITE, |ov| {
                // SAFETY: chunk and ov outlive the I/O (ov_io waits for completion or cancellation).
                unsafe { ReadFile(self.h.0, Some(chunk), None, Some(ov)) }
            })?;
            off += n as usize;
        }
        Ok(())
    }

    /// Blocks (without spinning) until a whole frame arrives, the peer goes away or `stop` fires.
    pub fn read_frame(&self, stop: &OwnedHandle) -> Result<Vec<u8>, PipeError> {
        let mut hdr = [0u8; 4];
        self.read_exact(&mut hdr, stop)?;
        let len = u32::from_le_bytes(hdr);
        if len > MAX_FRAME {
            return Err(PipeError::FrameTooLarge(len));
        }
        let mut body = vec![0u8; len as usize];
        self.read_exact(&mut body, stop)?;
        Ok(body)
    }

    /// Safe to call from another thread while `read_frame` is pending (overlapped handle).
    pub fn write_frame(&self, payload: &[u8], timeout_ms: u32) -> Result<(), PipeError> {
        let mut buf = Vec::with_capacity(4 + payload.len());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(payload);
        let mut off = 0;
        while off < buf.len() {
            let chunk = &buf[off..];
            let n = ov_io(self.h.0, &self.write_ev, None, timeout_ms, |ov| {
                // SAFETY: chunk and ov outlive the I/O.
                unsafe { WriteFile(self.h.0, Some(chunk), None, Some(ov)) }
            })?;
            off += n as usize;
        }
        Ok(())
    }
}

pub fn flags_none() -> FILE_FLAGS_AND_ATTRIBUTES {
    FILE_FLAGS_AND_ATTRIBUTES(0)
}
