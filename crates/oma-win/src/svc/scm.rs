//! Service control over the SCM (spike S3 §3).
//!
//! Each operation opens the SCM with `SC_MANAGER_CONNECT` and the service
//! with only the right it needs: the access check happens in `OpenServiceW`,
//! all or nothing, so asking for more would make a permitted call fail.

use windows::core::PCWSTR;
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx,
    StartServiceW, SC_HANDLE, SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_CONTROL_STOP,
    SERVICE_QUERY_STATUS, SERVICE_START, SERVICE_STATUS, SERVICE_STATUS_PROCESS, SERVICE_STOP,
};

use super::win32_code;

const _: () = assert!(size_of::<SERVICE_STATUS_PROCESS>() == 36);
const _: () = assert!(size_of::<SERVICE_STATUS>() == 28);

/// Name of the sensor service in the SCM.
pub const SERVICE_NAME: &str = "oma-service";

const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_SERVICE_ALREADY_RUNNING: u32 = 1056;
const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1060;
const ERROR_SERVICE_NOT_ACTIVE: u32 = 1062;

/// `dwCurrentState` of a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Stopped,
    StartPending,
    StopPending,
    Running,
    /// Continue pending, pause pending, paused or an unknown value.
    Other(u32),
}

impl RunState {
    pub fn from_raw(state: u32) -> Self {
        match state {
            1 => Self::Stopped,
            2 => Self::StartPending,
            3 => Self::StopPending,
            4 => Self::Running,
            other => Self::Other(other),
        }
    }
}

/// What [`ServiceControl::query`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceQuery {
    NotInstalled,
    AccessDenied,
    /// Any other SCM error: not proof that the service is missing.
    Error(u32),
    /// `pid` is 0 unless the service process is running.
    State {
        state: RunState,
        pid: u32,
    },
}

/// Maps the error of any step of a query.
pub(crate) fn query_error(code: u32) -> ServiceQuery {
    match code {
        ERROR_SERVICE_DOES_NOT_EXIST => ServiceQuery::NotInstalled,
        ERROR_ACCESS_DENIED => ServiceQuery::AccessDenied,
        other => ServiceQuery::Error(other),
    }
}

/// `StartServiceW` failing because the service already runs is a success.
pub(crate) fn start_result(code: u32) -> Result<(), u32> {
    match code {
        ERROR_SERVICE_ALREADY_RUNNING => Ok(()),
        other => Err(other),
    }
}

/// `SERVICE_CONTROL_STOP` failing because the service is not running is a success.
pub(crate) fn stop_result(code: u32) -> Result<(), u32> {
    match code {
        ERROR_SERVICE_NOT_ACTIVE => Ok(()),
        other => Err(other),
    }
}

/// Queries, starts and stops one service. A trait so the connection state
/// machine can be tested with a fake.
pub trait ServiceControl: Send + Sync {
    fn query(&self) -> ServiceQuery;
    /// Asks the SCM to start the service; `Err` carries the Win32 code.
    fn start(&self) -> Result<(), u32>;
    /// Sends `SERVICE_CONTROL_STOP`; `Err` carries the Win32 code.
    fn stop(&self) -> Result<(), u32>;
}

/// An SCM or service handle closed on drop.
struct ScHandle(SC_HANDLE);

impl Drop for ScHandle {
    fn drop(&mut self) {
        // SAFETY: this value is the sole owner of the handle, never used after drop.
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}

/// [`ServiceControl`] over the real SCM of this machine.
pub struct WindowsScm {
    /// The service name as a NUL-terminated wide string.
    name: Vec<u16>,
}

impl WindowsScm {
    pub fn new(service_name: &str) -> Self {
        Self {
            name: service_name.encode_utf16().chain(Some(0)).collect(),
        }
    }

