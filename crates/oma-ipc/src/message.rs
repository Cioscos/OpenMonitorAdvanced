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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subscribe {
    pub interval_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireSchema {
    pub devices: Vec<WireDevice>,
    pub sensors: Vec<WireSensor>,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireError {
    pub code: String,
    pub message: String,
}
