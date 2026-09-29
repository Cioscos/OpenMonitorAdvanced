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

/// The state of the PawnIO driver as the service reports it in `Hello` (spec M5 §2.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PawnIoStatus {
    /// The driver opens: the advanced sensors work.
    Ok,
    /// The driver is not installed.
    Missing,
    /// The driver is there but cannot be opened or loaded.
    Unavailable,
    /// The diagnosis was not conclusive.
    Unknown,
    /// The installer asked for a restart that has not happened yet.
    RebootPending,
}

impl PawnIoStatus {
    /// The status named by a `Hello::pawn_io` string; anything unrecognised
    /// (a newer service, a corrupt value) reads as [`PawnIoStatus::Unknown`].
    pub fn from_wire(value: &str) -> Self {
        match value {
            "ok" => Self::Ok,
            "missing" => Self::Missing,
            "unavailable" => Self::Unavailable,
            "rebootPending" => Self::RebootPending,
            _ => Self::Unknown,
        }
    }
}

/// Whether the service has taken the sources this client asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reconfiguration {
    Applied,
    /// The request was sent and the service has not reflected it yet.
    Pending,
    /// The service did not manage to apply the change in time.
    Failed,
}

impl Reconfiguration {
    /// The state named by `WireServiceState::reconfiguration`; an
    /// unrecognised value reads as [`Reconfiguration::Applied`], so a newer
    /// service never leaves the app waiting for a state it cannot name.
    pub fn from_wire(value: &str) -> Self {
        match value {
            "pending" => Self::Pending,
            "failed" => Self::Failed,
            _ => Self::Applied,
        }
    }
}

/// The service's effective sources as the UI sees them: the service is
/// shared by every client, so what runs may differ from what this app asked.
/// Disks are named by core device id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSources {
    /// Modules that are on ([`crate::MODULES`] names).
    pub active_modules: Vec<String>,
    /// Core ids of the disks whose SMART is off.
    pub smart_disabled_drives: Vec<String>,
    pub reconfiguration: Reconfiguration,
    /// Core ids of the disks that keep SMART closed for all disks. A disk
    /// this app cannot identify shows as its raw key (never a core id), which
    /// the UI reads as an unknown disk; the list can be empty while the gate
    /// is closed when the blocking disk has no model or serial.
    pub smart_blocked_by: Vec<String>,
}

/// What this app asks of the service and filters locally, by core ids: the
/// modules the user turned off and the disks whose SMART is off. The Windows
/// link translates it for the wire (disks become [`crate::drive_key`]s); the
/// `svc` provider applies it to what the service sends, whether or not
/// another client keeps those sources on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceRequest {
    /// Names from [`crate::MODULES`].
    pub disabled_modules: Vec<String>,
    /// Core device ids of disks.
    pub smart_disabled_drives: Vec<String>,
}

/// The service status shown by the shell: `detail`, `pawn_io` and `sources`
/// are always serialized (`null` when there is none). The last two exist only
/// while the app is connected to the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub state: ServiceState,
    pub detail: Option<ServiceDetail>,
    pub pawn_io: Option<PawnIoStatus>,
    pub sources: Option<ServiceSources>,
}

impl ServiceStatus {
    /// A status without PawnIO or sources: what the app can say before it is
    /// connected.
    pub const fn new(state: ServiceState, detail: Option<ServiceDetail>) -> Self {
        Self {
            state,
            detail,
            pawn_io: None,
            sources: None,
        }
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
            r#"{"state":"antiCheat","detail":null,"pawnIo":null,"sources":null}"#
        );
        let mismatch =
            ServiceStatus::new(ServiceState::Unreachable, Some(ServiceDetail::PidMismatch));
        assert_eq!(
            serde_json::to_string(&mismatch).unwrap(),
            r#"{"state":"unreachable","detail":"pidMismatch","pawnIo":null,"sources":null}"#
        );
        assert_eq!(
            serde_json::from_str::<ServiceStatus>(
                r#"{"state":"notInstalled","detail":null,"pawnIo":null,"sources":null}"#
            )
            .unwrap(),
            ServiceStatus::new(ServiceState::NotInstalled, None)
        );
    }

    #[test]
    fn service_status_serializes_with_pawn_io_and_sources() {
        let status = ServiceStatus {
            state: ServiceState::Connected,
            detail: None,
            pawn_io: Some(PawnIoStatus::RebootPending),
            sources: Some(ServiceSources {
                active_modules: vec!["cpu".to_owned(), "storage".to_owned()],
                smart_disabled_drives: vec!["storage/device-a".to_owned()],
                reconfiguration: Reconfiguration::Pending,
                smart_blocked_by: vec!["storage/device-b".to_owned()],
            }),
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "state": "connected",
                "detail": null,
                "pawnIo": "rebootPending",
                "sources": {
                    "activeModules": ["cpu", "storage"],
                    "smartDisabledDrives": ["storage/device-a"],
                    "reconfiguration": "pending",
                    "smartBlockedBy": ["storage/device-b"],
                },
            })
        );
        assert_eq!(
            serde_json::from_value::<ServiceStatus>(json).unwrap(),
            status
        );
    }

    #[test]
    fn wire_strings_map_to_statuses() {
        for (wire, expected) in [
            ("ok", PawnIoStatus::Ok),
            ("missing", PawnIoStatus::Missing),
            ("unavailable", PawnIoStatus::Unavailable),
            ("unknown", PawnIoStatus::Unknown),
            ("rebootPending", PawnIoStatus::RebootPending),
            ("something-new", PawnIoStatus::Unknown),
        ] {
            assert_eq!(PawnIoStatus::from_wire(wire), expected, "{wire}");
        }
        assert_eq!(
            Reconfiguration::from_wire("pending"),
            Reconfiguration::Pending
        );
        assert_eq!(
            Reconfiguration::from_wire("failed"),
            Reconfiguration::Failed
        );
        assert_eq!(
            Reconfiguration::from_wire("applied"),
            Reconfiguration::Applied
        );
        assert_eq!(
            Reconfiguration::from_wire("other"),
            Reconfiguration::Applied
        );
    }
}
