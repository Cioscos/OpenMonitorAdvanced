//! The Windows version for the sensor report (spec §3.1).
//!
//! `RtlGetVersion` answers the real version whatever the executable's
//! manifest says, unlike `GetVersionExW`.

use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

// Five `u32` fields and `szCSDVersion: [u16; 128]`, as in the Windows SDK.
const _: () = assert!(std::mem::size_of::<OSVERSIONINFOW>() == 276);

/// `major.minor.build`, e.g. `10.0.26300`.
pub fn format_version(major: u32, minor: u32, build: u32) -> String {
    format!("{major}.{minor}.{build}")
}

/// The running Windows version as `major.minor.build`; `None` if the call fails.
pub fn os_version() -> Option<String> {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid, writable OSVERSIONINFOW whose size field is
    // set as the call requires; it outlives the call and nothing else aliases it.
    let status = unsafe { RtlGetVersion(&mut info) };
    status
        .is_ok()
        .then(|| format_version(info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_version_joins_parts() {
        assert_eq!(format_version(10, 0, 26300), "10.0.26300");
        assert_eq!(format_version(6, 3, 9600), "6.3.9600");
    }

    #[test]
    fn os_version_reads_this_system() {
        let version = os_version().expect("RtlGetVersion");
        let parts: Vec<u32> = version
            .split('.')
            .map(|part| part.parse().expect("number"))
            .collect();
        assert_eq!(parts.len(), 3, "{version}");
        assert!(parts[0] >= 10, "{version}");
    }
}
