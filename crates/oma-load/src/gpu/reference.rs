//! CPU references of the S1 and S2 shaders (plan DG5): the same formulas, bit for bit, so a
//! sample of the GPU's threads can be checked. Pure.

/// `-1.0f32`: with it each S1 step maps `x` to `c - x`, so every value stays a small
/// integer and FP32 stays exact. It travels in the constant buffer so the compiler cannot
/// fold it.
pub const FMA_M: f32 = -1.0;
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
    // Lane `l` of chain `j` starts at `b + l + 4 j` with constant `1000 + l + 4 j`; each
    // step applies four `mad`s to every chain.
    std::array::from_fn(|l| {
        let x: [f32; 4] = std::array::from_fn(|j| {
            let offset = l as f32 + 4.0 * j as f32;
            let c = 1000.0 + offset;
            let mut v = b + offset;
            for _ in 0..u64::from(steps) * 4 {
                v = v.mul_add(FMA_M, c);
            }
            v
        });
        (x[0] + x[1] * 2.0 + x[2] * 4.0 + x[3] * 8.0).to_bits()
    })
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
