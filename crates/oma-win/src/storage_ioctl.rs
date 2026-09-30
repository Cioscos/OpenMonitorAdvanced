//! Metadata-only access to disks: `\\.\PhysicalDriveN` (or a disk interface
//! path) opened with zero desired access, so no administrator rights are
//! needed and no data is ever read or written.

use windows::core::{BOOL, HSTRING};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    PropertyStandardQuery, IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_DEVICE_NUMBER, STORAGE_PROPERTY_ID, STORAGE_PROPERTY_QUERY,
};
use windows::Win32::System::Power::GetDevicePowerState;
use windows::Win32::System::IO::DeviceIoControl;

/// `FILE_DEVICE_DISK` device type (winioctl.h); not exported by the `windows` crate.
const FILE_DEVICE_DISK: u32 = 7;

/// Size of a `STORAGE_PROPERTY_QUERY` for a standard query with no
/// additional parameters (`PropertyId` + `QueryType` + the 1-byte
/// `AdditionalParameters` placeholder, plus trailing padding).
const QUERY_SIZE: usize = size_of::<STORAGE_PROPERTY_QUERY>();

const _: () = assert!(QUERY_SIZE == 12);
const _: () = assert!(std::mem::offset_of!(STORAGE_PROPERTY_QUERY, PropertyId) == 0);
const _: () = assert!(std::mem::offset_of!(STORAGE_PROPERTY_QUERY, QueryType) == 4);
const _: () = assert!(size_of::<STORAGE_DEVICE_NUMBER>() == 12);
const _: () = assert!(std::mem::offset_of!(STORAGE_DEVICE_NUMBER, DeviceNumber) == 4);

/// Little-endian `u16` at `offset`, `None` past the end.
pub(crate) fn le_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

/// Little-endian `i16` at `offset`, `None` past the end.
pub(crate) fn le_i16(bytes: &[u8], offset: usize) -> Option<i16> {
    le_u16(bytes, offset).map(|v| v as i16)
}

/// Little-endian `u32` at `offset`, `None` past the end.
pub(crate) fn le_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// Little-endian `i64` at `offset`, `None` past the end.
pub(crate) fn le_i64(bytes: &[u8], offset: usize) -> Option<i64> {
    Some(i64::from_le_bytes(
        bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// Disk number from a `STORAGE_DEVICE_NUMBER`; `None` for devices that are not disks.
fn parse_disk_number(bytes: &[u8]) -> Option<u32> {
    if le_u32(bytes, 0)? != FILE_DEVICE_DISK {
        return None;
    }
    le_u32(bytes, 4)
}

/// An open disk handle, closed on drop.
pub(crate) struct PhysicalDrive(HANDLE);

impl PhysicalDrive {
    /// Opens `\\.\PhysicalDrive<index>`; `None` if it does not exist.
    pub(crate) fn open(index: u32) -> Option<Self> {
        Self::open_path(&format!(r"\\.\PhysicalDrive{index}"))
    }

    /// Opens a disk device path, such as a disk device interface path.
    pub(crate) fn open_path(path: &str) -> Option<Self> {
        let path = HSTRING::from(path);
        // SAFETY: valid NUL-terminated path; zero desired access only allows
        // metadata queries, never reads or writes of disk data.
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
        Some(Self(handle))
    }

    /// Sends `code` with an optional input buffer and returns the bytes the
    /// driver wrote (at most `capacity`); `None` if the request failed.
    pub(crate) fn ioctl(
        &self,
        code: u32,
        input: Option<&[u8]>,
        capacity: usize,
    ) -> Option<Vec<u8>> {
        self.ioctl_result(code, input, capacity).ok()
    }

    /// [`Self::ioctl`], keeping the Win32 error of a failed request: `None`
    /// inside the error when the failure is not a Win32 error, or when the
    /// driver claimed more bytes than the buffer holds.
    pub(crate) fn ioctl_result(
        &self,
        code: u32,
        input: Option<&[u8]>,
        capacity: usize,
    ) -> Result<Vec<u8>, Option<WIN32_ERROR>> {
        let mut out = vec![0u8; capacity];
        let mut returned = 0u32;
        // SAFETY: the handle is open; the input slice and the output buffer are
        // valid for the lengths passed; `returned` is a valid out-pointer.
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                input.map(|bytes| bytes.as_ptr().cast()),
                input.map_or(0, |bytes| bytes.len() as u32),
                Some(out.as_mut_ptr().cast()),
                out.len() as u32,
                Some(&mut returned),
                None,
            )
        }
        .map_err(|error| WIN32_ERROR::from_error(&error))?;
        let returned = returned as usize;
        if returned > out.len() {
            return Err(None);
        }
        out.truncate(returned);
        Ok(out)
    }

    /// `IOCTL_STORAGE_QUERY_PROPERTY` standard query for `property`.
    pub(crate) fn query_property(
        &self,
        property: STORAGE_PROPERTY_ID,
        capacity: usize,
    ) -> Option<Vec<u8>> {
        // A fully initialised byte buffer, rather than a view over
        // `STORAGE_PROPERTY_QUERY`'s trailing padding (not guaranteed
        // initialised by a struct literal with `..Default::default()`).
        let mut input = [0u8; QUERY_SIZE];
        input[0..4].copy_from_slice(&property.0.to_le_bytes());
        input[4..8].copy_from_slice(&PropertyStandardQuery.0.to_le_bytes());
        self.ioctl(IOCTL_STORAGE_QUERY_PROPERTY, Some(&input), capacity)
    }

    /// The N of `\\.\PhysicalDriveN` for this device; `None` if it is not a disk.
    pub(crate) fn disk_number(&self) -> Option<u32> {
        let bytes = self.ioctl(
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            None,
            size_of::<STORAGE_DEVICE_NUMBER>(),
        )?;
        parse_disk_number(&bytes)
    }

    /// `Some(false)` while the disk is spun down or in a low-power state,
    /// `None` if Windows cannot tell. Asking does not wake the disk.
    pub(crate) fn powered_on(&self) -> Option<bool> {
        let mut on = BOOL(0);
        // SAFETY: the handle is open and `on` is a valid out-pointer.
        let known = unsafe { GetDevicePowerState(self.0, &mut on) }.as_bool();
        known.then(|| on.as_bool())
    }
}

impl Drop for PhysicalDrive {
    fn drop(&mut self) {
        // SAFETY: this value is the sole owner of the handle, never used after drop.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_fields_within_bounds() {
        let bytes = [1, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(le_u32(&bytes, 0), Some(1));
        assert_eq!(le_i64(&bytes, 4), Some(-1));
        assert_eq!(le_u32(&bytes, 9), None);
        assert_eq!(le_i64(&bytes, 5), None);
        assert_eq!(le_u32(&bytes, usize::MAX), None);
        assert_eq!(le_u16(&bytes, 0), Some(1));
        assert_eq!(le_i16(&bytes, 4), Some(-1));
        assert_eq!(le_i16(&bytes, 11), None);
    }

    #[test]
    fn disk_number_only_for_disks() {
        let mut bytes = [0u8; 12];
        bytes[0..4].copy_from_slice(&FILE_DEVICE_DISK.to_le_bytes());
        bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(parse_disk_number(&bytes), Some(3));
        bytes[0..4].copy_from_slice(&2u32.to_le_bytes()); // CD-ROM
        assert_eq!(parse_disk_number(&bytes), None);
        assert_eq!(parse_disk_number(&bytes[..6]), None);
    }
}
