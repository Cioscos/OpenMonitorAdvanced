//! Carry-less 64x64 -> 128 multiplication: PCLMULQDQ and a software reference.

use std::arch::x86_64::*;

/// The software product, shift and xor.
pub fn mul_sw(a: u64, b: u64) -> u128 {
    (0..64)
        .filter(|i| b >> i & 1 == 1)
        .fold(0, |acc, i| acc ^ (u128::from(a) << i))
}

fn words(pair: &[u8]) -> (u64, u64) {
    (
        u64::from_le_bytes(pair[..8].try_into().unwrap()),
        u64::from_le_bytes(pair[8..].try_into().unwrap()),
    )
}

/// Multiplies the words of `buf` in pairs, xoring the products; every `every`-th product
/// is compared with the software one. `Err((expected, actual))` on a difference.
///
/// # Safety
/// The CPU has PCLMULQDQ and SSE2.
#[target_feature(enable = "pclmulqdq,sse2")]
pub unsafe fn accumulate_hw(buf: &[u8], every: usize) -> Result<u128, (u128, u128)> {
    let mut acc = 0u128;
    for (n, pair) in buf.chunks_exact(16).enumerate() {
        let (a, b) = words(pair);
        let p = _mm_clmulepi64_si128(_mm_set_epi64x(0, a as i64), _mm_set_epi64x(0, b as i64), 0);
        let mut out = [0u64; 2];
        // SAFETY: `out` is 16 bytes long; the store is unaligned.
        unsafe { _mm_storeu_si128(out.as_mut_ptr().cast(), p) };
        let hw = u128::from(out[0]) | u128::from(out[1]) << 64;
        if n % every == 0 {
            let sw = mul_sw(a, b);
            if sw != hw {
                return Err((sw, hw));
            }
        }
        acc ^= hw;
    }
    Ok(acc)
}

/// The same accumulation with the software product only.
pub fn accumulate_sw(buf: &[u8]) -> u128 {
    buf.chunks_exact(16).fold(0, |acc, pair| {
        let (a, b) = words(pair);
        acc ^ mul_sw(a, b)
    })
}
