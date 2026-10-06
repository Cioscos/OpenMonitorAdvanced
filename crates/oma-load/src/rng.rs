//! Deterministic generators for the kernels' data and the load modes: SplitMix64, used for
//! seeding, and xoshiro256**, used for the data (the published algorithms by Blackman and
//! Vigna). The same seed always gives the same stream, on every CPU.

/// The SplitMix64 increment (the golden ratio in 64 bits).
const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GAMMA);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn fill_u64(&mut self, out: &mut [u64]) {
        out.iter_mut().for_each(|w| *w = self.next_u64());
    }
}

#[derive(Debug, Clone)]
pub struct Xoshiro256ss {
    s: [u64; 4],
}

impl Xoshiro256ss {
    /// The state comes from SplitMix64, so it is never all zeros.
    pub fn new(seed: u64) -> Self {
        let mut sm = SplitMix64::new(seed);
        Self {
            s: [sm.next_u64(), sm.next_u64(), sm.next_u64(), sm.next_u64()],
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    pub fn fill_u64(&mut self, out: &mut [u64]) {
        out.iter_mut().for_each(|w| *w = self.next_u64());
    }
}

/// The seed of phase `phase` of a plan: output `phase + 1` of SplitMix64 from the plan seed.
pub fn phase_seed(plan_seed: u64, phase: u32) -> u64 {
    SplitMix64::new(plan_seed.wrapping_add(u64::from(phase).wrapping_mul(GAMMA))).next_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_seeds_differ_per_phase() {
        // Reference value of SplitMix64 from seed 0.
        assert_eq!(SplitMix64::new(0).next_u64(), 0xE220_A839_7B1D_CDAF);

        let (mut a, mut b) = (Xoshiro256ss::new(42), Xoshiro256ss::new(42));
        let first: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        assert_eq!(first, (0..8).map(|_| b.next_u64()).collect::<Vec<_>>());
        let mut filled = [0u64; 8];
        Xoshiro256ss::new(42).fill_u64(&mut filled);
        assert_eq!(filled.to_vec(), first);
        assert_ne!(Xoshiro256ss::new(43).next_u64(), first[0]);

        assert_eq!(phase_seed(7, 3), phase_seed(7, 3));
        assert_ne!(phase_seed(7, 0), phase_seed(7, 1));
        assert_ne!(phase_seed(7, 0), phase_seed(8, 0));
        let mut sm = SplitMix64::new(7);
        assert_eq!(phase_seed(7, 0), sm.next_u64());
        assert_eq!(phase_seed(7, 1), sm.next_u64());
    }
}
