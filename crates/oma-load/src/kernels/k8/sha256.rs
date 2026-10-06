//! SHA-256 (FIPS 180-4): a scalar implementation and the SHA-NI one; both give the same
//! digest.

use std::arch::x86_64::*;

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// SHA-256 of `data`, with SHA-NI when `use_ni` (the caller checked the CPU has it, and
/// SSSE3 and SSE4.1).
pub fn sha256(data: &[u8], use_ni: bool) -> [u8; 32] {
    let mut state = H0;
    let mut tail = data[data.len() / 64 * 64..].to_vec();
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for part in [&data[..data.len() / 64 * 64], &tail[..]] {
        if use_ni {
            // SAFETY: the caller checked SHA, SSSE3 and SSE4.1 (`use_ni`).
            unsafe { compress_ni(&mut state, part) };
        } else {
            compress(&mut state, part);
        }
    }
    let mut out = [0u8; 32];
    for (o, s) in out.chunks_exact_mut(4).zip(state) {
        o.copy_from_slice(&s.to_be_bytes());
    }
    out
}

fn compress(state: &mut [u32; 8], blocks: &[u8]) {
    for block in blocks.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
}

/// # Safety
/// The CPU has SHA, SSSE3 and SSE4.1.
#[target_feature(enable = "sha,sse2,ssse3,sse4.1")]
unsafe fn compress_ni(state: &mut [u32; 8], blocks: &[u8]) {
    let mask = _mm_set_epi64x(0x0c0d0e0f08090a0b, 0x0405060700010203);
    // SAFETY: `state` has 8 words; the loads are unaligned and 16 bytes each.
    let (lo, hi) = unsafe {
        (
            _mm_loadu_si128(state.as_ptr().cast()),
            _mm_loadu_si128(state.as_ptr().add(4).cast()),
        )
    };
    let tmp = _mm_shuffle_epi32(lo, 0xB1); // CDAB
    let efgh = _mm_shuffle_epi32(hi, 0x1B); // EFGH
    let mut abef = _mm_alignr_epi8(tmp, efgh, 8);
    let mut cdgh = _mm_blend_epi16(efgh, tmp, 0xF0);
    for block in blocks.chunks_exact(64) {
        let (save_abef, save_cdgh) = (abef, cdgh);
        let mut m = [_mm_setzero_si128(); 4];
        for (i, w) in m.iter_mut().enumerate() {
            // SAFETY: `block` has 64 bytes; each load is 16 bytes, unaligned.
            let v = unsafe { _mm_loadu_si128(block.as_ptr().add(16 * i).cast()) };
            *w = _mm_shuffle_epi8(v, mask);
        }
        for g in 0..16 {
            if g >= 4 {
                let t = _mm_add_epi32(
                    _mm_sha256msg1_epu32(m[g % 4], m[(g + 1) % 4]),
                    _mm_alignr_epi8(m[(g + 3) % 4], m[(g + 2) % 4], 4),
                );
                m[g % 4] = _mm_sha256msg2_epu32(t, m[(g + 3) % 4]);
            }
            // SAFETY: `K` has 64 words; 4 * g + 4 <= 64.
            let k = unsafe { _mm_loadu_si128(K.as_ptr().add(4 * g).cast()) };
            let msg = _mm_add_epi32(m[g % 4], k);
            cdgh = _mm_sha256rnds2_epu32(cdgh, abef, msg);
            abef = _mm_sha256rnds2_epu32(abef, cdgh, _mm_shuffle_epi32(msg, 0x0E));
        }
        abef = _mm_add_epi32(abef, save_abef);
        cdgh = _mm_add_epi32(cdgh, save_cdgh);
    }
    let tmp = _mm_shuffle_epi32(abef, 0x1B); // FEBA
    let cdgh = _mm_shuffle_epi32(cdgh, 0xB1); // DCHG
    let lo = _mm_blend_epi16(tmp, cdgh, 0xF0); // DCBA
    let hi = _mm_alignr_epi8(cdgh, tmp, 8); // HGFE
                                            // SAFETY: `state` has 8 words; the stores are unaligned and 16 bytes each.
    unsafe {
        _mm_storeu_si128(state.as_mut_ptr().cast(), lo);
        _mm_storeu_si128(state.as_mut_ptr().add(4).cast(), hi);
    }
}
