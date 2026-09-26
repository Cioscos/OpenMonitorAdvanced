//! In-process fake pipe server for CI tests (no admin, no .NET): same ACL shape, unique name.
use windows::Win32::Foundation::{
    ERROR_PIPE_CONNECTED, GetLastError, HLOCAL, INVALID_HANDLE_VALUE, LocalFree, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, PIPE_ACCESS_DUPLEX, ReadFile,
    WriteFile,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::core::HSTRING;

use crate::pipe::OwnedHandle;

pub const TIGHT_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x0012019b;;;IU)";

fn code(e: &windows::core::Error) -> WIN32_ERROR {
    WIN32_ERROR::from_error(e).unwrap_or(WIN32_ERROR(e.code().0 as u32))
}

pub fn create_instance(name: &str, sddl: &str, first: bool) -> Result<OwnedHandle, WIN32_ERROR> {
    let mut psd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: valid SDDL string; psd receives a LocalAlloc'd descriptor freed below.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(sddl),
            SDDL_REVISION_1,
            &mut psd,
            None,
        )
    }
    .map_err(|e| code(&e))?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: psd.0,
        bInheritHandle: false.into(),
    };
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    // SAFETY: sa and psd are alive for the call.
    let h = unsafe {
        CreateNamedPipeW(
            &HSTRING::from(format!(r"\\.\pipe\{name}")),
            FILE_FLAGS_AND_ATTRIBUTES(open_mode.0),
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            8,
            64 * 1024,
            64 * 1024,
            0,
            Some(&sa),
        )
    };
    // SAFETY: read right after the failing call, before LocalFree can overwrite it.
    let err = unsafe { GetLastError() };
    // SAFETY: psd came from LocalAlloc inside the SDDL conversion.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(psd.0)));
    }
    if h == INVALID_HANDLE_VALUE {
        return Err(err);
    }
    Ok(OwnedHandle(h))
}

fn write_frame(h: &OwnedHandle, s: &[u8]) -> bool {
    let mut buf = (s.len() as u32).to_le_bytes().to_vec();
    buf.extend_from_slice(s);
    // SAFETY: synchronous write on a live server handle.
    unsafe { WriteFile(h.0, Some(&buf), None, None) }.is_ok()
}

fn read_exact(h: &OwnedHandle, buf: &mut [u8]) -> bool {
    let mut off = 0;
    while off < buf.len() {
        let mut n = 0u32;
        // SAFETY: synchronous read on a live server handle.
        if unsafe { ReadFile(h.0, Some(&mut buf[off..]), Some(&mut n), None) }.is_err() {
            return false;
        }
        off += n as usize;
    }
    true
}

/// Serves `clients` connections one after the other on a single instance.
pub fn serve(inst: OwnedHandle, clients: usize) {
    for _ in 0..clients {
        // SAFETY: live server instance; ERROR_PIPE_CONNECTED means a client raced in first.
        if let Err(e) = unsafe { ConnectNamedPipe(inst.0, None) } {
            if code(&e) != ERROR_PIPE_CONNECTED {
                return;
            }
        }
        write_frame(&inst, b"hello");
        loop {
            let mut hdr = [0u8; 4];
            if !read_exact(&inst, &mut hdr) {
                break;
            }
            let mut body = vec![0u8; u32::from_le_bytes(hdr) as usize];
            if !read_exact(&inst, &mut body) {
                break;
            }
            if body == b"bye" {
                break;
            }
            let mut echo = b"echo:".to_vec();
            echo.extend_from_slice(&body);
            write_frame(&inst, &echo);
            for i in 1..=3 {
                write_frame(&inst, format!("snap-{i}").as_bytes());
            }
        }
        // SAFETY: live instance; makes it reusable for the next client.
        unsafe {
            let _ = DisconnectNamedPipe(inst.0);
        }
    }
}
