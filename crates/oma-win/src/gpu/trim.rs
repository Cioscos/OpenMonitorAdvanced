//! Releases the pages of a loaded module's `.data` section from the process working set.
//!
//! The DriverStore copy of nvml.dll writes its whole `.data` section (19.4 MB with driver
//! 617.14) during `nvmlInit_v2` and then keeps using only a small part of it. The written
//! copy-on-write pages count as private working set for the whole process lifetime (+19.3 MB
//! measured), which alone would eat most of the tray budget. `VirtualUnlock` on a range that
//! is not locked removes its pages from the working set (documented side effect; the call
//! then fails with `ERROR_NOT_LOCKED`, which is the expected outcome here). Contents are
//! preserved: the pages move to the standby/modified lists and fault back in when touched
//! again (+0.5 MB measured over 600 polls).

use windows::Win32::Foundation::{ERROR_NOT_LOCKED, HMODULE};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Memory::VirtualUnlock;
use windows::Win32::System::ProcessStatus::EnumProcessModules;
use windows::Win32::System::Threading::GetCurrentProcess;

/// Bytes read at a module base to parse its headers: the DOS, NT and section headers of a
/// mapped image always lie in its first page.
const HEADER_PAGE: usize = 4096;
const SECTION_HEADER_LEN: usize = 40;

/// Position of a section inside a mapped image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Section {
    pub rva: u32,
    pub virtual_size: u32,
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    let b = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let b = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// True when the 8-byte, NUL-padded section name field holds exactly `name`.
fn section_name_is(field: &[u8], name: &[u8]) -> bool {
    field.len() == 8
        && name.len() <= 8
        && field[..name.len()] == *name
        && field[name.len()..].iter().all(|&b| b == 0)
}

/// Finds section `name` (e.g. `b".data"`) in the PE headers at the start of `image`.
pub(crate) fn find_section(image: &[u8], name: &[u8]) -> Result<Section, String> {
    if image.get(..2) != Some(b"MZ".as_slice()) {
        return Err("missing MZ signature".to_owned());
    }
    let nt = u32_at(image, 0x3C).ok_or("truncated DOS header")? as usize;
    if image.get(nt..nt + 4) != Some(b"PE\0\0".as_slice()) {
        return Err("missing PE signature".to_owned());
    }
    let sections = u16_at(image, nt + 6).ok_or("truncated file header")? as usize;
    let optional_len = u16_at(image, nt + 20).ok_or("truncated file header")? as usize;
    let table = nt + 24 + optional_len;
    for i in 0..sections {
        let start = table + i * SECTION_HEADER_LEN;
        let header = image
            .get(start..start + SECTION_HEADER_LEN)
            .ok_or("section table extends past the header page")?;
        if section_name_is(&header[..8], name) {
            return Ok(Section {
                virtual_size: u32_at(header, 8).ok_or("truncated section header")?,
                rva: u32_at(header, 12).ok_or("truncated section header")?,
            });
        }
    }
    Err(format!("no {} section", String::from_utf8_lossy(name)))
}

/// True when `path` contains `path_contains` and its file name is `file_name` (both
/// case-insensitive).
pub(crate) fn module_matches(path: &str, path_contains: &str, file_name: &str) -> bool {
    let path = path.to_lowercase();
    let name = path.rsplit(['\\', '/']).next().unwrap_or_default();
    path.contains(&path_contains.to_lowercase()) && name == file_name.to_lowercase()
}

fn module_path(module: HMODULE) -> String {
    let mut buffer = [0u16; 1024];
    // SAFETY: `buffer` is writable for its whole length; `module` comes from EnumProcessModules.
    let len = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
    String::from_utf16_lossy(&buffer[..len.min(buffer.len())])
}

fn loaded_modules() -> Result<Vec<HMODULE>, String> {
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing.
    let process = unsafe { GetCurrentProcess() };
    let mut modules = vec![HMODULE::default(); 512];
    loop {
        let capacity = std::mem::size_of_val(modules.as_slice()) as u32;
        let mut needed = 0u32;
        // SAFETY: `modules` is writable for `capacity` bytes.
        unsafe { EnumProcessModules(process, modules.as_mut_ptr(), capacity, &mut needed) }
            .map_err(|e| format!("EnumProcessModules: {e}"))?;
        let count = needed as usize / std::mem::size_of::<HMODULE>();
        if count <= modules.len() {
            modules.truncate(count);
            return Ok(modules);
        }
        modules.resize(count, HMODULE::default());
    }
}

