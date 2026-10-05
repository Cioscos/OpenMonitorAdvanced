//! The private pipe between the app and `oma-overlay.exe` (M7c).
//!
//! The app creates the pipe under a random name (`OVERLAY_PIPE_PREFIX` + a
//! UUID v4 from the system RNG) and passes the name to the overlay it
//! spawns. The server end is hardened so that only that overlay can be the
//! client:
//!
//! - `FILE_FLAG_FIRST_PIPE_INSTANCE` with a single instance, so nobody can
//!   have created the name first and nobody can add a second instance;
//! - `PIPE_REJECT_REMOTE_CLIENTS`;
//! - a protected DACL that grants access to the current user's SID only
//!   (no inherited ACEs, no Administrators, no SYSTEM);
//! - [`OverlayPipeServer::accept`] returns the client's PID, which the
//!   caller compares with the PID of the overlay it spawned.
//!
//! Both ends then speak the overlay protocol with the overlapped framing of
//! [`crate::pipe_io`]. Validating each decoded message
//! ([`OverlayMessage::validate`]) is the receiver's job.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oma_ipc::encode_frame_of;
use oma_ipc::overlay::{OverlayMessage, OVERLAY_PIPE_PREFIX};
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{
    GetLastError, LocalFree, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE,
    HANDLE, HLOCAL, INVALID_HANDLE_VALUE,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    FILE_SHARE_MODE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::pipe_io::{new_event, overlapped_io, IoFailure, OwnedHandle, PipeConn};
pub use crate::pipe_io::{CloseReason, PipeEvent, PipeReader};
use crate::svc::win32_code;

/// Size of each of the pipe's two buffers.
const BUFFER_BYTES: u32 = 64 * 1024;

/// Names the other end in logs and errors.
const PEER: &str = "overlay pipe peer";

/// A fresh overlay pipe name: [`OVERLAY_PIPE_PREFIX`] followed by a UUID v4
/// drawn from the system-preferred RNG.
pub fn random_pipe_name() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    // SAFETY: `bytes` is a live, writable 16-byte buffer for the whole call; no algorithm
    // handle is needed with BCRYPT_USE_SYSTEM_PREFERRED_RNG.
    let status = unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status.is_err() {
        return Err(io::Error::other(format!(
            "BCryptGenRandom failed with status {:#010x}",
            status.0 as u32
        )));
    }
    Ok(format!("{OVERLAY_PIPE_PREFIX}{}", uuid_v4(bytes)))
}

/// The pipe's security descriptor: owner `sid` (so an elevated app does not
/// hand ownership to Administrators), a protected DACL with one ACE (generic
/// all for `sid`), and a medium mandatory label with no-write-up and
/// no-read-up, so low-integrity processes of the same user cannot open it.
pub(crate) fn user_only_sddl(sid: &str) -> String {
    format!("O:{sid}D:P(A;;GA;;;{sid})S:(ML;;NWNR;;;ME)")
}

/// Formats 16 random bytes as a UUID v4 (RFC 4122 variant), lowercase.
pub(crate) fn uuid_v4(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The name starts with [`OVERLAY_PIPE_PREFIX`] and names nothing below it.
fn is_overlay_pipe_name(name: &str) -> bool {
    name.strip_prefix(OVERLAY_PIPE_PREFIX)
        .is_some_and(|rest| !rest.is_empty() && !rest.contains(['\\', '/']))
}

fn check_name(name: &str) -> io::Result<()> {
    if is_overlay_pipe_name(name) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name:?} is not an overlay pipe name"),
        ))
    }
}

fn os_error(e: &windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(win32_code(e) as i32)
}

fn code_error(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}

/// Memory the security APIs allocated with `LocalAlloc`, freed on drop.
struct LocalMem(*mut core::ffi::c_void);

impl Drop for LocalMem {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from LocalAlloc inside the API that returned it, is owned
            // by this guard alone and is freed exactly once.
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0)));
            }
        }
    }
}

