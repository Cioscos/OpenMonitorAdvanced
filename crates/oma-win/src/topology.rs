//! CPU topology for the stress test (M8a1): logical processors with their
//! core, efficiency class and cache domain, cache sizes, vendor and brand.
//!
//! The records come from `GetSystemCpuSetInformation` and
//! `GetLogicalProcessorInformationEx(RelationCache)`; the pure parsers walk
//! the variable-size records by their `Size` field and are tested on
//! hand-built buffers. `apic_id` stays `None` here: `oma-load` fills it in
//! (DA3).
//!
//! "Core N" (DA4) is the rank of the core among the distinct
//! `(group, CoreIndex)` pairs in ascending order, from 0. `CoreIndex` and
//! `LastLevelCacheIndex` are group-relative in Windows, so both are ranked
//! over (group, value): `LogicalCpu::core` and `LogicalCpu::llc` are unique
//! system-wide, `core_index` keeps the raw value.

use std::collections::BTreeSet;
use std::io;

use oma_ipc::load::{CacheSizes, LogicalCpu, Topology};
use windows::Win32::System::SystemInformation::{
    GetLogicalProcessorInformationEx, GetSystemCpuSetInformation, RelationCache,
    CACHE_RELATIONSHIP, GROUP_AFFINITY, SYSTEM_CPU_SET_INFORMATION,
    SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};
use windows::Win32::System::Threading::GetCurrentProcess;

use crate::overlay_pipe::os_error;

/// `CpuSetInformation`, the only value of `CPU_SET_INFORMATION_TYPE`.
const CPU_SET_TYPE: u32 = 0;
/// `Parked` is bit 0 of the flags byte.
const PARKED_BIT: u8 = 1;

const ERROR_INSUFFICIENT_BUFFER: i32 = 122;

/// One record of the CPU set buffer, before ranking.
struct RawCpu {
    group: u16,
    number: u8,
    core_index: u8,
    llc: u8,
    efficiency_class: u8,
    parked: bool,
}

/// Reads a `T` at `off` of `buf` if it fits (the buffer has no alignment promise).
fn read_at<T: Copy>(buf: &[u8], off: usize) -> Option<T> {
    let end = off.checked_add(size_of::<T>())?;
    if end > buf.len() {
        return None;
    }
    // SAFETY: the range `off..end` is inside `buf`; `T` is a plain-old-data FFI struct for which
    // every bit pattern is valid, and `read_unaligned` needs no alignment.
    Some(unsafe { buf.as_ptr().add(off).cast::<T>().read_unaligned() })
}

/// Rank of `key` among the distinct values of `keys`, from 0.
fn rank<K: Ord + Copy>(keys: &BTreeSet<K>, key: K) -> u32 {
    keys.range(..key).count() as u32
}

/// Walks the CPU set records of `buf` and returns the logical processors
/// sorted by (group, number), with the global `index`, the DA4 `core` and
/// the system-wide `llc`. Records of an unknown type or too short are skipped;
/// a size that runs past the buffer ends the walk.
pub fn parse_cpu_sets(buf: &[u8]) -> Vec<LogicalCpu> {
    let mut raw = Vec::new();
    let mut off = 0usize;
    while let Some(rec) = read_at::<SYSTEM_CPU_SET_INFORMATION>(buf, off) {
        let size = rec.Size as usize;
        if size < 8 || off + size > buf.len() {
            break;
        }
        if rec.Type.0 as u32 == CPU_SET_TYPE && size >= size_of::<SYSTEM_CPU_SET_INFORMATION>() {
            // SAFETY: the record type is CpuSetInformation, so the `CpuSet` arm of the union is
            // the active one, and the flags byte is plain data.
            let (set, flags) = unsafe {
                let set = rec.Anonymous.CpuSet;
                (set, set.Anonymous1.AllFlags)
            };
            raw.push(RawCpu {
                group: set.Group,
                number: set.LogicalProcessorIndex,
                core_index: set.CoreIndex,
                llc: set.LastLevelCacheIndex,
                efficiency_class: set.EfficiencyClass,
                parked: flags & PARKED_BIT != 0,
            });
        }
        off += size;
    }
    raw.sort_by_key(|c| (c.group, c.number));
    let cores: BTreeSet<(u16, u8)> = raw.iter().map(|c| (c.group, c.core_index)).collect();
    let llcs: BTreeSet<(u16, u8)> = raw.iter().map(|c| (c.group, c.llc)).collect();
    raw.iter()
        .enumerate()
        .map(|(i, c)| LogicalCpu {
            index: i as u32,
            group: c.group,
            number: c.number,
            core: rank(&cores, (c.group, c.core_index)),
            core_index: u32::from(c.core_index),
            efficiency_class: c.efficiency_class,
            llc: rank(&llcs, (c.group, c.llc)),
            parked: c.parked,
            apic_id: None,
        })
        .collect()
}

