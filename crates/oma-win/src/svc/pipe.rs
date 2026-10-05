//! Overlapped client for the sensor named pipe (spec §6, spike S3 §2), on
//! top of the shared overlapped I/O in [`crate::pipe_io`].

use std::io;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::Arc;

use oma_ipc::{encode_frame, Message};
use windows::core::HSTRING;
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_MODE, OPEN_EXISTING, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;

use super::win32_code;
pub use crate::pipe_io::{CloseReason, PipeReader};
use crate::pipe_io::{OwnedHandle, PipeConn};

/// What the reader thread delivers on the caller's channel.
pub type PipeEvent = crate::pipe_io::PipeEvent<Message>;

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

/// A connection to the sensor pipe.
pub struct PipeClient {
    conn: Arc<PipeConn>,
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
        let conn = PipeConn::new(OwnedHandle(h), "sensor service").map_err(ConnectError::Other)?;
        Ok(PipeClient { conn })
    }

    /// PID of the process serving the other end (for the service identity check).
    pub fn server_pid(&self) -> Option<u32> {
        let mut pid = 0u32;
        // SAFETY: live client pipe handle and a valid out pointer.
        unsafe { GetNamedPipeServerProcessId(self.conn.handle(), &mut pid) }.ok()?;
        Some(pid)
    }

    /// Writes one frame, waiting at most 2 s for the server to take it.
    /// Concurrent calls are serialised. Any failure after the connection
    /// was opened leaves it unusable (a frame may be half written): later
    /// calls fail with `BrokenPipe`, and the caller should drop the client.
    pub fn send(&self, msg: &Message) -> io::Result<()> {
        let frame = encode_frame(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.conn.send_frame(&frame)
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
        self.conn.start_reader(deliver)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError};
    use std::time::{Duration, Instant};

    use oma_ipc::{Hello, IpcError, Subscribe, WireSnapshot, MAX_FRAME_BYTES};

    use super::*;
    use crate::pipe_io::close_reason_for;
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
