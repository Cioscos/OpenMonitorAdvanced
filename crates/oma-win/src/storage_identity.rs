//! Persistent identities. Raw serial numbers, disk GUIDs, MBR signatures and
//! PnP instance ids never leave this module: only their SHA-256 hashes do.
//!
//! Disk identity is a fallback chain (serial, GPT disk id, MBR signature plus
//! size, PnP instance id). A tier is used only when its value is unique among
//! the disks of this machine, so two disks never merge under one id.
use std::collections::{BTreeMap, HashMap};

use sha2::{Digest, Sha256};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE,
    DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    SP_DEVINFO_DATA,
};
use windows::Win32::Storage::FileSystem::GetVolumeNameForVolumeMountPointW;
use windows::Win32::System::Ioctl::{
    StorageDeviceProperty, DISK_GEOMETRY_EX, DRIVE_LAYOUT_INFORMATION_EX,
    DRIVE_LAYOUT_INFORMATION_GPT, DRIVE_LAYOUT_INFORMATION_MBR, GUID_DEVINTERFACE_DISK,
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_DISK_GET_DRIVE_LAYOUT_EX, PARTITION_STYLE_GPT,
    PARTITION_STYLE_MBR,
};

use crate::storage_ioctl::{le_i64, le_u32, PhysicalDrive};

/// DRIVE_LAYOUT_INFORMATION_EX: `PartitionStyle` at 0, the MBR/GPT union at 8.
const LAYOUT_STYLE: usize = 0;
const LAYOUT_UNION: usize = 8;
/// DISK_GEOMETRY_EX: `DiskSize` follows the 24-byte DISK_GEOMETRY.
const GEOMETRY_DISK_SIZE: usize = 24;

const _: () = assert!(std::mem::offset_of!(DRIVE_LAYOUT_INFORMATION_EX, PartitionStyle) == 0);
const _: () = assert!(std::mem::offset_of!(DRIVE_LAYOUT_INFORMATION_EX, Anonymous) == 8);
const _: () = assert!(std::mem::offset_of!(DRIVE_LAYOUT_INFORMATION_GPT, DiskId) == 0);
const _: () = assert!(size_of::<windows::core::GUID>() == 16);
const _: () = assert!(std::mem::offset_of!(DRIVE_LAYOUT_INFORMATION_MBR, Signature) == 0);
const _: () = assert!(std::mem::offset_of!(DISK_GEOMETRY_EX, DiskSize) == 24);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<SP_DEVICE_INTERFACE_DATA>() == 32);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<SP_DEVINFO_DATA>() == 32);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() == 8);
const _: () = assert!(std::mem::offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath) == 4);

/// Bytes of the NUL-terminated string whose offset is stored at `field`.
fn descriptor_field(bytes: &[u8], field: usize) -> Option<&[u8]> {
    let offset = le_u32(bytes, field)? as usize;
    if offset < 36 {
        return None;
    }
    let tail = bytes.get(offset..)?;
    let end = tail.iter().position(|&b| b == 0)?;
    Some(&tail[..end])
}

fn descriptor_text(bytes: &[u8], field: usize) -> Option<&str> {
    let text = std::str::from_utf8(descriptor_field(bytes, field)?)
        .ok()?
        .trim();
    (!text.is_empty()).then_some(text)
}

/// The serial as hashed: the trimmed text when it is UTF-8 (ids unchanged
/// since M1), otherwise the raw bytes without surrounding ASCII whitespace.
fn descriptor_serial(bytes: &[u8]) -> Option<&[u8]> {
    let raw = descriptor_field(bytes, 24)?;
    let serial = match std::str::from_utf8(raw) {
        Ok(text) => text.trim().as_bytes(),
        Err(_) => raw.trim_ascii(),
    };
    (!serial.is_empty()).then_some(serial)
}

fn identity_from_descriptor(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 36 {
        return None;
    }
    let serial = descriptor_serial(bytes)?;
    let vendor = descriptor_text(bytes, 12).unwrap_or("");
    let model = descriptor_text(bytes, 16).unwrap_or("");
    let mut hasher = Sha256::new();
    hasher.update(vendor.as_bytes());
    hasher.update([0]);
    hasher.update(model.as_bytes());
    hasher.update([0]);
    hasher.update(serial);
    Some(format!("storage/device-{:x}", hasher.finalize()))
}

