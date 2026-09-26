//! In-process fake sensor pipe server for the client tests: one synchronous
//! instance with the default DACL and a name unique to this process, so the
//! tests need neither administrator rights nor the real service.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};

use oma_ipc::{encode_frame, FrameDecoder, Message};
use windows::core::HSTRING;
use windows::Win32::Foundation::{GetLastError, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    FlushFileBuffers, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};

use super::pipe::OwnedHandle;
use super::win32_code;

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A name no other test (in this or another test process) is using.
pub(crate) fn unique_name() -> String {
    format!(
        "OpenMonitorAdvanced.Sensors.test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// One listening instance of a byte-mode pipe (64 KiB buffers each way).
pub(crate) struct FakeServer {
    pub(crate) name: String,
    pipe: OwnedHandle,
    decoder: RefCell<FrameDecoder>,
}

impl FakeServer {
    pub(crate) fn new() -> Self {
        let name = unique_name();
        let path = HSTRING::from(format!(r"\\.\pipe\{name}"));
        // SAFETY: `path` is a valid NUL-terminated wide string for the call;
        // no security attributes, so the pipe gets the default DACL.
        let h = unsafe {
            CreateNamedPipeW(
                &path,
                FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_DUPLEX.0 | FILE_FLAG_FIRST_PIPE_INSTANCE.0),
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                64 * 1024,
                64 * 1024,
                0,
                None,
            )
        };
        if h == INVALID_HANDLE_VALUE {
            // SAFETY: read right after the failing call on this thread.
            let err = unsafe { GetLastError() };
            panic!("CreateNamedPipeW({name}) failed: {}", err.0);
        }
        Self {
            name,
            pipe: OwnedHandle(h),
            decoder: RefCell::new(FrameDecoder::new()),
        }
    }

    /// Waits for the client. A client that opened the pipe before this call
    /// (the usual order in the tests) makes it return at once.
    pub(crate) fn accept(&self) {
        // SAFETY: live server instance, synchronous call.
        if let Err(e) = unsafe { ConnectNamedPipe(self.pipe.0, None) } {
            assert_eq!(
                win32_code(&e),
                ERROR_PIPE_CONNECTED.0,
                "ConnectNamedPipe failed"
            );
        }
    }

    pub(crate) fn send(&self, msg: &Message) {
        self.send_raw(&encode_frame(msg).expect("encode"));
    }

    pub(crate) fn send_raw(&self, bytes: &[u8]) {
        let mut off = 0;
        while off < bytes.len() {
            let mut n = 0u32;
            // SAFETY: synchronous write from a live buffer on a live handle.
            unsafe { WriteFile(self.pipe.0, Some(&bytes[off..]), Some(&mut n), None) }
                .expect("fake server WriteFile");
            off += n as usize;
        }
    }

    /// Blocks until one whole message has arrived from the client.
    pub(crate) fn recv(&self) -> Message {
        let mut decoder = self.decoder.borrow_mut();
        let mut buf = [0u8; 4096];
        loop {
            if let Some(msg) = decoder.next_message().expect("fake server decode") {
                return msg;
            }
            let mut n = 0u32;
            // SAFETY: synchronous read into a live buffer on a live handle.
            unsafe { ReadFile(self.pipe.0, Some(&mut buf), Some(&mut n), None) }
                .expect("fake server ReadFile");
            decoder.push(&buf[..n as usize]).expect("fake server push");
        }
    }

    /// Blocks until the client has read everything written so far.
    pub(crate) fn flush(&self) {
        // SAFETY: live server instance; on a pipe this waits for the client to drain it.
        unsafe { FlushFileBuffers(self.pipe.0) }.expect("FlushFileBuffers");
    }

    /// Server-side disconnect: the client's pending read fails with 233.
    pub(crate) fn disconnect(&self) {
        // SAFETY: live server instance.
        unsafe { DisconnectNamedPipe(self.pipe.0) }.expect("DisconnectNamedPipe");
    }
}
