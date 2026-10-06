//! APIC id of the logical processor the calling thread runs on.

/// Leaf 0x0B (extended topology): EDX is the x2APIC id of the current processor.
pub fn x2apic_from_leaf_b(edx: u32) -> u32 {
    edx
}

/// Leaf 1: EBX[31:24] is the initial 8-bit APIC id.
pub fn apic_from_leaf_1(ebx: u32) -> u32 {
    ebx >> 24
}

/// APIC id of the processor this thread runs now: leaf 0x0B when the CPU has it
/// (EBX of subleaf 0 is zero without it), else leaf 1.
#[cfg(target_arch = "x86_64")]
pub fn apic_id() -> u32 {
    use std::arch::x86_64::__cpuid_count;
    // SAFETY: CPUID exists on every x86_64 CPU; leaf 0 and the leaves read below are plain queries.
    unsafe {
        if __cpuid_count(0, 0).eax >= 0x0B {
            let b = __cpuid_count(0x0B, 0);
            if b.ebx != 0 {
                return x2apic_from_leaf_b(b.edx);
            }
        }
        apic_from_leaf_1(__cpuid_count(1, 0).ebx)
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn apic_id() -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpuid_leaf_parsing() {
        assert_eq!(x2apic_from_leaf_b(0x0000_0123), 0x123);
        assert_eq!(x2apic_from_leaf_b(0xFFFF_FFFF), 0xFFFF_FFFF);
        assert_eq!(apic_from_leaf_1(0x1A00_0800), 0x1A);
        assert_eq!(apic_from_leaf_1(0x00FF_FFFF), 0);
    }
}
