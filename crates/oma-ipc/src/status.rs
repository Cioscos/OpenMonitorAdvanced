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

/// The availability of a drive's SMART path as the service reports it. It is not the freshness
/// of a temperature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DriveState {
    /// The drive answers as awake, or does not need a power check.
    Active,
    /// `CHECK POWER MODE` answers standby.
    Standby,
    /// The drive needs a power check, Windows reports it on and it has no recent activity: the
    /// service sends it nothing. It may hide a standby the drive decided on its own.
    Idle,
    /// The drive needs a power check and no path answers.
    Unknown,
    /// The service does not query this drive.
    SmartOff,
    /// The driver reports no medium.
    NoMedia,
}

impl DriveState {
    /// The state named by `WireDrive::state`; an unrecognised value (a newer service, a corrupt
    /// value) reads as [`DriveState::Unknown`].
    pub fn from_wire(value: &str) -> Self {
        match value {
            "active" => Self::Active,
            "standby" => Self::Standby,
            "idle" => Self::Idle,
            "smartOff" => Self::SmartOff,
            "noMedia" => Self::NoMedia,
            _ => Self::Unknown,
        }
    }
}

/// A physical drive of the service, with the core device id of the disk it was matched to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDrive {
    pub physical_drive: u32,
    /// Core id of the disk this drive was matched to, `None` when it could not be identified.
    pub device_id: Option<String>,
    pub model: Option<String>,
    pub state: DriveState,
    /// Whether this drive keeps the SMART gate closed for all drives.
    pub blocks_smart: bool,
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
    /// The modules this app asked off in the request that `reconfiguration`
    /// refers to. A module among them that is still active is kept on by
    /// another client; the UI tells it apart only from this one status, since
    /// the settings (with `applyStatus`) reach it before the next status.
    pub requested_disabled_modules: Vec<String>,
    /// Core ids of the disks whose SMART is off.
    pub smart_disabled_drives: Vec<String>,
    pub reconfiguration: Reconfiguration,
    /// Every physical drive the service enumerates, in `physical_drive` order.
    pub drives: Vec<SourceDrive>,
}

/// What this app asks of the service and filters locally, by core ids: the
/// modules the user turned off, the disks whose SMART is off and the disks switched on. The Windows
/// link translates it for the wire (disks become [`crate::drive_key`]s); the
/// `svc` provider applies it to what the service sends, whether or not
/// another client keeps those sources on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceRequest {
    /// Names from [`crate::MODULES`].
    pub disabled_modules: Vec<String>,
    /// Core device ids of disks.
    pub smart_disabled_drives: Vec<String>,
    /// Core device ids of the disks that are off by default and that the user wants on.
    pub smart_enabled_drives: Vec<String>,
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
                requested_disabled_modules: vec!["psu".to_owned()],
                smart_disabled_drives: vec!["storage/device-a".to_owned()],
                reconfiguration: Reconfiguration::Pending,
                drives: vec![SourceDrive {
                    physical_drive: 1,
                    device_id: None,
                    model: Some("ST2000DM008-2UB102".to_owned()),
                    state: DriveState::Standby,
                    blocks_smart: true,
                }],
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
                    "requestedDisabledModules": ["psu"],
                    "smartDisabledDrives": ["storage/device-a"],
                    "reconfiguration": "pending",
                    "drives": [{
                        "physicalDrive": 1,
                        "deviceId": null,
                        "model": "ST2000DM008-2UB102",
                        "state": "standby",
                        "blocksSmart": true,
                    }],
                },
            })
        );
        assert_eq!(
            serde_json::from_value::<ServiceStatus>(json).unwrap(),
            status
        );
    }

    #[test]
    fn an_unknown_drive_state_reads_as_unknown() {
        assert_eq!(DriveState::from_wire("spinning"), DriveState::Unknown);
        for (wire, expected) in [
            ("active", DriveState::Active),
            ("standby", DriveState::Standby),
            ("unknown", DriveState::Unknown),
            ("smartOff", DriveState::SmartOff),
            ("noMedia", DriveState::NoMedia),
        ] {
            assert_eq!(DriveState::from_wire(wire), expected, "{wire}");
            assert_eq!(
                serde_json::to_value(expected).unwrap(),
                serde_json::json!(wire)
            );
        }
    }

    #[test]
    fn an_idle_drive_state_round_trips() {
        assert_eq!(DriveState::from_wire("idle"), DriveState::Idle);
        let json = serde_json::to_value(DriveState::Idle).unwrap();
        assert_eq!(json, serde_json::json!("idle"));
        assert_eq!(
            serde_json::from_value::<DriveState>(json).unwrap(),
            DriveState::Idle
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
