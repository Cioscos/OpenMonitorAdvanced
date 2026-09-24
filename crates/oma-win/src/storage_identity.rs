//! Persistent identities; raw serial numbers never leave this module.
use sha2::{Digest, Sha256};
use windows::core::HSTRING;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetVolumeNameForVolumeMountPointW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_PROPERTY_QUERY,
};
use windows::Win32::System::IO::DeviceIoControl;

fn descriptor_text(bytes: &[u8], field: usize) -> Option<&str> {
    let offset = u32::from_le_bytes(bytes.get(field..field + 4)?.try_into().ok()?) as usize;
    if offset < 36 {
        return None;
    }
    let tail = bytes.get(offset..)?;
    let end = tail.iter().position(|&b| b == 0)?;
    let text = std::str::from_utf8(&tail[..end]).ok()?.trim();
    (!text.is_empty()).then_some(text)
}

fn identity_from_descriptor(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 36 {
        return None;
    }
    let serial = descriptor_text(bytes, 24)?;
    let vendor = descriptor_text(bytes, 12).unwrap_or("");
    let model = descriptor_text(bytes, 16).unwrap_or("");
    let hash = Sha256::digest(format!("{vendor}\0{model}\0{serial}").as_bytes());
    Some(format!("storage/device-{hash:x}"))
}

pub(crate) fn disk_identity(index: u32) -> Option<String> {
    let path = HSTRING::from(format!(r"\\.\PhysicalDrive{index}"));
    // SAFETY: valid path; zero desired access only queries metadata, never writes.
    let handle = unsafe {
        CreateFileW(
            &path,
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .ok()?;
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        ..Default::default()
    };
    let mut bytes = vec![0u8; 65_536];
    let mut returned = 0u32;
    // SAFETY: both buffers and the returned-size pointer are valid for the call.
    let result = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some((&query as *const STORAGE_PROPERTY_QUERY).cast()),
            std::mem::size_of_val(&query) as u32,
            Some(bytes.as_mut_ptr().cast()),
            bytes.len() as u32,
            Some(&mut returned),
            None,
        )
    };
    // SAFETY: sole owned handle, no longer used after this call.
    unsafe {
        let _ = CloseHandle(handle);
    }
    result.ok()?;
    if returned as usize > bytes.len() {
        return None;
    }
    bytes.truncate(returned as usize);
    identity_from_descriptor(&bytes)
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
    fn missing_or_malformed_serial_has_no_identity() {
        assert!(identity_from_descriptor(&descriptor(b"")).is_none());
        assert!(identity_from_descriptor(&[0; 36]).is_none());
        let mut bytes = descriptor(b"abc");
        bytes[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(identity_from_descriptor(&bytes).is_none());
    }
}