/// Walks the `RelationCache` records of `buf`. `l1d_bytes`, `l2_bytes` and
/// `l2_shared_by` come from the first L1 data (or unified) and the first L2
/// instance; `l3_bytes` is one L3 instance, `l3_total_bytes` the sum of all
/// of them (both 0 without an L3).
pub fn parse_caches(buf: &[u8]) -> CacheSizes {
    const CACHE_OFF: usize =
        core::mem::offset_of!(SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX, Anonymous);
    const MASKS_OFF: usize = CACHE_OFF + core::mem::offset_of!(CACHE_RELATIONSHIP, Anonymous);
    const CACHE_DATA: i32 = 2;
    const CACHE_INSTRUCTION: i32 = 1;

    let mut out = CacheSizes {
        l1d_bytes: 0,
        l2_bytes: 0,
        l2_shared_by: 0,
        l3_bytes: 0,
        l3_total_bytes: 0,
    };
    let mut off = 0usize;
    // Only the 8-byte header is read at first: the records vary in size and the cache ones are
    // shorter than the whole union.
    while let Some([relationship, size]) = read_at::<[u32; 2]>(buf, off) {
        let size = size as usize;
        if size < 8 || off + size > buf.len() {
            break;
        }
        if relationship == RelationCache.0 as u32 && size >= MASKS_OFF + size_of::<GROUP_AFFINITY>()
        {
            let Some(cache) = read_at::<CACHE_RELATIONSHIP>(buf, off + CACHE_OFF) else {
                break;
            };
            let bytes = u64::from(cache.CacheSize);
            match cache.Level {
                1 if cache.Type.0 != CACHE_INSTRUCTION && out.l1d_bytes == 0 => {
                    debug_assert!(cache.Type.0 == CACHE_DATA || cache.Type.0 == 0);
                    out.l1d_bytes = bytes;
                }
                2 if out.l2_bytes == 0 => {
                    out.l2_bytes = bytes;
                    let masks = usize::from(cache.GroupCount)
                        .min((size - MASKS_OFF) / size_of::<GROUP_AFFINITY>());
                    out.l2_shared_by = (0..masks)
                        .filter_map(|i| {
                            read_at::<GROUP_AFFINITY>(
                                buf,
                                off + MASKS_OFF + i * size_of::<GROUP_AFFINITY>(),
                            )
                        })
                        .map(|m| m.Mask.count_ones())
                        .sum();
                }
                3 => {
                    if out.l3_bytes == 0 {
                        out.l3_bytes = bytes;
                    }
                    out.l3_total_bytes += bytes;
                }
                _ => {}
            }
        }
        off += size;
    }
    out
}

/// Calls a "query the size, then fill a buffer" API until the buffer fits.
/// `call` gets the buffer pointer (null on the first, size-only call), its
/// length, and the in/out length.
fn fill_buffer(mut call: impl FnMut(*mut u8, &mut u32) -> Result<(), i32>) -> io::Result<Vec<u8>> {
    let mut len = 0u32;
    // The size can change between the two calls (a CPU set parks): retry a few times.
    for _ in 0..4 {
        // u64 elements keep the buffer 8-aligned, as the records contain 8-byte fields.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ptr = if len == 0 {
            std::ptr::null_mut()
        } else {
            buf.as_mut_ptr().cast()
        };
        match call(ptr, &mut len) {
            Ok(()) if len == 0 || !ptr.is_null() => {
                let mut bytes: Vec<u8> = buf.iter().flat_map(|w| w.to_ne_bytes()).collect();
                bytes.truncate(len as usize);
                return Ok(bytes);
            }
            Ok(()) => {}
            Err(ERROR_INSUFFICIENT_BUFFER) => {}
            Err(code) => return Err(io::Error::from_raw_os_error(code)),
        }
    }
    Err(io::Error::other("topology size kept changing"))
}