/// The SID of the user of this process's token, as a string (`S-1-5-21-…`).
fn current_user_sid() -> io::Result<String> {
    let mut token = HANDLE::default();
    // SAFETY: the pseudo-handle of the current process needs no closing; `token` receives a
    // handle that the `OwnedHandle` below closes.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| os_error(&e))?;
    let token = OwnedHandle(token);

    let mut needed = 0u32;
    // SAFETY: a size query without a buffer: it fails with ERROR_INSUFFICIENT_BUFFER and
    // writes the size the TOKEN_USER needs into `needed`.
    let _ = unsafe { GetTokenInformation(token.0, TokenUser, None, 0, &mut needed) };
    if (needed as usize) < size_of::<TOKEN_USER>() {
        return Err(io::Error::last_os_error());
    }
    // `u64` elements keep the buffer aligned for TOKEN_USER (pointer-aligned).
    let mut buf = vec![0u64; (needed as usize).div_ceil(size_of::<u64>())];
    let len = (buf.len() * size_of::<u64>()) as u32;
    // SAFETY: `buf` is a live, writable, 8-aligned buffer of `len` bytes for the call.
    unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            len,
            &mut needed,
        )
    }
    .map_err(|e| os_error(&e))?;
    // SAFETY: the call succeeded, so the buffer starts with an initialised TOKEN_USER whose SID
    // points inside `buf`, which outlives this borrow and is not written while it lives.
    let user = unsafe { &*buf.as_ptr().cast::<TOKEN_USER>() };

    let mut text = PWSTR::null();
    // SAFETY: `user.User.Sid` is a valid SID inside `buf` (alive here); `text` receives a
    // LocalAlloc'd NUL-terminated string that the `LocalMem` guard frees.
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) }.map_err(|e| os_error(&e))?;
    let text = LocalMem(text.0.cast());
    // SAFETY: `text` is a NUL-terminated wide string, alive until the guard drops.
    unsafe { PWSTR(text.0.cast()).to_string() }
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// The server end of the overlay pipe, owned by the app.
pub struct OverlayPipeServer {
    conn: Arc<PipeConn>,
}

impl OverlayPipeServer {
    /// Creates the only instance of the pipe `name` (which must start with
    /// [`OVERLAY_PIPE_PREFIX`]), with the descriptor of [`user_only_sddl`]
    /// for the current user. When the name exists it fails: with
    /// `ERROR_PIPE_BUSY` (231) if it is our own instance (the one-instance
    /// limit is checked first), with `ERROR_ACCESS_DENIED` (5) if another
    /// process created it (`FILE_FLAG_FIRST_PIPE_INSTANCE`).
    pub fn create(name: &str) -> io::Result<Self> {
        check_name(name)?;
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
    /// which the caller checks against the overlay it spawned. Fails with
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
                        "no overlay client connected in time",
                    ))
                }
            }
        }
        let mut pid = 0u32;
        // SAFETY: live, connected server pipe handle and a valid out pointer.
        unsafe { GetNamedPipeClientProcessId(h, &mut pid) }.map_err(|e| os_error(&e))?;
        Ok(pid)
    }

    /// The connected pipe, ready to send and read overlay messages. Call it
    /// only after a successful [`accept`](Self::accept) and the caller's
    /// check of the client PID; never send before that.
    pub fn into_connection(self) -> OverlayConnection {
        OverlayConnection { conn: self.conn }
    }
}

/// Opens the overlay pipe `name` (used by `oma-overlay`), which must start
/// with [`OVERLAY_PIPE_PREFIX`], for overlapped reads and writes, with an
/// identification-level impersonation token for the server.
pub fn connect_overlay_client(name: &str) -> io::Result<OverlayConnection> {
    check_name(name)?;
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
    Ok(OverlayConnection { conn })
}

/// One connected end of the overlay pipe (app or overlay).
pub struct OverlayConnection {
    conn: Arc<PipeConn>,
}

impl OverlayConnection {
    /// Writes one message, waiting at most 2 s for the other end to take it
    /// (the same timeout as the sensor pipe client). Concurrent calls are
    /// serialised. Any failure leaves the connection unusable: later calls
    /// fail with `BrokenPipe`.
    pub fn send(&self, msg: &OverlayMessage) -> io::Result<()> {
        let frame =
            encode_frame_of(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.conn.send_frame(&frame)
    }

    /// Starts the reader thread, which decodes overlay messages and gives
    /// each to `deliver` until the connection closes or [`PipeReader::stop`].
    /// `deliver` must not block for good: it returns `false` when nobody can
    /// take the event, and the reader then closes the connection without a
    /// `Closed` event. Messages are not validated here: the receiver calls
    /// [`OverlayMessage::validate`].
    ///
    /// One reader per connection: a second call only reports `Closed(Io(170))`.
    pub fn start_reader(
        &self,
        deliver: impl Fn(PipeEvent<OverlayMessage>) -> bool + Send + 'static,
    ) -> PipeReader {
        self.conn.start_reader(deliver)
    }
}
#[cfg(test)]
mod tests {
    use std::sync::mpsc::{sync_channel, SyncSender};
    use std::time::Instant;

