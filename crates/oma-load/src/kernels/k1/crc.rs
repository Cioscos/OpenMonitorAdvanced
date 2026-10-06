/******************************************************************************
 * FIRESTARTER - A Processor Stress Test Utility
 * Copyright (C) 2020-2023 TU Dresden, Center for Information Services and High
 * Performance Computing
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/\>.
 *
 * Contact: daniel.hackenberg@tu-dresden.de
 *****************************************************************************/

// Adapted from FIRESTARTER (https://github.com/tud-zih-energy/FIRESTARTER), commit 927ae17e55f3f90f7575f6a68630a366fde9c94e, include/firestarter/X86/Payload/X86Payload.hpp: emitErrorDetectionCode, the CRC32 hash of the vector registers, 64 bits at a time from 0xffffffff.
// Modified for OpenMonitor Advanced: Rust intrinsics generated at build time instead of asmjit at run time; accumulators reset every block; L3/RAM items dropped.
// The modifications are part of OpenMonitor Advanced, GPL-3.0-or-later.

//! CRC32C (Castagnoli), the hash FIRESTARTER computes over its registers with the SSE4.2
//! `crc32` instruction, with a table in software when the CPU lacks SSE4.2. K1 hashes its
//! accumulators with it; K8 and K10 use it on buffers.

use std::arch::x86_64::{_mm_crc32_u64, _mm_crc32_u8};

/// The reflected Castagnoli polynomial.
const POLY: u32 = 0x82F6_3B78;

static TABLE: [u32; 256] = table();

const fn table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ POLY } else { c >> 1 };
            bit += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

/// Adds the 8 bytes of `x` (little endian) to `acc`, without the final inversion: what
/// `_mm_crc32_u64` computes.
pub fn crc32c_u64(acc: u32, x: u64) -> u32 {
    if is_x86_feature_detected!("sse4.2") {
        // SAFETY: SSE4.2 is present, checked just above.
        unsafe { hw_u64(acc, x) }
    } else {
        crc32c_u64_sw(acc, x)
    }
}

/// The standard CRC32C of `data` (start `0xffffffff`, final inversion).
pub fn crc32c(data: &[u8]) -> u32 {
    if is_x86_feature_detected!("sse4.2") {
        // SAFETY: SSE4.2 is present, checked just above.
        !unsafe { hw_bytes(!0, data) }
    } else {
        !sw_bytes(!0, data)
    }
}

/// [`crc32c_u64`] with the table only.
fn crc32c_u64_sw(acc: u32, x: u64) -> u32 {
    sw_bytes(acc, &x.to_le_bytes())
}

fn sw_bytes(acc: u32, data: &[u8]) -> u32 {
    data.iter()
        .fold(acc, |c, &b| (c >> 8) ^ TABLE[usize::from((c as u8) ^ b)])
}

#[target_feature(enable = "sse4.2")]
unsafe fn hw_u64(acc: u32, x: u64) -> u32 {
    // The upper half of the result is always zero.
    _mm_crc32_u64(u64::from(acc), x) as u32
}

#[target_feature(enable = "sse4.2")]
unsafe fn hw_bytes(acc: u32, data: &[u8]) -> u32 {
    let mut chunks = data.chunks_exact(8);
    let mut c = u64::from(acc);
    for chunk in &mut chunks {
        let word = u64::from_le_bytes(chunk.try_into().expect("8 bytes"));
        c = _mm_crc32_u64(c, word);
    }
    let mut c = c as u32;
    for &b in chunks.remainder() {
        c = _mm_crc32_u8(c, b);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Xoshiro256ss;

    #[test]
    fn crc32c_matches_the_software_table() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283, "the CRC32C check value");
        assert_eq!(crc32c(b""), 0);
        let mut rng = Xoshiro256ss::new(5);
        let mut hw = 0xFFFF_FFFF;
        let mut sw = 0xFFFF_FFFF;
        for _ in 0..1000 {
            let x = rng.next_u64();
            hw = crc32c_u64(hw, x);
            sw = crc32c_u64_sw(sw, x);
            assert_eq!(hw, sw);
        }
        if !is_x86_feature_detected!("sse4.2") {
            eprintln!("skipped: sse4.2 not available");
        }
        // The 8-byte step is the byte-wise CRC of the little-endian bytes.
        let bytes = 0x0123_4567_89AB_CDEFu64.to_le_bytes();
        assert_eq!(!crc32c_u64(!0, 0x0123_4567_89AB_CDEF), crc32c(&bytes));
    }
}
