//! The private, single-client pipe shared by the overlay and load pipes.
//!
//! The server end is hardened so that only the process the app spawned can
//! be the client:
//!
//! - `FILE_FLAG_FIRST_PIPE_INSTANCE` with a single instance, so nobody can
//!   have created the name first and nobody can add a second instance;
//! - `PIPE_REJECT_REMOTE_CLIENTS`;
//! - a protected DACL that grants access to the current user's SID only
//!   (no inherited ACEs, no Administrators, no SYSTEM);
//! - [`PrivatePipeServer::accept`] returns the client's PID, which the
//!   caller compares with the PID of the process it spawned.
//!
//! Both ends speak one message type with the overlapped framing of
//! [`crate::pipe_io`]. Validating each decoded message is the receiver's job.

use std::io;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oma_ipc::encode_frame_of;
use serde::de::DeserializeOwned;
use serde::Serialize;
use windows::core::HSTRING;
use windows::Win32::Foundation::{
    GetLastError, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE,
    INVALID_HANDLE_VALUE,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    FILE_SHARE_MODE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};

use crate::overlay_pipe::{code_error, current_user_sid, os_error, user_only_sddl, LocalMem};
use crate::pipe_io::{new_event, overlapped_io, IoFailure, OwnedHandle, PipeConn};
pub use crate::pipe_io::{CloseReason, PipeEvent, PipeReader};

/// Size of each of the pipe's two buffers.
pub(crate) const BUFFER_BYTES: u32 = 64 * 1024;

/// Names the other end in logs and errors.
const PEER: &str = "private pipe peer";

/// The name starts with `prefix` and names nothing below it.
fn check_name(prefix: &str, name: &str) -> io::Result<()> {
    if name
        .strip_prefix(prefix)
        .is_some_and(|rest| !rest.is_empty() && !rest.contains(['\\', '/']))
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name:?} is not a pipe name under {prefix:?}"),
        ))
    }
}

/// The server end of a private pipe, owned by the app.
pub struct PrivatePipeServer {
    pub(crate) conn: Arc<PipeConn>,
}

