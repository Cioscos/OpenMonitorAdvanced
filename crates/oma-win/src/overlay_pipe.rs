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
use std::time::Duration;

use oma_ipc::overlay::{OverlayMessage, OVERLAY_PIPE_PREFIX};
use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};

pub use crate::pipe_io::{CloseReason, PipeEvent, PipeReader};
use crate::private_pipe::{connect_private_client, PrivateConnection, PrivatePipeServer};

/// A fresh overlay pipe name: [`OVERLAY_PIPE_PREFIX`] followed by a UUID v4
/// drawn from the system-preferred RNG.
pub fn random_pipe_name() -> io::Result<String> {
    Ok(format!("{OVERLAY_PIPE_PREFIX}{}", random_uuid_v4()?))
}

/// A random UUID v4 (lowercase, 8-4-4-4-12) drawn from the system-preferred RNG.
pub fn random_uuid_v4() -> io::Result<String> {
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
    Ok(uuid_v4(bytes))
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

/// The server end of the overlay pipe, owned by the app.
pub struct OverlayPipeServer(PrivatePipeServer);

impl OverlayPipeServer {
    /// Creates the only instance of the pipe `name` (which must start with
    /// [`OVERLAY_PIPE_PREFIX`]); see [`PrivatePipeServer::create`] for the
    /// failure modes when the name exists.
    pub fn create(name: &str) -> io::Result<Self> {
        PrivatePipeServer::create(OVERLAY_PIPE_PREFIX, name).map(Self)
    }

    /// Waits at most `timeout` for a client and returns its PID; see
    /// [`PrivatePipeServer::accept`].
    pub fn accept(&self, timeout: Duration) -> io::Result<u32> {
        self.0.accept(timeout)
    }

    /// The connected pipe; call it only after a successful
    /// [`accept`](Self::accept) and the caller's check of the client PID.
    pub fn into_connection(self) -> OverlayConnection {
        self.0.into_connection()
    }
}

/// One connected end of the overlay pipe (app or overlay).
pub type OverlayConnection = PrivateConnection<OverlayMessage>;

/// Opens the overlay pipe `name` (used by `oma-overlay`), which must start
/// with [`OVERLAY_PIPE_PREFIX`].
pub fn connect_overlay_client(name: &str) -> io::Result<OverlayConnection> {
    connect_private_client(OVERLAY_PIPE_PREFIX, name)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{sync_channel, SyncSender};
    use std::time::Instant;

    use oma_ipc::overlay::{
        OverlayHello, PxArea, SetPlacement, OVERLAY_PIPE_PREFIX, OVERLAY_PROTOCOL_VERSION,
    };

    use crate::pipe_io::OwnedHandle;
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
        SE_KERNEL_OBJECT,
    };
    use windows::Win32::Security::PSECURITY_DESCRIPTOR;
    use windows::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    };
    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows::Win32::System::Pipes::PIPE_UNLIMITED_INSTANCES;
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    use super::*;
    use crate::private_pipe::{current_user_sid, user_only_sddl, LocalMem, BUFFER_BYTES};

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
    fn random_uuid_v4_is_lowercase_8_4_4_4_12() {
        let a = random_uuid_v4().expect("uuid");
        let b = random_uuid_v4().expect("uuid");
        assert_ne!(a, b);
        let lens: Vec<usize> = a.split('-').map(str::len).collect();
        assert_eq!(lens, [8, 4, 4, 4, 12], "{a}");
        assert!(
            a.chars()
                .all(|c| c == '-' || matches!(c, '0'..='9' | 'a'..='f')),
            "{a}"
        );
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
        // SDDL writes the built-in Administrator (RID 500, the CI runner's account) as `LA`.
        let sid = if sid.ends_with("-500") {
            "LA".to_owned()
        } else {
            sid
        };

        let info =
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION;
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        // SAFETY: live server handle (it has READ_CONTROL); `sd` receives a LocalAlloc'd
        // descriptor freed by the guard below, and `dacl` points inside it.
        let err = unsafe {
            GetSecurityInfo(
                server.0.conn.handle(),
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
