//! Signed 4 KiB block format shared by the disk benchmark and stress test.
//!
//! Header (64 bytes): magic, session, index, generation, version, xxh3-64 of
//! bytes 0..32 and 64..4096, then zero padding. The payload is SplitMix64 words
//! (or zeros when the block is compressible).

use oma_ipc::load::ErrorKind;

pub const BLOCK_BYTES: usize = 4096;
pub const HEADER_BYTES: usize = 64;
pub const MAGIC: [u8; 8] = *b"OMADTEST";
pub const BLOCK_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// Number of differing bits against the expected block.
    BitFlip(u32),
    /// Index found in a block that is otherwise intact.
    Misplaced(u64),
    /// Older generation found in an otherwise intact block.
    Stale(u32),
    Zeros,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockFault {
    pub kind: FaultKind,
    pub expected: u64,
    pub actual: u64,
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn rd64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn rd32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn checksum(buf: &[u8]) -> u64 {
    // Only the one-shot API exists without the crate's alloc feature.
    let mut tmp = [0u8; BLOCK_BYTES - 32];
    tmp[..32].copy_from_slice(&buf[..32]);
    tmp[32..].copy_from_slice(&buf[HEADER_BYTES..BLOCK_BYTES]);
    twox_hash::XxHash3_64::oneshot(&tmp)
}

/// Fills `buf` (exactly `BLOCK_BYTES`) with the block for these coordinates.
pub fn write_block(buf: &mut [u8], session: u64, index: u64, generation: u32, compressible: bool) {
    assert_eq!(buf.len(), BLOCK_BYTES);
    buf[..8].copy_from_slice(&MAGIC);
    buf[8..16].copy_from_slice(&session.to_le_bytes());
    buf[16..24].copy_from_slice(&index.to_le_bytes());
    buf[24..28].copy_from_slice(&generation.to_le_bytes());
    buf[28..32].copy_from_slice(&BLOCK_VERSION.to_le_bytes());
    buf[32..HEADER_BYTES].fill(0);
    if compressible {
        buf[HEADER_BYTES..].fill(0);
    } else {
        let mut s = splitmix(&mut (session ^ index.rotate_left(17) ^ ((generation as u64) << 48)));
        for w in buf[HEADER_BYTES..].chunks_exact_mut(8) {
            w.copy_from_slice(&splitmix(&mut s).to_le_bytes());
        }
    }
    let sum = checksum(buf);
    buf[32..40].copy_from_slice(&sum.to_le_bytes());
}

fn bit_diff(a: &[u8], b: &[u8]) -> u32 {
    a.iter().zip(b).map(|(x, y)| (x ^ y).count_ones()).sum()
}

/// Checks that `buf` is exactly the block written for these coordinates.
pub fn check_block(
    buf: &[u8],
    session: u64,
    index: u64,
    generation: u32,
) -> Result<(), BlockFault> {
    assert_eq!(buf.len(), BLOCK_BYTES);
    let sum = checksum(buf);
    let coherent = sum == rd64(buf, 32)
        && buf[..8] == MAGIC
        && rd64(buf, 8) == session
        && rd32(buf, 28) == BLOCK_VERSION
        && buf[40..HEADER_BYTES].iter().all(|&b| b == 0);
    let (found_index, found_gen) = (rd64(buf, 16), rd32(buf, 24));
    if coherent && found_index == index && found_gen == generation {
        return Ok(());
    }
    let fault = |kind, expected, actual| {
        Err(BlockFault {
            kind,
            expected,
            actual,
        })
    };
    if buf.iter().all(|&b| b == 0) {
        return fault(FaultKind::Zeros, 0, 0);
    }
    if coherent && found_index != index {
        return fault(FaultKind::Misplaced(found_index), index, found_index);
    }
    if coherent && found_gen < generation {
        return fault(
            FaultKind::Stale(found_gen),
            generation as u64,
            found_gen as u64,
        );
    }
    // The expected payload may be random or compressible: take the closer one.
    let mut exp = [0u8; BLOCK_BYTES];
    write_block(&mut exp, session, index, generation, false);
    let mut bits = bit_diff(buf, &exp);
    write_block(&mut exp, session, index, generation, true);
    bits = bits.min(bit_diff(buf, &exp));
    fault(FaultKind::BitFlip(bits), 0, bits as u64)
}

pub fn fault_error_kind(kind: &FaultKind) -> ErrorKind {
    match kind {
        FaultKind::BitFlip(_) => ErrorKind::BitFlip,
        FaultKind::Misplaced(_) => ErrorKind::Misplaced,
        FaultKind::Stale(_) => ErrorKind::Stale,
        FaultKind::Zeros => ErrorKind::Zeros,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(s: u64, i: u64, g: u32, c: bool) -> [u8; BLOCK_BYTES] {
        let mut b = [0u8; BLOCK_BYTES];
        write_block(&mut b, s, i, g, c);
        b
    }

    #[test]
    fn written_block_checks_ok() {
        assert_eq!(check_block(&block(1, 7, 3, false), 1, 7, 3), Ok(()));
    }

    #[test]
    fn same_inputs_give_the_same_bytes() {
        assert_eq!(block(1, 7, 3, false), block(1, 7, 3, false));
        assert_ne!(block(1, 7, 3, false), block(1, 7, 4, false));
    }

    #[test]
    fn every_bit_of_the_payload_is_covered() {
        for c in [false, true] {
            for n in 0..64usize {
                let pos = HEADER_BYTES + n * (BLOCK_BYTES - HEADER_BYTES) / 64 + n % 7;
                let mut b = block(5, 2, 1, c);
                b[pos] ^= 1 << (n % 8);
                let f = check_block(&b, 5, 2, 1).unwrap_err();
                assert_eq!(f.kind, FaultKind::BitFlip(1), "pos {pos}");
            }
        }
    }

    #[test]
    fn header_flip_is_a_bit_flip() {
        for pos in [3usize, 9, 17, 25, 33, 45, 63] {
            let mut b = block(5, 2, 1, false);
            b[pos] ^= 0x10;
            let f = check_block(&b, 5, 2, 1).unwrap_err();
            assert_eq!(f.kind, FaultKind::BitFlip(1), "pos {pos}");
        }
    }

    #[test]
    fn block_from_another_index_is_misplaced() {
        let f = check_block(&block(1, 7, 1, false), 1, 9, 1).unwrap_err();
        assert_eq!(f.kind, FaultKind::Misplaced(7));
    }

    #[test]
    fn older_generation_is_stale() {
        let f = check_block(&block(1, 7, 2, false), 1, 7, 3).unwrap_err();
        assert_eq!(f.kind, FaultKind::Stale(2));
    }

    #[test]
    fn zeroed_block_is_zeros() {
        let f = check_block(&[0u8; BLOCK_BYTES], 1, 7, 3).unwrap_err();
        assert_eq!(f.kind, FaultKind::Zeros);
    }

    #[test]
    fn compressible_payload_still_checks() {
        let b = block(1, 7, 3, true);
        assert!(b[HEADER_BYTES..].iter().all(|&x| x == 0));
        assert_eq!(check_block(&b, 1, 7, 3), Ok(()));
    }

    #[test]
    fn fault_kinds_map_to_error_kinds() {
        assert_eq!(fault_error_kind(&FaultKind::BitFlip(1)), ErrorKind::BitFlip);
        assert_eq!(
            fault_error_kind(&FaultKind::Misplaced(1)),
            ErrorKind::Misplaced
        );
        assert_eq!(fault_error_kind(&FaultKind::Stale(1)), ErrorKind::Stale);
        assert_eq!(fault_error_kind(&FaultKind::Zeros), ErrorKind::Zeros);
    }
}
