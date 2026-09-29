//! The wire key of a physical disk (spec M5 §2.8, ruling P8).
//!
//! `sha256(trim(model) + "\0" + trim(serial))` in lowercase hexadecimal, computed on the
//! texts of the disk's storage device descriptor. The service computes the same value in
//! `Sensors/DriveKey.cs`; `protocol/fixtures/drive_key.json` is the vector both test suites
//! read, so the two implementations agree byte for byte.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

/// The key of the disk with this descriptor `model` and `serial`, or `None` when either is
/// empty after trimming (such a disk cannot be told apart, so it has no key).
///
/// Trimming is [`str::trim`] (Unicode `White_Space`); the .NET side trims the same set.
pub fn drive_key(model: &str, serial: &str) -> Option<String> {
    let model = model.trim();
    let serial = serial.trim();
    if model.is_empty() || serial.is_empty() {
        return None;
    }

    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    hasher.update([0u8]);
    hasher.update(serial.as_bytes());
    let digest = hasher.finalize();

    let mut key = String::with_capacity(64);
    for byte in digest {
        // Writing to a `String` cannot fail.
        let _ = write!(key, "{byte:02x}");
    }
    Some(key)
}