/// GPT tier: hash of the disk GUID stored in the GPT header.
fn gpt_identity(layout: &[u8]) -> Option<String> {
    if le_u32(layout, LAYOUT_STYLE)? != PARTITION_STYLE_GPT.0 as u32 {
        return None;
    }
    let disk_id = layout.get(LAYOUT_UNION..LAYOUT_UNION + 16)?;
    if disk_id.iter().all(|&b| b == 0) {
        return None;
    }
    Some(format!("storage/gpt-{:x}", Sha256::digest(disk_id)))
}

/// MBR tier: hash of the 32-bit disk signature and the disk size. Signature 0
/// means "never initialised" and is not an identity.
fn mbr_identity(layout: &[u8], disk_size: Option<u64>) -> Option<String> {
    if le_u32(layout, LAYOUT_STYLE)? != PARTITION_STYLE_MBR.0 as u32 {
        return None;
    }
    let signature = le_u32(layout, LAYOUT_UNION).filter(|&s| s != 0)?;
    let size = disk_size.filter(|&s| s > 0)?;
    let mut hasher = Sha256::new();
    hasher.update(signature.to_le_bytes());
    hasher.update(size.to_le_bytes());
    Some(format!("storage/mbr-{:x}", hasher.finalize()))
}

/// `DiskSize` of a DISK_GEOMETRY_EX, in bytes.
fn disk_size(geometry: &[u8]) -> Option<u64> {
    le_i64(geometry, GEOMETRY_DISK_SIZE).and_then(|size| u64::try_from(size).ok())
}

/// PnP tier: hash of the device instance id (case-insensitive in Windows).
fn pnp_identity(instance_id: &str) -> Option<String> {
    let id = instance_id.trim().to_uppercase();
    (!id.is_empty()).then(|| format!("storage/pnp-{:x}", Sha256::digest(id.as_bytes())))
}

/// Serial tier: `storage/device-<sha256(vendor\0model\0serial)>`.
pub(crate) fn disk_identity(index: u32) -> Option<String> {
    let bytes = PhysicalDrive::open(index)?.query_property(StorageDeviceProperty, 65_536)?;
    identity_from_descriptor(&bytes)
}

/// Identity tiers, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdentityTier {
    /// Travels with the drive.
    Serial,
    /// Stored on the media; duplicated by byte-for-byte clones.
    Gpt,
    /// 32-bit signature plus disk size; duplicated by clones.
    Mbr,
    /// Bound to the port or slot: changes if the disk is moved.
    Pnp,
}

impl IdentityTier {
    const ALL: [IdentityTier; 4] = [
        IdentityTier::Serial,
        IdentityTier::Gpt,
        IdentityTier::Mbr,
        IdentityTier::Pnp,
    ];

    fn name(self) -> &'static str {
        match self {
            IdentityTier::Serial => "serial",
            IdentityTier::Gpt => "gpt",
            IdentityTier::Mbr => "mbr",
            IdentityTier::Pnp => "pnp",
        }
    }
}

/// Full candidate ids (`storage/<tier prefix>-<sha256 hex>`) of one disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DiskIdentityCandidates {
    pub serial: Option<String>,
    pub gpt: Option<String>,
    pub mbr: Option<String>,
    pub pnp: Option<String>,
}

impl DiskIdentityCandidates {
    fn get(&self, tier: IdentityTier) -> Option<&str> {
        match tier {
            IdentityTier::Serial => self.serial.as_deref(),
            IdentityTier::Gpt => self.gpt.as_deref(),
            IdentityTier::Mbr => self.mbr.as_deref(),
            IdentityTier::Pnp => self.pnp.as_deref(),
        }
    }
}

/// Reads every identity tier of `\\.\PhysicalDrive<index>`; a tier that
/// cannot be read is `None`.
pub(crate) fn disk_identity_candidates(index: u32) -> DiskIdentityCandidates {
    let drive = PhysicalDrive::open(index);
    let layout = drive
        .as_ref()
        .and_then(|d| d.ioctl(IOCTL_DISK_GET_DRIVE_LAYOUT_EX, None, 65_536));
    let size = drive
        .as_ref()
        .and_then(|d| d.ioctl(IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, None, 256))
        .and_then(|geometry| disk_size(&geometry));
    DiskIdentityCandidates {
        serial: disk_identity(index),
        gpt: layout.as_deref().and_then(gpt_identity),
        mbr: layout.as_deref().and_then(|l| mbr_identity(l, size)),
        pnp: pnp_instance_id(index).as_deref().and_then(pnp_identity),
    }
}