    fn open(&self, access: u32) -> Result<ScHandle, u32> {
        // SAFETY: local machine and default database (null strings), connect right only; the
        // handle is owned by an `ScHandle` right away.
        let scm = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
            .map(ScHandle)
            .map_err(|e| win32_code(&e))?;
        // SAFETY: live SCM handle; `self.name` is NUL-terminated and outlives the call.
        unsafe { OpenServiceW(scm.0, PCWSTR(self.name.as_ptr()), access) }
            .map(ScHandle)
            .map_err(|e| win32_code(&e))
    }
}

impl ServiceControl for WindowsScm {
    fn query(&self) -> ServiceQuery {
        let service = match self.open(SERVICE_QUERY_STATUS) {
            Ok(service) => service,
            Err(code) => return query_error(code),
        };
        let mut status = SERVICE_STATUS_PROCESS::default();
        let mut needed = 0u32;
        // SAFETY: the byte view covers exactly `status`, a plain-data struct that outlives it.
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(
                (&mut status as *mut SERVICE_STATUS_PROCESS).cast::<u8>(),
                size_of::<SERVICE_STATUS_PROCESS>(),
            )
        };
        // SAFETY: live service handle opened with SERVICE_QUERY_STATUS; the buffer is as large
        // as SC_STATUS_PROCESS_INFO requires.
        match unsafe {
            QueryServiceStatusEx(service.0, SC_STATUS_PROCESS_INFO, Some(bytes), &mut needed)
        } {
            Ok(()) => ServiceQuery::State {
                state: RunState::from_raw(status.dwCurrentState.0),
                pid: status.dwProcessId,
            },
            Err(e) => query_error(win32_code(&e)),
        }
    }

    fn start(&self) -> Result<(), u32> {
        let service = self.open(SERVICE_START)?;
        // SAFETY: live service handle opened with SERVICE_START; no arguments.
        unsafe { StartServiceW(service.0, None) }.or_else(|e| start_result(win32_code(&e)))
    }

    fn stop(&self) -> Result<(), u32> {
        let service = self.open(SERVICE_STOP)?;
        let mut status = SERVICE_STATUS::default();
        // SAFETY: live service handle opened with SERVICE_STOP; `status` is a valid out pointer.
        unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) }
            .or_else(|e| stop_result(win32_code(&e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_states_map_to_run_states() {
        assert_eq!(RunState::from_raw(1), RunState::Stopped);
        assert_eq!(RunState::from_raw(2), RunState::StartPending);
        assert_eq!(RunState::from_raw(3), RunState::StopPending);
        assert_eq!(RunState::from_raw(4), RunState::Running);
        assert_eq!(RunState::from_raw(7), RunState::Other(7));
    }

    #[test]
    fn query_errors_map_to_outcomes() {
        assert_eq!(query_error(1060), ServiceQuery::NotInstalled);
        assert_eq!(query_error(5), ServiceQuery::AccessDenied);
        assert_eq!(query_error(1115), ServiceQuery::Error(1115));
    }

    #[test]
    fn already_running_and_not_active_are_success() {
        assert_eq!(start_result(1056), Ok(()));
        assert_eq!(start_result(1058), Err(1058));
        assert_eq!(start_result(5), Err(5));
        assert_eq!(stop_result(1062), Ok(()));
        assert_eq!(stop_result(1051), Err(1051));
        assert_eq!(stop_result(5), Err(5));
    }

    #[test]
    fn missing_service_is_not_installed() {
        assert_eq!(
            WindowsScm::new("oma-service-test-missing").query(),
            ServiceQuery::NotInstalled
        );
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn spooler_is_queryable_but_not_startable() {
        // Unelevated: the default Spooler DACL grants users query but not start,
        // so `start` fails at OpenServiceW and never reaches StartServiceW.
        let spooler = WindowsScm::new("Spooler");
        match spooler.query() {
            ServiceQuery::State {
                state: RunState::Running,
                pid,
            } => assert!(pid > 0),
            other => panic!("expected Spooler running, got {other:?}"),
        }
        assert_eq!(spooler.start(), Err(5));
    }
}
