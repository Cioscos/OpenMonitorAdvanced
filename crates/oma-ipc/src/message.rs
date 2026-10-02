//! Wire message types for the sensor IPC protocol (spec §6).
//!
//! These types are separate from `oma_core::model`'s camelCase UI-facing
//! types on purpose: the wire contract uses snake_case field names and a
//! different set of derives/attributes, and reusing the UI types directly
//! would let a rename on one side silently break the other (see the S2
//! spike report, `docs/superpowers/references/m4/s2-msgpack.md`).
//!
//! Every field is always present on the wire (`nil` for "absent"); no type
//! here uses `#[serde(skip_serializing_if)]`, and none uses
//! `#[serde(deny_unknown_fields)]` (unknown fields must be tolerated for
//! forward compatibility).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The one envelope every frame payload decodes to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum Message {
    Hello(Hello),
    Subscribe(Subscribe),
    Schema(WireSchema),
    Snapshot(WireSnapshot),
    Error(WireError),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol_version: u32,
    pub service_version: String,
    /// State of the PawnIO driver: `"ok"`, `"missing"`, `"unavailable"`, `"unknown"` or
    /// `"rebootPending"`. The decoder alone is lenient: a protocol v1 service sends no such
    /// key, and its `Hello` must still decode so the version check can report it as
    /// incompatible. Encoders always write the key.
    #[serde(default = "unknown_pawn_io")]
    pub pawn_io: String,
}

fn unknown_pawn_io() -> String {
    "unknown".to_owned()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subscribe {
    pub interval_ms: u32,
    /// Service modules this client does not want (names from [`crate::MODULES`]).
    pub disabled_modules: Vec<String>,
    /// [`crate::drive_key`]s of the disks whose SMART this client does not want.
    pub smart_disabled_drives: Vec<String>,
    /// [`crate::drive_key`]s of the disks that are off by default and that this client wants on.
    pub smart_enabled_drives: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireSchema {
    pub devices: Vec<WireDevice>,
    pub sensors: Vec<WireSensor>,
    pub service: WireServiceState,
}

/// The service's effective configuration, global to all its clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireServiceState {
    /// Modules that are on ([`crate::MODULES`] names).
    pub active_modules: Vec<String>,
    /// [`crate::drive_key`]s of the disks whose SMART is off.
    pub smart_disabled_drives: Vec<String>,
    /// `"applied"`, `"pending"` or `"failed"`.
    pub reconfiguration: String,
    /// One entry per physical drive the service enumerates, in `physical_drive` order.
    pub drives: Vec<WireDrive>,
}

/// A physical drive as the service sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireDrive {
    pub physical_drive: u32,
    /// The drive's [`crate::drive_key`], `nil` when it has no model or no serial.
    pub key: Option<String>,
    pub model: Option<String>,
    /// `"active"`, `"standby"`, `"unknown"`, `"smartOff"` or `"noMedia"`.
    pub state: String,
    /// Whether this drive keeps the SMART gate closed for all drives.
    pub blocks_smart: bool,
}

impl Default for WireServiceState {
    /// Nothing switched off and nothing in progress: every module on, `"applied"`.
    fn default() -> Self {
        Self {
            active_modules: crate::MODULES.map(str::to_owned).to_vec(),
            smart_disabled_drives: Vec::new(),
            reconfiguration: "applied".to_owned(),
            drives: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireDevice {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub vendor: Option<String>,
    pub properties: BTreeMap<String, String>,
    pub hint: Option<IdentityHint>,
}

/// Tagged union of device-identity hints, adjacently tagged with
/// `kind`/`value` (not `type`/`body`, to avoid colliding with
/// `WireDevice.kind`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum IdentityHint {
    Cpu {
        index: u32,
    },
    Storage {
        physical_drive: u32,
        model: Option<String>,
        serial: Option<String>,
    },
    Memory {},
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireSensor {
    pub device_id: String,
    pub kind: String,
    pub name: String,
    pub unit: String,
    pub label_key: String,
    pub label_arg: Option<String>,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireSnapshot {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub values: Vec<Option<f64>>,
    /// Same length and order as `values`: the value is kept from an earlier measurement.
    /// Always `false` for an absent value.
    pub held: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireError {
    pub code: String,
    pub message: String,
}