/// Assigned ids plus, per disk, why each tier before the chosen one (or every
/// tier, for an omitted disk) could not be used.
type Resolution = (
    BTreeMap<u32, (String, IdentityTier)>,
    BTreeMap<u32, Vec<String>>,
);

fn resolve(candidates: &BTreeMap<u32, DiskIdentityCandidates>) -> Resolution {
    let mut assigned = BTreeMap::new();
    let mut causes: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for tier in IdentityTier::ALL {
        // Ambiguity is judged against every disk of the machine, identified
        // or not: a value shared with any other disk is not an identity.
        let mut counts = HashMap::<&str, usize>::new();
        for c in candidates.values() {
            if let Some(id) = c.get(tier) {
                *counts.entry(id).or_default() += 1;
            }
        }
        for (&index, c) in candidates {
            if assigned.contains_key(&index) {
                continue;
            }
            match c.get(tier) {
                None => causes
                    .entry(index)
                    .or_default()
                    .push(format!("{}: unavailable", tier.name())),
                Some(id) if counts[id] > 1 => causes
                    .entry(index)
                    .or_default()
                    .push(format!("{}: shared with another disk", tier.name())),
                Some(id) => {
                    assigned.insert(index, (id.to_owned(), tier));
                }
            }
        }
    }
    (assigned, causes)
}

/// Picks for every disk the strongest tier whose value is unique on this
/// machine. Disks missing from the result have no usable identity and must
/// be omitted; the cause is logged per tier.
pub(crate) fn assign_disk_ids(
    candidates: &BTreeMap<u32, DiskIdentityCandidates>,
) -> BTreeMap<u32, (String, IdentityTier)> {
    let (assigned, causes) = resolve(candidates);
    for (index, reasons) in &causes {
        let reasons = reasons.join("; ");
        match assigned.get(index) {
            Some((_, tier)) => tracing::info!(
                index,
                tier = tier.name(),
                skipped = %reasons,
                "disk identified by a fallback identity"
            ),
            None => tracing::warn!(
                index,
                causes = %reasons,
                "disk has no unique persistent identity; omitted"
            ),
        }
    }
    assigned
}

/// Owns a SetupAPI device information set.
struct DeviceInfoSet(HDEVINFO);

impl Drop for DeviceInfoSet {
    fn drop(&mut self) {
        // SAFETY: sole owner of the set, never used after drop.
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

/// Device path and device element of one disk interface.
fn interface_detail(
    set: &DeviceInfoSet,
    interface: &SP_DEVICE_INTERFACE_DATA,
) -> Option<(String, SP_DEVINFO_DATA)> {
    let mut required = 0u32;
    // SAFETY: size query with no output buffer; the set and the interface
    // data are valid. It fails with ERROR_INSUFFICIENT_BUFFER by design.
    let _ = unsafe {
        SetupDiGetDeviceInterfaceDetailW(set.0, interface, None, 0, Some(&mut required), None)
    };
    let required = required as usize;
    let path_offset = std::mem::offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
    if required <= path_offset {
        return None;
    }
    // u32 storage keeps the variable-length structure 4-byte aligned.
    let mut buffer = vec![0u32; required.div_ceil(4)];
    let detail = buffer
        .as_mut_ptr()
        .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    // SAFETY: the buffer holds at least `required` bytes, is suitably aligned,
    // and cbSize must be the size of the fixed part, as documented.
    unsafe {
        (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
    }
    let mut device = SP_DEVINFO_DATA {
        cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
        ..Default::default()
    };
    // SAFETY: `detail` points to `required` writable bytes; `device` has its
    // cbSize set; the set and the interface data are valid.
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            set.0,
            interface,
            Some(detail),
            required as u32,
            None,
            Some(&mut device),
        )
    }
    .ok()?;
    // SAFETY: the path lies within the `required` bytes of the buffer, right
    // after cbSize, as UTF-16 code units.
    let path = unsafe {
        std::slice::from_raw_parts(
            buffer.as_ptr().cast::<u8>().add(path_offset).cast::<u16>(),
            (required - path_offset) / 2,
        )
    };
    let end = path.iter().position(|&c| c == 0).unwrap_or(path.len());
    Some((String::from_utf16_lossy(&path[..end]), device))
}

fn instance_id(set: &DeviceInfoSet, device: &SP_DEVINFO_DATA) -> Option<String> {
    // Device instance ids are at most 200 characters (MAX_DEVICE_ID_LEN).
    let mut buffer = [0u16; 512];
    // SAFETY: `device` belongs to `set`; the buffer is writable for its length.
    unsafe { SetupDiGetDeviceInstanceIdW(set.0, device, Some(&mut buffer), None) }.ok()?;
    let end = buffer.iter().position(|&c| c == 0)?;
    Some(String::from_utf16_lossy(&buffer[..end]))
}

/// PnP device instance id of `\\.\PhysicalDrive<index>`. The disk interface
/// path is not the PhysicalDrive path, so each interface is opened and matched
/// by its disk number.
fn pnp_instance_id(index: u32) -> Option<String> {
    // SAFETY: valid interface class GUID, no enumerator, no parent window.
    let set = unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVINTERFACE_DISK),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    }
    .ok()?;
    let set = DeviceInfoSet(set);
    for member in 0u32.. {
        let mut interface = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: valid set, class GUID and interface data with cbSize set.
        // Failure (ERROR_NO_MORE_ITEMS) ends the enumeration.
        let more = unsafe {
            SetupDiEnumDeviceInterfaces(
                set.0,
                None,
                &GUID_DEVINTERFACE_DISK,
                member,
                &mut interface,
            )
        };
        if more.is_err() {
            return None;
        }
        let Some((path, device)) = interface_detail(&set, &interface) else {
            continue;
        };
        if PhysicalDrive::open_path(&path).and_then(|d| d.disk_number()) == Some(index) {
            return instance_id(&set, &device);
        }
    }
    None
}

