//! Sensor IPC protocol: wire message types and MessagePack framing.
//!
//! This crate is portable (no Windows-specific code): it only defines the
//! `oma-service` <-> `oma-app` wire contract (spec §6) and the encoder /
//! decoder used on both ends of the named pipe.

mod frame;
mod message;
mod status;

pub use frame::{decode_payload, encode_frame, encode_payload, FrameDecoder};
pub use message::{
    Hello, IdentityHint, Message, Subscribe, WireDevice, WireError, WireSchema, WireSensor,
    WireSnapshot,
};
pub use status::{ServiceDetail, ServiceState, ServiceStatus};

/// Current sensor IPC protocol version, sent in [`Hello::protocol_version`].
pub const PROTOCOL_VERSION: u32 = 1;

/// Name of the sensor named pipe.
pub const PIPE_NAME: &str = "OpenMonitorAdvanced.Sensors.v1";

/// Maximum size, in bytes, of a single frame's MessagePack payload.
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Minimum accepted [`Subscribe::interval_ms`].
pub const MIN_INTERVAL_MS: u32 = 250;

/// Maximum accepted [`Subscribe::interval_ms`].
pub const MAX_INTERVAL_MS: u32 = 5_000;

/// Errors from encoding, decoding, or framing sensor IPC messages.
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("frame of {0} bytes exceeds the maximum of {MAX_FRAME_BYTES} bytes")]
    FrameTooLarge(u32),
    #[error("failed to encode message: {0}")]
    Encode(String),
    #[error("failed to decode message: {0}")]
    Decode(String),
}
