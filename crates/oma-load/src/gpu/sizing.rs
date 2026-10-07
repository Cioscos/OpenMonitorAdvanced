//! VRAM and submission sizing of the GPU phases (plan DG4, DG6). Pure.

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
/// Left free for the desktop and the other processes.
const HEADROOM: u64 = 400 * MIB;

/// What the VRAM check sizes itself on: the local segment of node 0 and the free RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VramBudget {
    pub budget: u64,
    pub usage: u64,
    pub available_ram: u64,
}

/// Bytes the VRAM check aims to allocate (DG6). An integrated GPU's budget is shared RAM,
/// so it is also capped by a quarter of the free RAM and 4 GiB.
pub fn vram_target(budget: u64, integrated: bool, available_ram: u64) -> u64 {
    let percent = if integrated { 90 } else { 95 };
    let target = (budget / 100 * percent + budget % 100 * percent / 100).saturating_sub(HEADROOM);
    if integrated {
        target.min(available_ram / 4).min(4 * GIB)
    } else {
        target
    }
}

/// Size of one VRAM allocation: a quarter of the dedicated VRAM, between 256 MiB and
/// 1 GiB, rounded down to 4 KiB.
pub fn chunk_bytes(dedicated: u64) -> u64 {
    (dedicated / 4).clamp(256 * MIB, GIB) & !4095
}

/// GPU time each submission is calibrated to (DG4).
pub fn submit_target_ms(integrated: bool) -> f64 {
    if integrated {
        20.0
    } else {
        40.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedicated_vram_target_is_95_percent_minus_400_mib() {
        assert_eq!(vram_target(15_280 * MIB, false, 30 * GIB), 14_116 * MIB);
    }

    #[test]
    fn integrated_vram_target_is_capped() {
        assert_eq!(vram_target(15_647 * MIB, true, 30 * GIB), 4 * GIB);
        assert_eq!(vram_target(15_647 * MIB, true, 8 * GIB), 2 * GIB);
        // A small budget wins over the RAM caps.
        assert_eq!(vram_target(2_000 * MIB, true, 30 * GIB), 1_400 * MIB);
    }

    #[test]
    fn tiny_budget_gives_zero() {
        assert_eq!(vram_target(300 * MIB, false, 30 * GIB), 0);
        assert_eq!(vram_target(0, true, 0), 0);
    }

    #[test]
    fn chunks_are_between_256_mib_and_1_gib() {
        assert_eq!(chunk_bytes(512 * MIB), 256 * MIB);
        assert_eq!(chunk_bytes(16 * GIB), GIB);
        assert_eq!(chunk_bytes(2 * GIB), 512 * MIB);
        let odd = chunk_bytes(3 * GIB + 5);
        assert_eq!(odd % 4096, 0);
        assert!((256 * MIB..=GIB).contains(&odd));
    }

    #[test]
    fn submit_target_is_20_ms_on_integrated() {
        assert_eq!(submit_target_ms(true), 20.0);
        assert_eq!(submit_target_ms(false), 40.0);
    }
}