pub(crate) fn volume_identity(letter: &str) -> Option<String> {
    let mut buffer = [0u16; 64];
    let root = HSTRING::from(format!("{letter}\\"));
    // SAFETY: valid mount point and output buffer.
    unsafe { GetVolumeNameForVolumeMountPointW(&root, &mut buffer) }.ok()?;
    let end = buffer.iter().position(|&v| v == 0)?;
    let name = String::from_utf16_lossy(&buffer[..end]).to_ascii_lowercase();
    Some(
        name.strip_prefix(r"\\?\volume{")?
            .strip_suffix("}\\")?
            .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn descriptor(serial: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 36];
        bytes[24..28].copy_from_slice(&36u32.to_le_bytes());
        bytes.extend_from_slice(serial);
        bytes.push(0);
        bytes
    }

    #[test]
    fn identity_depends_on_hardware_not_disk_number() {
        let serial = descriptor(b"serial-a");
        assert_eq!(
            identity_from_descriptor(&serial),
            identity_from_descriptor(&serial)
        );
        assert_ne!(
            identity_from_descriptor(&serial),
            identity_from_descriptor(&descriptor(b"serial-b"))
        );
        assert!(!identity_from_descriptor(&serial)
            .unwrap()
            .contains("serial-a"));
    }

    #[test]
    fn serial_ids_are_unchanged_since_m1() {
        // sha256("\0\0serial-a"): no vendor, no model, as computed by M1.
        assert_eq!(
            identity_from_descriptor(&descriptor(b"  serial-a ")).as_deref(),
            Some("storage/device-1963b90ffdd2185fc60ac5ebd1a877aa8bca8a2098d890bd4ffcb4b5d9ec4cee")
        );
    }

    #[test]
    fn non_utf8_serial_is_hashed_raw() {
        let raw = identity_from_descriptor(&descriptor(b" \xFF\xFEserial ")).expect("raw serial");
        assert!(raw.starts_with("storage/device-"));
        assert_ne!(
            Some(raw),
            identity_from_descriptor(&descriptor(b"\xFF\xFDserial"))
        );
    }

    #[test]
    fn missing_or_malformed_serial_has_no_identity() {
        assert!(identity_from_descriptor(&descriptor(b"")).is_none());
        assert!(identity_from_descriptor(&descriptor(b"   ")).is_none());
        assert!(identity_from_descriptor(&[0; 36]).is_none());
        let mut bytes = descriptor(b"abc");
        bytes[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(identity_from_descriptor(&bytes).is_none());
    }

    fn layout(style: u32, union: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 48];
        bytes[0..4].copy_from_slice(&style.to_le_bytes());
        bytes[8..8 + union.len()].copy_from_slice(union);
        bytes
    }

    const GUID_A: [u8; 16] = [
        0x18, 0xFE, 0x37, 0xB9, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    ];

    #[test]
    fn gpt_identity_hashes_the_disk_guid() {
        let gpt = gpt_identity(&layout(1, &GUID_A)).expect("gpt id");
        assert!(gpt.starts_with("storage/gpt-"));
        assert_eq!(gpt.len(), "storage/gpt-".len() + 64);
        let mut other = GUID_A;
        other[15] ^= 1;
        assert_ne!(Some(gpt), gpt_identity(&layout(1, &other)));
        assert_eq!(gpt_identity(&layout(1, &[0; 16])), None);
        assert_eq!(gpt_identity(&layout(0, &GUID_A)), None); // MBR disk
        assert_eq!(gpt_identity(&layout(2, &GUID_A)), None); // RAW disk
        assert_eq!(gpt_identity(&layout(1, &GUID_A)[..20]), None);
    }

    #[test]
    fn mbr_identity_needs_signature_and_size() {
        let signature = 0x1234_ABCDu32.to_le_bytes();
        let mbr = mbr_identity(&layout(0, &signature), Some(512 << 30)).expect("mbr id");
        assert!(mbr.starts_with("storage/mbr-"));
        assert_ne!(
            Some(mbr),
            mbr_identity(&layout(0, &signature), Some(256 << 30)),
            "same signature, different size"
        );
        assert_eq!(mbr_identity(&layout(0, &signature), None), None);
        assert_eq!(mbr_identity(&layout(0, &signature), Some(0)), None);
        assert_eq!(mbr_identity(&layout(0, &[0; 4]), Some(512 << 30)), None);
        assert_eq!(mbr_identity(&layout(1, &signature), Some(512 << 30)), None);
    }

    #[test]
    fn disk_size_from_geometry() {
        let mut geometry = vec![0u8; 40];
        geometry[24..32].copy_from_slice(&2_000_398_934_016i64.to_le_bytes());
        assert_eq!(disk_size(&geometry), Some(2_000_398_934_016));
        geometry[24..32].copy_from_slice(&(-1i64).to_le_bytes());
        assert_eq!(disk_size(&geometry), None);
        assert_eq!(disk_size(&geometry[..30]), None);
    }

    #[test]
    fn pnp_identity_is_case_insensitive_and_hashed() {
        let id = r"SCSI\DISK&VEN_NVME&PROD_SHPP41-2000GM\5&1EB1FAB4&0&000000";
        let hashed = pnp_identity(id).expect("pnp id");
        assert!(hashed.starts_with("storage/pnp-"));
        assert!(!hashed.to_uppercase().contains("SHPP41"));
        assert_eq!(pnp_identity(&id.to_lowercase()), Some(hashed));
        assert_eq!(pnp_identity("  "), None);
    }

    fn candidates(
        serial: Option<&str>,
        gpt: Option<&str>,
        mbr: Option<&str>,
        pnp: Option<&str>,
    ) -> DiskIdentityCandidates {
        DiskIdentityCandidates {
            serial: serial.map(str::to_owned),
            gpt: gpt.map(str::to_owned),
            mbr: mbr.map(str::to_owned),
            pnp: pnp.map(str::to_owned),
        }
    }

    fn ids(disks: &[(u32, DiskIdentityCandidates)]) -> BTreeMap<u32, (String, IdentityTier)> {
        assign_disk_ids(&disks.iter().cloned().collect())
    }

    #[test]
    fn unique_serials_win() {
        let result = ids(&[
            (0, candidates(Some("s0"), Some("g0"), None, Some("p0"))),
            (1, candidates(Some("s1"), Some("g1"), None, Some("p1"))),
        ]);
        assert_eq!(result[&0], ("s0".to_owned(), IdentityTier::Serial));
        assert_eq!(result[&1], ("s1".to_owned(), IdentityTier::Serial));
    }

    #[test]
    fn ambiguous_serials_fall_back_to_the_gpt_disk_id() {
        let result = ids(&[
            (0, candidates(Some("same"), Some("g0"), None, Some("p0"))),
            (1, candidates(Some("same"), Some("g1"), None, Some("p1"))),
        ]);
        assert_eq!(result[&0], ("g0".to_owned(), IdentityTier::Gpt));
        assert_eq!(result[&1], ("g1".to_owned(), IdentityTier::Gpt));
    }

    #[test]
    fn serial_less_disk_uses_the_gpt_disk_id() {
        let result = ids(&[
            (0, candidates(Some("s0"), Some("g0"), None, Some("p0"))),
            (1, candidates(None, Some("g1"), None, Some("p1"))),
        ]);
        assert_eq!(result[&0].1, IdentityTier::Serial);
        assert_eq!(result[&1], ("g1".to_owned(), IdentityTier::Gpt));
    }

    #[test]
    fn cloned_gpt_disks_fall_back_to_pnp() {
        // A byte-for-byte clone shares the GPT disk id with its source, even
        // when the source itself is identified by its serial.
        let result = ids(&[
            (0, candidates(Some("s0"), Some("g"), None, Some("p0"))),
            (1, candidates(None, Some("g"), None, Some("p1"))),
            (2, candidates(None, Some("g"), None, Some("p2"))),
        ]);
        assert_eq!(result[&0], ("s0".to_owned(), IdentityTier::Serial));
        assert_eq!(result[&1], ("p1".to_owned(), IdentityTier::Pnp));
        assert_eq!(result[&2], ("p2".to_owned(), IdentityTier::Pnp));
    }

    #[test]
    fn mbr_disk_without_serial_uses_the_mbr_signature() {
        let result = ids(&[(3, candidates(None, None, Some("m3"), Some("p3")))]);
        assert_eq!(result[&3], ("m3".to_owned(), IdentityTier::Mbr));
    }

    #[test]
    fn disk_is_omitted_only_when_every_tier_fails() {
        let result = ids(&[
            (0, candidates(None, None, None, None)),
            (1, candidates(None, Some("g"), None, Some("p"))),
            (2, candidates(None, Some("g"), None, Some("p"))),
            (3, candidates(None, None, None, Some("p3"))),
        ]);
        assert_eq!(result.keys().copied().collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn omission_causes_name_every_tier() {
        let (assigned, causes) = resolve(
            &[
                (0, candidates(None, Some("g"), None, None)),
                (1, candidates(Some("s1"), Some("g"), None, None)),
            ]
            .into_iter()
            .collect(),
        );
        assert!(!assigned.contains_key(&0));
        assert_eq!(
            causes[&0],
            vec![
                "serial: unavailable",
                "gpt: shared with another disk",
                "mbr: unavailable",
                "pnp: unavailable",
            ]
        );
        assert!(!causes.contains_key(&1), "disk 1 used its first tier");
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn every_disk_of_this_machine_has_an_identity() {
        let found: BTreeMap<u32, DiskIdentityCandidates> = (0..16)
            .filter(|&index| PhysicalDrive::open(index).is_some())
            .map(|index| (index, disk_identity_candidates(index)))
            .collect();
        assert!(!found.is_empty(), "no physical drive");
        for (index, c) in &found {
            println!("disk {index}: {c:?}");
            assert_eq!(c.serial, disk_identity(*index), "disk {index}");
            assert!(c.pnp.is_some(), "disk {index}: no PnP instance id");
            assert!(
                c.gpt.is_some() || c.mbr.is_some(),
                "disk {index}: no partition-table identity"
            );
        }
        let assigned = assign_disk_ids(&found);
        assert_eq!(assigned.len(), found.len(), "every disk identified");

        // The same disks without a serial: the fallback tiers alone still
        // identify every disk, uniquely.
        for strip in [1usize, 3] {
            let stripped: BTreeMap<u32, DiskIdentityCandidates> = found
                .iter()
                .map(|(&index, c)| {
                    let mut c = c.clone();
                    c.serial = None;
                    if strip == 3 {
                        c.gpt = None;
                        c.mbr = None;
                    }
                    (index, c)
                })
                .collect();
            let fallback = assign_disk_ids(&stripped);
            assert_eq!(fallback.len(), found.len(), "strip {strip}");
            let unique: BTreeSet<&String> = fallback.values().map(|(id, _)| id).collect();
            assert_eq!(unique.len(), fallback.len(), "strip {strip}");
            assert!(fallback
                .values()
                .all(|(_, tier)| *tier != IdentityTier::Serial));
        }
    }
}