    use oma_ipc::overlay::{
        OverlayHello, PxArea, SetPlacement, OVERLAY_PIPE_PREFIX, OVERLAY_PROTOCOL_VERSION,
    };

    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
    };
    use windows::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    };
    use windows::Win32::System::Pipes::PIPE_UNLIMITED_INSTANCES;

    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    fn into_channel(
        tx: SyncSender<PipeEvent<OverlayMessage>>,
    ) -> impl Fn(PipeEvent<OverlayMessage>) -> bool + Send + 'static {
        move |event| tx.try_send(event).is_ok()
    }

    fn is_lower_hex(s: &str) -> bool {
        s.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    }

    #[test]
    fn user_only_sddl_grants_generic_all_to_the_sid() {
        assert_eq!(
            user_only_sddl("S-1-5-21-1-2-3-1001"),
            "O:S-1-5-21-1-2-3-1001D:P(A;;GA;;;S-1-5-21-1-2-3-1001)S:(ML;;NWNR;;;ME)"
        );
    }

    #[test]
    fn uuid_v4_sets_version_and_variant() {
        assert_eq!(uuid_v4([0; 16]), "00000000-0000-4000-8000-000000000000");
        assert_eq!(uuid_v4([0xff; 16]), "ffffffff-ffff-4fff-bfff-ffffffffffff");
        let bytes = [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB,
            0xCD, 0xEF,
        ];
        assert_eq!(uuid_v4(bytes), "01234567-89ab-4def-8123-456789abcdef");
    }

    #[test]
    fn random_pipe_name_has_prefix_and_uuid() {
        let a = random_pipe_name().expect("random name");
        let b = random_pipe_name().expect("random name");
        assert_ne!(a, b);
        let uuid = a.strip_prefix(OVERLAY_PIPE_PREFIX).expect("prefix");
        assert_eq!(uuid.len(), 36);
        let groups: Vec<&str> = uuid.split('-').collect();
        assert_eq!(
            groups.iter().map(|g| g.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(groups.iter().all(|g| is_lower_hex(g)), "{uuid}");
        assert!(groups[2].starts_with('4'), "{uuid}");
        assert!(groups[3].starts_with(['8', '9', 'a', 'b']), "{uuid}");
    }

    #[test]
    fn server_and_client_exchange_overlay_messages() {
        let name = random_pipe_name().unwrap();
        let server = OverlayPipeServer::create(&name).expect("create the server");
        let client = connect_overlay_client(&name).expect("connect the client");
        server.accept(WAIT).expect("accept");
        let server = server.into_connection();

        let (server_tx, server_rx) = sync_channel(8);
        let server_reader = server.start_reader(into_channel(server_tx));
        let (client_tx, client_rx) = sync_channel(8);
        let client_reader = client.start_reader(into_channel(client_tx));

        let placement = OverlayMessage::SetPlacement(SetPlacement {
            area: Some(PxArea {
                x: 10,
                y: 20,
                width: 1920,
                height: 1080,
            }),
            dpi: 144,
        });
        server.send(&placement).expect("server send");
        match client_rx.recv_timeout(WAIT).expect("SetPlacement") {
            PipeEvent::Message(msg) => assert_eq!(msg, placement),
            other => panic!("expected SetPlacement, got {other:?}"),
        }

        let hello = OverlayMessage::Hello(OverlayHello {
            protocol_version: OVERLAY_PROTOCOL_VERSION,
            version: "0.5.0".to_owned(),
        });
        client.send(&hello).expect("client send");
        match server_rx.recv_timeout(WAIT).expect("Hello") {
            PipeEvent::Message(msg) => assert_eq!(msg, hello),
            other => panic!("expected Hello, got {other:?}"),
        }

        client_reader.stop();
        server_reader.stop();
    }

    #[test]
    fn second_server_with_same_name_fails() {
        let name = random_pipe_name().unwrap();
        let _first = OverlayPipeServer::create(&name).expect("create the first server");
        let err = OverlayPipeServer::create(&name)
            .err()
            .expect("the name is taken");
        // Our single instance is taken: ERROR_PIPE_BUSY (231) comes before the first-instance
        // check; ERROR_ACCESS_DENIED (5) would be FILE_FLAG_FIRST_PIPE_INSTANCE refusing.
        assert!(
            matches!(err.raw_os_error(), Some(231) | Some(5)),
            "unexpected error {err:?}"
        );
    }

    #[test]
    fn create_refuses_a_name_someone_else_created_first() {
        let name = random_pipe_name().unwrap();
        let path = HSTRING::from(name.as_str());
        // A squatter: default DACL, unlimited instances, no first-instance flag.
        // SAFETY: `path` is a valid NUL-terminated wide string; the handle is owned right away.
        let squatter = OwnedHandle(unsafe {
            CreateNamedPipeW(
                &path,
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER_BYTES,
                BUFFER_BYTES,
                0,
                None,
            )
        });
        assert!(squatter.0 != INVALID_HANDLE_VALUE, "squatter pipe");
        let err = OverlayPipeServer::create(&name)
            .err()
            .expect("the squatted name is refused");
        assert_eq!(err.raw_os_error(), Some(5));
    }

    #[test]
    fn server_dacl_admits_only_the_current_user() {
        let name = random_pipe_name().unwrap();
        let server = OverlayPipeServer::create(&name).unwrap();
        let sid = current_user_sid().expect("current user SID");
        assert!(sid.starts_with("S-1-5-"), "{sid}");

        let info =
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION;
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        // SAFETY: live server handle (it has READ_CONTROL); `sd` receives a LocalAlloc'd
        // descriptor freed by the guard below, and `dacl` points inside it.
        let err = unsafe {
            GetSecurityInfo(
                server.conn.handle(),
                SE_KERNEL_OBJECT,
                info,
                None,
                None,
                Some(&mut dacl),
                None,
                Some(&mut sd),
            )
        };
        let sd = LocalMem(sd.0);
        assert_eq!(err.0, 0, "GetSecurityInfo");
        let mut text = PWSTR::null();
        // SAFETY: `sd` is a valid self-relative descriptor; `text` receives a LocalAlloc'd
        // string freed by the guard below.
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                PSECURITY_DESCRIPTOR(sd.0),
                SDDL_REVISION_1,
                info,
                &mut text,
                None,
            )
        }
        .expect("descriptor to SDDL");
        let text = LocalMem(text.0.cast());
        // SAFETY: NUL-terminated wide string alive until the guard drops.
        let sddl = unsafe { PWSTR(text.0.cast()).to_string() }.unwrap();
        // Owned by this user, not by Administrators even when elevated.
        let rest = sddl
            .strip_prefix(&format!("O:{sid}"))
            .unwrap_or_else(|| panic!("owner: {sddl}"));
        let (dacl, label) = rest
            .split_once("S:")
            .unwrap_or_else(|| panic!("no label: {sddl}"));
        // Protected, one allow ACE, for this user only (generic all may read back mapped).
        assert!(dacl.starts_with("D:P(A;;"), "{sddl}");
        assert!(dacl.ends_with(&format!(";;;{sid})")), "{sddl}");
        assert_eq!(dacl.matches('(').count(), 1, "{sddl}");
        // Medium integrity, no write-up and no read-up: low-integrity processes stay out.
        // The kernel may add SACL control flags (`AI`) before the one label ACE.
        assert!(label.ends_with("(ML;;NWNR;;;ME)"), "{sddl}");
        assert_eq!(label.matches('(').count(), 1, "{sddl}");
    }

    #[test]
    fn accept_reports_the_client_pid() {
        let name = random_pipe_name().unwrap();
        let server = OverlayPipeServer::create(&name).unwrap();
        let _client = connect_overlay_client(&name).expect("connect the client");
        assert_eq!(server.accept(WAIT).expect("accept"), std::process::id());
    }

    #[test]
    fn accept_skips_a_client_that_already_left() {
        let name = random_pipe_name().unwrap();
        let server = OverlayPipeServer::create(&name).unwrap();
        // Connects and closes before the server accepts: ConnectNamedPipe sees ERROR_NO_DATA.
        drop(connect_overlay_client(&name).expect("connect the first client"));
        let err = server
            .accept(Duration::from_millis(200))
            .expect_err("the departed client is not accepted");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err:?}");
        // The instance was recycled: the next client gets in.
        let _client = connect_overlay_client(&name).expect("connect the second client");
        assert_eq!(server.accept(WAIT).expect("accept"), std::process::id());
    }

    #[test]
    fn accept_times_out_without_client() {
        let name = random_pipe_name().unwrap();
        let server = OverlayPipeServer::create(&name).unwrap();
        let t = Instant::now();
        let err = server
            .accept(Duration::from_millis(200))
            .expect_err("nobody connects");
        let took = t.elapsed();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(
            took >= Duration::from_millis(180) && took < Duration::from_secs(2),
            "accept gave up after {took:?}"
        );
        // The server still works after a timed-out accept.
        let _client = connect_overlay_client(&name).expect("connect after the timeout");
        assert_eq!(server.accept(WAIT).expect("accept"), std::process::id());
    }

    #[test]
    fn client_refuses_a_name_without_the_prefix() {
        let name = random_pipe_name().unwrap();
        let _server = OverlayPipeServer::create(&name).unwrap();
        let foreign = name.replace("OpenMonitorAdvanced-Overlay-", "Other-");
        let err = connect_overlay_client(&foreign)
            .err()
            .expect("foreign name");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let err = connect_overlay_client(r"\\.\pipe\OpenMonitorAdvanced.Sensors.v1")
            .err()
            .expect("sensor pipe name");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