fn cpu_set_buffer() -> io::Result<Vec<u8>> {
    fill_buffer(|ptr, len| {
        let cap = if ptr.is_null() { 0 } else { *len };
        // SAFETY: `ptr` is null with a zero length (size query) or points to a live, writable,
        // 8-aligned buffer of `cap` bytes; `len` is a valid out pointer.
        let ok = unsafe {
            GetSystemCpuSetInformation(
                (!ptr.is_null()).then_some(ptr.cast()),
                cap,
                len,
                Some(GetCurrentProcess()),
                None,
            )
        };
        if ok.as_bool() {
            Ok(())
        } else {
            Err(io::Error::last_os_error().raw_os_error().unwrap_or(0))
        }
    })
}

fn cache_buffer() -> io::Result<Vec<u8>> {
    fill_buffer(|ptr, len| {
        // SAFETY: `ptr` is null for the size query or points to a live, writable, 8-aligned
        // buffer whose size is in `*len`; the API reads `*len` as the capacity.
        unsafe {
            GetLogicalProcessorInformationEx(
                RelationCache,
                (!ptr.is_null()).then_some(ptr.cast()),
                len,
            )
        }
        .map_err(|e| os_error(&e).raw_os_error().unwrap_or(0))
    })
}

/// The 12-byte vendor string, the brand string and the hypervisor bit.
#[cfg(target_arch = "x86_64")]
fn cpuid_identity() -> (String, String, bool) {
    use std::arch::x86_64::__cpuid;
    // SAFETY: CPUID is available on every x86-64 CPU; the leaves queried only read registers.
    let (vendor, brand, hypervisor) = unsafe {
        let l0 = __cpuid(0);
        let mut vendor = Vec::with_capacity(12);
        for r in [l0.ebx, l0.edx, l0.ecx] {
            vendor.extend_from_slice(&r.to_le_bytes());
        }
        let max_ext = __cpuid(0x8000_0000).eax;
        let mut brand = Vec::with_capacity(48);
        if max_ext >= 0x8000_0004 {
            for leaf in 0x8000_0002..=0x8000_0004 {
                let r = __cpuid(leaf);
                for reg in [r.eax, r.ebx, r.ecx, r.edx] {
                    brand.extend_from_slice(&reg.to_le_bytes());
                }
            }
        }
        (vendor, brand, __cpuid(1).ecx >> 31 & 1 == 1)
    };
    let text = |b: Vec<u8>| {
        String::from_utf8_lossy(&b)
            .trim_matches(|c: char| c == '\0' || c.is_whitespace())
            .to_owned()
    };
    (text(vendor), text(brand), hypervisor)
}

#[cfg(not(target_arch = "x86_64"))]
fn cpuid_identity() -> (String, String, bool) {
    (String::new(), String::new(), false)
}

