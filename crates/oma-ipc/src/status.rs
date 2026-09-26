//! Status of the sensor service as the app shell and the UI see it.
//!
//! These are not wire messages: they never travel on the pipe. They live in
//! this portable crate so the shell can name them on every platform (outside
//! Windows it only ever reports [`ServiceState::NotInstalled`]), while the
//! Windows connection manager in `oma-win` re-exports and produces them.

use serde::{Deserialize, Serialize};

/// What the app can say about the sensor service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceState {
    /// The service is not installed on this machine.
    NotInstalled,
    /// Anti-cheat compatible mode: the app keeps the service stopped.
    AntiCheat,
    /// The service is starting, or the app is connecting to it.
    Starting,
    /// Snapshots are arriving.
    Connected,
    /// The service cannot be reached; the detail says why, when known.
    Unreachable,
    /// The service speaks another protocol version.
    Incompatible,
}

/// Why the service is in its current state, when there is more to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceDetail {
    /// This user may not query or start the service (or open its pipe).
    AccessDenied,
    /// The SCM refused to start the service.
    StartFailed,
    /// Anti-cheat mode: the stop is requested and not yet confirmed.
    Stopping,
    /// Anti-cheat mode: the service could not be stopped (or the stop could
    /// not be confirmed in time).
    StopFailed,
    /// The pipe is not served by the registered service process.
    PidMismatch,
    /// The connection closed, timed out or broke the protocol.
    Disconnected,
}

/// The service status shown by the shell: `detail` is always serialized
/// (`null` when there is none).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub state: ServiceState,
    pub detail: Option<ServiceDetail>,
}

impl ServiceStatus {
    pub const fn new(state: ServiceState, detail: Option<ServiceDetail>) -> Self {
        Self { state, detail }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_status_serializes_in_camel_case() {
        let anti_cheat = ServiceStatus::new(ServiceState::AntiCheat, None);
        assert_eq!(
            serde_json::to_string(&anti_cheat).unwrap(),
            r#"{"state":"antiCheat","detail":null}"#
        );
        let mismatch =
            ServiceStatus::new(ServiceState::Unreachable, Some(ServiceDetail::PidMismatch));
        assert_eq!(
            serde_json::to_string(&mismatch).unwrap(),
            r#"{"state":"unreachable","detail":"pidMismatch"}"#
        );
        assert_eq!(
            serde_json::from_str::<ServiceStatus>(r#"{"state":"notInstalled","detail":null}"#)
                .unwrap(),
            ServiceStatus::new(ServiceState::NotInstalled, None)
        );
    }
}
