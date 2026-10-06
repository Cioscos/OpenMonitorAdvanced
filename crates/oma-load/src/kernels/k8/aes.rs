//! AES-128 with AES-NI: key expansion by `_mm_aeskeygenassist_si128` and a CBC-style chain
//! over a buffer (each block is xored into the running state, which is then encrypted), so
//! the final block depends on every byte.

use std::arch::x86_64::*;

/// The final chain block of `buf` (length a multiple of 16), starting from a zero state.
///
/// # Safety
/// The CPU has AES-NI and SSE2.
#[target_feature(enable = "aes,sse2")]
pub unsafe fn chain(key: &[u8; 16], buf: &[u8]) -> [u8; 16] {
    let rk = expand(key);
    let mut state = _mm_setzero_si128();
    for block in buf.chunks_exact(16) {
        // SAFETY: `block` is 16 bytes long; the load is unaligned.
        let b = unsafe { _mm_loadu_si128(block.as_ptr().cast()) };
        state = encrypt(&rk, _mm_xor_si128(state, b));
    }
    let mut out = [0u8; 16];
    // SAFETY: `out` is 16 bytes long; the store is unaligned.
    unsafe { _mm_storeu_si128(out.as_mut_ptr().cast(), state) };
    out
}

/// One block, as in FIPS-197.
///
/// # Safety
/// The CPU has AES-NI and SSE2.
#[target_feature(enable = "aes,sse2")]
pub unsafe fn encrypt_block(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    let rk = expand(key);
    // SAFETY: both arrays are 16 bytes long; the accesses are unaligned.
    let c = encrypt(&rk, unsafe { _mm_loadu_si128(block.as_ptr().cast()) });
    let mut out = [0u8; 16];
    // SAFETY: as above.
    unsafe { _mm_storeu_si128(out.as_mut_ptr().cast(), c) };
    out
}

#[inline]
#[target_feature(enable = "aes,sse2")]
fn encrypt(rk: &[__m128i; 11], block: __m128i) -> __m128i {
    let mut s = _mm_xor_si128(block, rk[0]);
    for k in &rk[1..10] {
        s = _mm_aesenc_si128(s, *k);
    }
    _mm_aesenclast_si128(s, rk[10])
}

#[target_feature(enable = "aes,sse2")]
fn expand(key: &[u8; 16]) -> [__m128i; 11] {
    // SAFETY: `key` is 16 bytes long; the load is unaligned.
    let mut k = unsafe { _mm_loadu_si128(key.as_ptr().cast()) };
    let mut rk = [k; 11];
    macro_rules! round {
        ($i:expr, $rcon:literal) => {{
            let g = _mm_shuffle_epi32(_mm_aeskeygenassist_si128(k, $rcon), 0xff);
            k = _mm_xor_si128(k, _mm_slli_si128(k, 4));
            k = _mm_xor_si128(k, _mm_slli_si128(k, 4));
            k = _mm_xor_si128(k, _mm_slli_si128(k, 4));
            k = _mm_xor_si128(k, g);
            rk[$i] = k;
        }};
    }
    round!(1, 0x01);
    round!(2, 0x02);
    round!(3, 0x04);
    round!(4, 0x08);
    round!(5, 0x10);
    round!(6, 0x20);
    round!(7, 0x40);
    round!(8, 0x80);
    round!(9, 0x1b);
    round!(10, 0x36);
    rk
}
