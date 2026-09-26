//! Client side of the `oma-service` sensor service: the overlapped named-pipe
//! client ([`pipe`]) and the service control wrapper over the SCM ([`scm`]).
//! The connection state machine built on these lives in the app shell.

#[cfg(test)]
mod fake_server;
pub mod pipe;
pub mod scm;

/// The Win32 error code carried by a `windows` crate error (the HRESULT
/// itself when it is not a wrapped Win32 code).
pub(crate) fn win32_code(e: &windows::core::Error) -> u32 {
    windows::Win32::Foundation::WIN32_ERROR::from_error(e).map_or(e.code().0 as u32, |w| w.0)
}
