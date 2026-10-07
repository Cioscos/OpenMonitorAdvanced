//! CPU references of the S1 and S2 shaders (plan DG5): the same formulas, bit for bit, so a
//! sample of the GPU's threads can be checked. Pure.

/// `-1.0f32`: with it each S1 step maps `x` to `c - x`, so every value stays a small
/// integer and FP32 stays exact. It travels in the constant buffer so the compiler cannot
/// fold it.
pub const FMA_M: f32 = -1.0;
// `fma_thread` computes `mad(v, FMA_M, c)` as `c - v`.
const _: () = assert!(FMA_M == -1.0);
/// The multiplier of the S2 hash steps.
pub const HASH_K: u32 = 0x9E37_79B1;

/// Constant buffer of `s1_fma.hlsl`: `{ steps, seed, m, pad }`.
pub fn fma_params(steps: u32, seed: u32) -> [u32; 4] {
    [steps, seed, FMA_M.to_bits(), 0]
}

/// Constant buffer of `s2_hash.hlsl`: `{ steps, seed, k, pad }`.
pub fn hash_params(steps: u32, seed: u32) -> [u32; 4] {
    [steps, seed, HASH_K, 0]
}

/// Output of S1 thread `id` (the bits of its `float4`).
pub fn fma_thread(id: u32, seed: u32, steps: u32) -> [u32; 4] {
    let b = ((id ^ seed) & 255) as f32;
    // Value `k = 4 j + l` (lane `l` of chain `j`) starts at `b + k` with constant
    // `1000 + k`; each step applies four `mad`s to every chain. The 16 values step together:
    // independent operations pipeline and vectorize, where one chain at a time waits on
    // each result (~0.45 s a phase for the sample check).
    let c: [f32; 16] = std::array::from_fn(|k| 1000.0 + k as f32);
    let mut v: [f32; 16] = std::array::from_fn(|k| b + k as f32);
    for _ in 0..u64::from(steps) * 4 {
        for (v, c) in v.iter_mut().zip(&c) {
            // `mad(v, -1, c)`: the product is exact, so the FMA rounds once, as the
            // subtraction does; bit for bit the same, without the software `fmaf` of
            // `f32::mul_add` (no `fma` target feature).
            *v = c - *v;
        }
    }
    std::array::from_fn(|l| (v[l] + v[4 + l] * 2.0 + v[8 + l] * 4.0 + v[12 + l] * 8.0).to_bits())
}

/// Output of S2 thread `id`.
pub fn hash_thread(id: u32, seed: u32, steps: u32) -> [u32; 4] {
    let h0 = [
        id,
        id ^ seed,
        id.wrapping_mul(3).wrapping_add(1),
        id.wrapping_add(seed),
    ];
    let mut hs = [
        h0,
        h0.map(|v| v ^ 0x9E37_79B9),
        h0.map(|v| v.wrapping_add(0x85EB_CA6B)),
        h0.map(|v| v.wrapping_mul(0xC2B2_AE35)),
    ];
    for _ in 0..steps {
        for (c, h) in hs.iter_mut().enumerate() {
            for (l, v) in h.iter_mut().enumerate() {
                let mut x = v.wrapping_mul(HASH_K);
                x ^= x >> 15;
                x = x.rotate_left(13);
                // `step4(h, uint4(4 c + 1, ..., 4 c + 4))`.
                *v = x.wrapping_add((c * 4 + l + 1) as u32);
            }
        }
    }
    std::array::from_fn(|l| hs[0][l] ^ hs[1][l] ^ hs[2][l] ^ hs[3][l])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fma_reference_stays_integer() {
        for id in [0, 1, 97, 255, 1 << 19] {
            let b = ((id ^ 0x5A5A) & 255) as f32;
            // x0 + 2 x1 + 4 x2 + 8 x3 of the starting values b + lane + 4 j.
            let start: [u32; 4] =
                std::array::from_fn(|l| ((b + l as f32) * 15.0 + 136.0).to_bits());
            for steps in [0, 2, 1000] {
                assert_eq!(
                    fma_thread(id, 0x5A5A, steps),
                    start,
                    "id {id} steps {steps}"
                );
            }
        }
    }

    #[test]
    fn hash_reference_known_vector() {
        // Computed once by an independent Python port of `hash.hlsl`.
        assert_eq!(
            hash_thread(12345, 0x1234_5678, 1000),
            [0xA715_EFB2, 0x4A09_1ABC, 0xED7D_45CA, 0x62F7_A446]
        );
    }
}