impl PrivatePipeServer {
    /// Creates the only instance of the pipe `name` (which must start with
    /// `prefix`), with the descriptor of [`user_only_sddl`] for the current
    /// user. When the name exists it fails: with `ERROR_PIPE_BUSY` (231) if
    /// it is our own instance (the one-instance limit is checked first),
    /// with `ERROR_ACCESS_DENIED` (5) if another process created it
    /// (`FILE_FLAG_FIRST_PIPE_INSTANCE`).
    pub fn create(prefix: &'static str, name: &str) -> io::Result<Self> {
        check_name(prefix, name)?;
        let sddl = HSTRING::from(user_only_sddl(&current_user_sid()?));
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `sddl` is a valid NUL-terminated wide string for the call; `sd` receives a
        // LocalAlloc'd self-relative descriptor that the `LocalMem` guard frees.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &sddl,
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }
        .map_err(|e| os_error(&e))?;
        let sd = LocalMem(sd.0);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };
        let path = HSTRING::from(name);
        // SAFETY: `path` is a valid NUL-terminated wide string; `attributes` and the descriptor
        // it points to (kept alive by `sd`) outlive the call, which copies the descriptor into
        // the new pipe. The returned handle is checked and owned right away.
        let h = unsafe {
            CreateNamedPipeW(
                &path,
                FILE_FLAGS_AND_ATTRIBUTES(
                    PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_FIRST_PIPE_INSTANCE.0 | FILE_FLAG_OVERLAPPED.0,
                ),
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                BUFFER_BYTES,
                BUFFER_BYTES,
                0,
                Some(&attributes),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            // SAFETY: reads this thread's last error right after the failing call.
            return Err(code_error(unsafe { GetLastError() }.0));
        }
        let conn = PipeConn::new(OwnedHandle(h), PEER).map_err(code_error)?;
        Ok(Self { conn })
    }

    /// Waits at most `timeout` for a client to connect and returns its PID,
    /// which the caller checks against the process it spawned. Fails with
    /// `ErrorKind::TimedOut` when nobody connects in time; the pipe keeps
    /// listening and `accept` can be called again. A client that connected
    /// and left before it was accepted is dropped and the wait goes on.
    pub fn accept(&self, timeout: Duration) -> io::Result<u32> {
        let event = new_event().map_err(code_error)?;
        let h = self.conn.handle();
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            // Below INFINITE (u32::MAX), so a huge timeout still ends.
            let timeout_ms =
                u32::try_from(left.as_millis()).map_or(u32::MAX - 1, |ms| ms.min(u32::MAX - 1));
            // SAFETY: one overlapped ConnectNamedPipe on `h` with the given OVERLAPPED and no
            // buffer; `event` is a live manual-reset event for the whole call, and
            // `overlapped_io` cancels and waits for the operation before returning if it gives
            // up. ConnectNamedPipe resets the event when it starts, so it can be reused.
            let r = unsafe {
                overlapped_io(h, event.0, None, timeout_ms, |ov| {
                    ConnectNamedPipe(h, Some(ov))
                })
            };
            match r {
                Ok(_) => break,
                // The client connected before the call: nothing was pending.
                Err(IoFailure::Win32(code)) if code == ERROR_PIPE_CONNECTED.0 => break,
                // A client connected and closed before it was accepted: recycle the instance.
                Err(IoFailure::Win32(code)) if code == ERROR_NO_DATA.0 => {
                    // SAFETY: live server pipe handle with no operation pending on it.
                    unsafe { DisconnectNamedPipe(h) }.map_err(|e| os_error(&e))?;
                }
                Err(IoFailure::Win32(code)) => return Err(code_error(code)),
                Err(IoFailure::TimedOut | IoFailure::Stopped) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "no client connected in time",
                    ))
                }
            }
        }
        let mut pid = 0u32;
        // SAFETY: live, connected server pipe handle and a valid out pointer.
        unsafe { GetNamedPipeClientProcessId(h, &mut pid) }.map_err(|e| os_error(&e))?;
        Ok(pid)
    }

    /// The connected pipe, ready to send and read messages. Call it only
    /// after a successful [`accept`](Self::accept) and the caller's check of
    /// the client PID; never send before that.
    pub fn into_connection<M>(self) -> PrivateConnection<M> {
        PrivateConnection {
            conn: self.conn,
            _msg: PhantomData,
        }
    }
}

/// Opens the private pipe `name`, which must start with `prefix`, for
/// overlapped reads and writes, with an identification-level impersonation
/// token for the server.
pub fn connect_private_client<M>(
    prefix: &'static str,
    name: &str,
) -> io::Result<PrivateConnection<M>> {
    check_name(prefix, name)?;
    let path = HSTRING::from(name);
    // SAFETY: `path` is a valid NUL-terminated wide string for the call; the returned handle is
    // owned by an `OwnedHandle` right away.
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
    .map_err(|e| os_error(&e))?;
    let conn = PipeConn::new(OwnedHandle(h), PEER).map_err(code_error)?;
    Ok(PrivateConnection {
        conn,
        _msg: PhantomData,
    })
}

/// One connected end of a private pipe, speaking messages of type `M`.
pub struct PrivateConnection<M> {
    pub(crate) conn: Arc<PipeConn>,
    _msg: PhantomData<fn(&M)>,
}

impl<M: Serialize + DeserializeOwned + Send + 'static> PrivateConnection<M> {
    /// Writes one message, waiting at most 2 s for the other end to take it
    /// (the same timeout as the sensor pipe client). Concurrent calls are
    /// serialised. Any failure leaves the connection unusable: later calls
    /// fail with `BrokenPipe`.
    pub fn send(&self, msg: &M) -> io::Result<()> {
        let frame =
            encode_frame_of(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.conn.send_frame(&frame)
    }

    /// Starts the reader thread, which decodes messages and gives each to
    /// `deliver` until the connection closes or [`PipeReader::stop`].
    /// `deliver` must not block for good: it returns `false` when nobody can
    /// take the event, and the reader then closes the connection without a
    /// `Closed` event. Messages are not validated here: the receiver does it.
    ///
    /// One reader per connection: a second call only reports `Closed(Io(170))`.
    pub fn start_reader(
        &self,
        deliver: impl Fn(PipeEvent<M>) -> bool + Send + 'static,
    ) -> PipeReader {
        self.conn.start_reader(deliver)
    }
}