/// Reads the topology of this machine. `apic_id` stays `None`.
pub fn read() -> io::Result<Topology> {
    let logical = parse_cpu_sets(&cpu_set_buffer()?);
    if logical.is_empty() {
        return Err(io::Error::other(
            "GetSystemCpuSetInformation returned no CPU sets",
        ));
    }
    let caches = parse_caches(&cache_buffer()?);
    let (vendor, brand, hypervisor) = cpuid_identity();
    Ok(Topology {
        logical,
        caches,
        hypervisor,
        vendor,
        brand,
    })
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::SystemInformation::{
        CacheData, CacheUnified, LOGICAL_PROCESSOR_RELATIONSHIP, PROCESSOR_CACHE_TYPE,
    };

    use super::*;

    // The parsers read the `windows` crate's structs, so these pin the layout
    // the docs give (x64): the records are 32 bytes, the cache masks start at 40.
    #[test]
    fn struct_layouts_match_the_documented_ones() {
        assert_eq!(size_of::<SYSTEM_CPU_SET_INFORMATION>(), 32);
        assert_eq!(size_of::<GROUP_AFFINITY>(), 16);
        assert_eq!(
            core::mem::offset_of!(SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX, Anonymous)
                + core::mem::offset_of!(CACHE_RELATIONSHIP, Anonymous),
            40
        );
    }

    fn bytes_of<T: Copy>(v: &T, len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len.max(size_of::<T>())];
        // SAFETY: `out` holds at least size_of::<T>() bytes; `v` is a live `T`.
        unsafe {
            std::ptr::copy_nonoverlapping(
                (v as *const T).cast::<u8>(),
                out.as_mut_ptr(),
                size_of::<T>(),
            )
        };
        out
    }

    fn cpu_set(group: u16, number: u8, core: u8, llc: u8, eff: u8, parked: bool) -> Vec<u8> {
        let mut r = SYSTEM_CPU_SET_INFORMATION {
            Size: size_of::<SYSTEM_CPU_SET_INFORMATION>() as u32,
            ..Default::default()
        };
        r.Anonymous.CpuSet.Group = group;
        r.Anonymous.CpuSet.LogicalProcessorIndex = number;
        r.Anonymous.CpuSet.CoreIndex = core;
        r.Anonymous.CpuSet.LastLevelCacheIndex = llc;
        r.Anonymous.CpuSet.EfficiencyClass = eff;
        r.Anonymous.CpuSet.Anonymous1.AllFlags = u8::from(parked);
        bytes_of(&r, 0)
    }

    fn cache(level: u8, kind: PROCESSOR_CACHE_TYPE, size: u32, masks: &[(u16, usize)]) -> Vec<u8> {
        let mut r = SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX {
            Relationship: RelationCache,
            ..Default::default()
        };
        r.Anonymous.Cache.Level = level;
        r.Anonymous.Cache.Type = kind;
        r.Anonymous.Cache.CacheSize = size;
        r.Anonymous.Cache.GroupCount = masks.len() as u16;
        let base = 40 + masks.len().max(1) * 16;
        r.Size = base as u32;
        let mut out = bytes_of(&r, base);
        out.truncate(base);
        for (i, (group, mask)) in masks.iter().enumerate() {
            let m = GROUP_AFFINITY {
                Mask: *mask,
                Group: *group,
                Reserved: [0; 3],
            };
            out[40 + i * 16..56 + i * 16].copy_from_slice(&bytes_of(&m, 16)[..16]);
        }
        out
    }

    #[test]
    fn parse_cpu_sets_two_groups_and_smt() {
        // Group 0: cores 0..2 with SMT (2 each), the second one parked.
        // Group 1: one core with SMT. Listed out of order on purpose.
        let mut buf = Vec::new();
        for (g, n, core, llc, parked) in [
            (1u16, 0u8, 0u8, 0u8, false),
            (1, 1, 0, 0, false),
            (0, 2, 1, 0, true),
            (0, 3, 1, 0, true),
            (0, 0, 0, 0, false),
            (0, 1, 0, 0, false),
        ] {
            buf.extend(cpu_set(g, n, core, llc, 0, parked));
        }
        let cpus = parse_cpu_sets(&buf);
        let view: Vec<_> = cpus
            .iter()
            .map(|c| (c.index, c.group, c.number, c.core, c.llc, c.parked))
            .collect();
        assert_eq!(
            view,
            vec![
                (0, 0, 0, 0, 0, false),
                (1, 0, 1, 0, 0, false),
                (2, 0, 2, 1, 0, true),
                (3, 0, 3, 1, 0, true),
                // Group-relative CoreIndex 0 of group 1 is a third core, and its cache domain
                // a second one.
                (4, 1, 0, 2, 1, false),
                (5, 1, 1, 2, 1, false),
            ]
        );
        assert_eq!(cpus[2].core_index, 1);
        assert!(cpus.iter().all(|c| c.apic_id.is_none()));
    }

    #[test]
    fn parse_cpu_sets_hybrid_efficiency_classes() {
        // 2 P cores with SMT (class 1) then 2 E cores (class 0), raw CoreIndex with gaps.
        let mut buf = Vec::new();
        for (n, core, eff) in [
            (0u8, 0u8, 1u8),
            (1, 0, 1),
            (2, 2, 1),
            (3, 2, 1),
            (4, 4, 0),
            (5, 5, 0),
        ] {
            buf.extend(cpu_set(0, n, core, 0, eff, false));
        }
        let cpus = parse_cpu_sets(&buf);
        assert_eq!(
            cpus.iter()
                .map(|c| (c.core, c.efficiency_class))
                .collect::<Vec<_>>(),
            vec![(0, 1), (0, 1), (1, 1), (1, 1), (2, 0), (3, 0)]
        );
    }

    #[test]
    fn parse_cpu_sets_skips_unknown_and_stops_on_a_bad_size() {
        let mut unknown = cpu_set(0, 7, 7, 0, 0, false);
        unknown[4] = 9; // Type
        let mut buf = cpu_set(0, 0, 0, 0, 0, false);
        buf.extend(unknown);
        buf.extend(cpu_set(0, 1, 1, 0, 0, false));
        let mut broken = cpu_set(0, 2, 2, 0, 0, false);
        broken[0..4].copy_from_slice(&1000u32.to_le_bytes());
        buf.extend(broken);
        assert_eq!(parse_cpu_sets(&buf).len(), 2);
        assert!(parse_cpu_sets(&[]).is_empty());
        assert!(parse_cpu_sets(&[1, 2, 3]).is_empty());
    }

    #[test]
    fn parse_caches_l1_l2_l3_and_missing_l3() {
        let mut buf = Vec::new();
        buf.extend(cache(1, CacheData, 32 * 1024, &[(0, 0b11)]));
        buf.extend(cache(1, PROCESSOR_CACHE_TYPE(1), 32 * 1024, &[(0, 0b11)]));
        buf.extend(cache(2, CacheUnified, 1024 * 1024, &[(0, 0b11)]));
        buf.extend(cache(2, CacheUnified, 1024 * 1024, &[(0, 0b1100)]));
        // Two L3 instances of 32 MiB; the second one spans two groups.
        buf.extend(cache(3, CacheUnified, 32 << 20, &[(0, 0xff)]));
        buf.extend(cache(3, CacheUnified, 32 << 20, &[(0, 0xf), (1, 0xf)]));
        // A record of another relationship is skipped.
        let mut other = SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX {
            Relationship: LOGICAL_PROCESSOR_RELATIONSHIP(0),
            Size: 64,
            ..Default::default()
        };
        other.Size = 64;
        let mut other = bytes_of(&other, 64);
        other.truncate(64);
        buf.extend(other);
        let c = parse_caches(&buf);
        assert_eq!(c.l1d_bytes, 32 * 1024);
        assert_eq!(c.l2_bytes, 1024 * 1024);
        assert_eq!(c.l2_shared_by, 2);
        assert_eq!(c.l3_bytes, 32 << 20);
        assert_eq!(c.l3_total_bytes, 64 << 20);

        let no_l3 = parse_caches(&cache(2, CacheUnified, 512 * 1024, &[(0, 0b1)]));
        assert_eq!((no_l3.l3_bytes, no_l3.l3_total_bytes), (0, 0));
        assert_eq!((no_l3.l2_bytes, no_l3.l2_shared_by), (512 * 1024, 1));
        assert_eq!(parse_caches(&[]).l2_bytes, 0);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn real_topology_has_cores() {
        let t = read().expect("read the topology");
        assert!(!t.logical.is_empty());
        assert!(t.logical.iter().any(|c| c.core == 0));
        assert!(t.caches.l2_bytes > 0, "{:?}", t.caches);
        assert!(!t.vendor.is_empty());
        eprintln!("{} ({}) {:?}", t.brand, t.vendor, t.caches);
    }
}