/// Removes the `.data` pages of the loaded module whose full path contains `path_contains`
/// and whose file name is `file_name` from the working set. Returns the size of the released
/// range in bytes. The module must stay loaded for the rest of the process (decision D1).
pub(crate) fn release_module_data(path_contains: &str, file_name: &str) -> Result<usize, String> {
    let module = loaded_modules()?
        .into_iter()
        .find(|&m| module_matches(&module_path(m), path_contains, file_name))
        .ok_or_else(|| format!("no loaded {file_name} with {path_contains} in its path"))?;
    let base = module.0 as *const u8;
    // SAFETY: `module` is the base address of a mapped image; its first page holds the headers
    // and is always mapped readable.
    let headers = unsafe { std::slice::from_raw_parts(base, HEADER_PAGE) };
    let data = find_section(headers, b".data")?;
    let size = data.virtual_size as usize;
    // SAFETY: the range lies inside the mapped image; VirtualUnlock does not change its contents.
    let unlocked = unsafe { VirtualUnlock(base.add(data.rva as usize).cast(), size) };
    match unlocked {
        Ok(()) => Ok(size),
        Err(e) if e.code() == ERROR_NOT_LOCKED.to_hresult() => Ok(size),
        Err(e) => Err(format!("VirtualUnlock: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PE header page: DOS header, NT signature, file header, a 240-byte optional
    /// header and the given sections.
    fn image(sections: &[(&[u8], u32, u32)]) -> Vec<u8> {
        let mut page = vec![0u8; HEADER_PAGE];
        page[..2].copy_from_slice(b"MZ");
        let nt = 0x80usize;
        page[0x3C..0x40].copy_from_slice(&(nt as u32).to_le_bytes());
        page[nt..nt + 4].copy_from_slice(b"PE\0\0");
        page[nt + 6..nt + 8].copy_from_slice(&(sections.len() as u16).to_le_bytes());
        page[nt + 20..nt + 22].copy_from_slice(&240u16.to_le_bytes());
        let table = nt + 24 + 240;
        for (i, (name, virtual_size, rva)) in sections.iter().enumerate() {
            let h = table + i * SECTION_HEADER_LEN;
            page[h..h + name.len()].copy_from_slice(name);
            page[h + 8..h + 12].copy_from_slice(&virtual_size.to_le_bytes());
            page[h + 12..h + 16].copy_from_slice(&rva.to_le_bytes());
        }
        page
    }

    #[test]
    fn finds_the_data_section() {
        let page = image(&[
            (b".text", 0x1000, 0x1000),
            (b".rdata", 0x2000, 0x2000),
            (b".data", 0x0136_0000, 0x4000),
        ]);
        assert_eq!(
            find_section(&page, b".data"),
            Ok(Section {
                rva: 0x4000,
                virtual_size: 0x0136_0000
            })
        );
    }

    #[test]
    fn section_names_must_match_exactly() {
        let page = image(&[(b".data1", 0x10, 0x1000), (b".dat", 0x20, 0x2000)]);
        assert!(find_section(&page, b".data").is_err());
    }

    #[test]
    fn rejects_malformed_headers() {
        let mut page = image(&[(b".data", 0x10, 0x1000)]);
        page[0] = b'X';
        assert!(find_section(&page, b".data").is_err());
        let mut page = image(&[(b".data", 0x10, 0x1000)]);
        page[0x80] = b'X';
        assert!(find_section(&page, b".data").is_err());
        let mut page = image(&[(b".data", 0x10, 0x1000)]);
        page[0x3C..0x40].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        assert!(find_section(&page, b".data").is_err());
        // A section count that runs past the page must not read out of bounds.
        let mut page = image(&[(b".data", 0x10, 0x1000)]);
        page[0x86..0x88].copy_from_slice(&0xFFFFu16.to_le_bytes());
        page[0x80 + 24 + 240..0x80 + 24 + 240 + 5].copy_from_slice(b".text");
        assert!(find_section(&page, b".data").is_err());
    }

    #[test]
    fn matches_only_the_driver_store_copy() {
        let store = r"C:\WINDOWS\system32\DriverStore\FileRepository\nvmdi.inf_amd64_3990df11758b8744\nvml.dll";
        assert!(module_matches(store, "DriverStore", "nvml.dll"));
        assert!(module_matches(
            &store.to_uppercase(),
            "driverstore",
            "NVML.DLL"
        ));
        assert!(!module_matches(
            r"C:\WINDOWS\SYSTEM32\nvml.dll",
            "DriverStore",
            "nvml.dll"
        ));
        assert!(!module_matches(
            &format!("{store}.bak"),
            "DriverStore",
            "nvml.dll"
        ));
        assert!(!module_matches(
            r"C:\WINDOWS\system32\DriverStore\FileRepository\x\xnvml.dll",
            "DriverStore",
            "nvml.dll"
        ));
    }

    #[test]
    fn releases_the_data_section_of_a_loaded_module() {
        // kernel32.dll is loaded in every process and has a .data section.
        let released = release_module_data("system32", "kernel32.dll").expect("kernel32 .data");
        assert!(released > 0);
    }

    #[test]
    fn missing_module_is_an_error() {
        assert!(release_module_data("DriverStore", "oma-not-loaded.dll").is_err());
    }
}
