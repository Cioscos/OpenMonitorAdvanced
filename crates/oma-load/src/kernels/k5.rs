//! K5, exact 64-bit modular NTT (§4.1, DA9): the integer counterpart of the FFT kernels,
//! over the prime field P = 2147483641 · 2^32 + 1. Nothing is rounded, so the spectrum
//! digest is compared with the reference without any tolerance, and the kernel also checks
//! itself every iteration: the sum of the spectrum must be N·x0 and the inverse must give
//! the input back bit for bit. Radix-2, in place, on `u64` words with Montgomery products.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use oma_ipc::load::DataSize;

use crate::kernel::{
    Check, Kernel, KernelError, KernelFactory, PhaseShared, RefFailure, WorkerCtx,
};
use crate::rng::Xoshiro256ss;

/// 0x7FFF_FFF9_0000_0001 = 2147483641 · 2^32 + 1, prime and below 2^63.
pub const P: u64 = 9_223_372_006_790_004_737;
/// A generator of the multiplicative group (P - 1 = 2^32 · 2699 · 795659).
pub const G: u64 = 3;
pub const M61: u64 = (1 << 61) - 1;
/// K5 `ram` uses at most 2^24 points, a 128 MiB work buffer (DA9).
const MAX_RAM_POINTS: usize = 1 << 24;
/// Bytes per point of the whole footprint: input, work buffer and twiddles, 8 each.
const BYTES_PER_POINT: u64 = 24;
/// DB2: the NTT length of `DataSize::Fixed`, the same on every machine.
pub const FIXED_NTT_N: usize = 1 << 15;

// Montgomery arithmetic with R = 2^64. P < 2^63, so sums of two residues never overflow.
const PINV: u64 = {
    let mut inv = P;
    let mut i = 0;
    while i < 6 {
        inv = inv.wrapping_mul(2u64.wrapping_sub(P.wrapping_mul(inv)));
        i += 1;
    }
    inv
};
const R_MOD_P: u128 = (1u128 << 64) % P as u128;
const R2: u64 = (R_MOD_P * R_MOD_P % P as u128) as u64;

/// a·b/R mod P for a, b < P, in [0, P).
#[inline(always)]
fn mont_mul(a: u64, b: u64) -> u64 {
    let t = a as u128 * b as u128;
    let m = (t as u64).wrapping_mul(PINV);
    let mp_hi = ((m as u128 * P as u128) >> 64) as u64;
    let t_hi = (t >> 64) as u64;
    if t_hi < mp_hi {
        t_hi.wrapping_sub(mp_hi).wrapping_add(P)
    } else {
        t_hi - mp_hi
    }
}

/// `a` in Montgomery form: a·R mod P.
fn to_mont(a: u64) -> u64 {
    mont_mul(a, R2)
}

/// a·b mod P, for a, b < P: the 128-bit product reduced (two Montgomery steps).
pub fn mul_mod(a: u64, b: u64) -> u64 {
    mont_mul(mont_mul(a, b), R2)
}

fn pow_mod(mut base: u64, mut exp: u64) -> u64 {
    let mut r = 1;
    while exp > 0 {
        if exp & 1 == 1 {
            r = mul_mod(r, base);
        }
        base = mul_mod(base, base);
        exp >>= 1;
    }
    r
}

#[inline(always)]
fn add_mod(a: u64, b: u64) -> u64 {
    let s = a + b;
    if s >= P {
        s - P
    } else {
        s
    }
}

#[inline(always)]
fn sub_mod(a: u64, b: u64) -> u64 {
    if a >= b {
        a - b
    } else {
        a + P - b
    }
}

/// The root of unity of order `n` (a power of 2, at most 2^32).
fn root(n: usize) -> u64 {
    pow_mod(G, (P - 1) / n as u64)
}

/// Twiddles of every stage in Montgomery form: `tw[half + j]` = w_(2·half)^j.
struct Twiddles(Vec<u64>);

impl Twiddles {
    fn new(n: usize) -> Result<Self, KernelError> {
        let mut tw = alloc(n)?;
        let mut half = 1;
        while half < n {
            let w = to_mont(root(2 * half));
            let mut cur = to_mont(1);
            for t in &mut tw[half..2 * half] {
                *t = cur;
                cur = mont_mul(cur, w);
            }
            half *= 2;
        }
        Ok(Self(tw))
    }
}

/// Elements between two ticks in the passes outside the butterflies, so that a `ram`-size
/// iteration (2^24 points) keeps bumping the beat and polling `quit`.
const CHUNK: usize = 1 << 18;

/// `len` zeroed words, or `Insufficient` when the memory is not there (never aborts).
/// K5 is not in DA10's list, so a failure is `Insufficient`, not `Memory(next)`: the size
/// already comes from the quota, with the whole footprint accounted for.
fn alloc(len: usize) -> Result<Vec<u64>, KernelError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| KernelError::Insufficient)?;
    v.resize(len, 0);
    Ok(v)
}

/// Forward transform in place; `tick` runs before every stage and `false` from it stops
/// the transform (the data is then unusable).
fn forward(a: &mut [u64], tw: &Twiddles, tick: &mut impl FnMut() -> bool) -> bool {
    let n = a.len();
    let shift = 64 - n.ilog2();
    for i in 0..n {
        if i % CHUNK == 0 && !tick() {
            return false;
        }
        let j = ((i as u64).reverse_bits() >> shift) as usize;
        if i < j {
            a.swap(i, j);
        }
    }
    let mut half = 1;
    while half < n {
        if !tick() {
            return false;
        }
        for chunk in a.chunks_exact_mut(2 * half) {
            let (lo, hi) = chunk.split_at_mut(half);
            for ((x, y), &w) in lo.iter_mut().zip(hi.iter_mut()).zip(&tw.0[half..2 * half]) {
                let v = mont_mul(*y, w);
                let u = *x;
                *x = add_mod(u, v);
                *y = sub_mod(u, v);
            }
        }
        half *= 2;
    }
    true
}

/// The inverse: the forward transform, indices 1.. reversed, scaled by 1/N.
fn inverse(a: &mut [u64], tw: &Twiddles, tick: &mut impl FnMut() -> bool) -> bool {
    if !forward(a, tw, tick) {
        return false;
    }
    a[1..].reverse();
    let n_inv = to_mont(pow_mod(a.len() as u64 % P, P - 2));
    a.iter_mut().for_each(|x| *x = mont_mul(*x, n_inv));
    true
}

/// Forward NTT in place; `a.len()` is a power of 2, at least 2.
#[cfg(test)]
pub(crate) fn ntt_forward(a: &mut [u64]) {
    let tw = Twiddles::new(a.len()).expect("twiddles");
    forward(a, &tw, &mut || true);
}

/// Inverse NTT in place; `ntt_inverse(ntt_forward(x))` is `x`.
#[cfg(test)]
pub(crate) fn ntt_inverse(a: &mut [u64]) {
    let tw = Twiddles::new(a.len()).expect("twiddles");
    inverse(a, &tw, &mut || true);
}

fn mul_m61(a: u64, b: u64) -> u64 {
    let p = a as u128 * b as u128;
    let s = (p & M61 as u128) as u64 + (p >> 61) as u64;
    let s = (s & M61) + (s >> 61);
    if s >= M61 {
        s - M61
    } else {
        s
    }
}

/// Σ Xₖ·(k+1) mod M61; `None` when `tick` asked to stop.
fn digest(spectrum: &[u64], tick: &mut impl FnMut() -> bool) -> Option<u64> {
    let mut acc = 0u64;
    for (c, chunk) in spectrum.chunks(CHUNK).enumerate() {
        if !tick() {
            return None;
        }
        for (k, &x) in chunk.iter().enumerate() {
            acc += mul_m61(x % M61, (c * CHUNK + k) as u64 + 1);
            if acc >= M61 {
                acc -= M61;
            }
        }
    }
    Some(acc)
}

/// The largest power of 2 not above `v`, and at least `min`.
fn pow2_floor(v: u64, min: u64) -> usize {
    (1u64 << v.max(min).ilog2()) as usize
}

/// DA9, `l2`: 8·N <= 0.5 of the L2 of a thread.
fn n_l2(ctx: &WorkerCtx) -> usize {
    pow2_floor(ctx.budget.l2_thread / 2 / 8, 1024)
}

/// DA9, `l3`: 8·N <= 0.5 of the L3 share (the L2 of a thread without an L3).
fn n_l3(ctx: &WorkerCtx) -> usize {
    let share = match ctx.budget.l3_share {
        0 => ctx.budget.l2_thread,
        s => s,
    };
    pow2_floor(share / 2 / 8, 1024)
}

/// DA9, `ram`: 2^24 points (128 MiB) when the whole footprint fits the memory per thread,
/// otherwise the largest power of 2 that does. A `ram` run smaller than the `l3` size would
/// be no memory test at all, so that is `Insufficient` too (the extra `n < n_l3` rejection).
fn n_ram(ctx: &WorkerCtx) -> Result<usize, KernelError> {
    let n = pow2_floor(ctx.budget.ram_per_thread / BYTES_PER_POINT, 1).min(MAX_RAM_POINTS);
    if (n as u64) * BYTES_PER_POINT > ctx.budget.ram_per_thread || n < n_l3(ctx) {
        return Err(KernelError::Insufficient);
    }
    Ok(n)
}

pub(crate) struct K5 {
    tw: Twiddles,
    input: Vec<u64>,
    work: Vec<u64>,
    shared: Arc<PhaseShared>,
    /// Test hook: (word of the spectrum, bit) flipped once right after the forward
    /// transform, before the sum check.
    #[cfg(test)]
    pub(crate) flip_before_sum: Option<(usize, u32)>,
    /// Test hook: flipped once after the sum check, before the digest.
    #[cfg(test)]
    pub(crate) flip: Option<(usize, u32)>,
}

impl K5 {
    pub(crate) fn new(ctx: &WorkerCtx) -> Result<Self, KernelError> {
        let n = match ctx.size {
            DataSize::Fixed => FIXED_NTT_N,
            DataSize::L3 => n_l3(ctx),
            DataSize::Ram => n_ram(ctx)?,
            _ => n_l2(ctx),
        };
        let mut input = alloc(n)?;
        let work = alloc(n)?;
        let tw = Twiddles::new(n)?;
        let mut rng = Xoshiro256ss::new(ctx.seed);
        // Never null: a zero becomes 1.
        input
            .iter_mut()
            .for_each(|x| *x = (rng.next_u64() % P).max(1));
        Ok(Self {
            tw,
            input,
            work,
            shared: ctx.shared.clone(),
            #[cfg(test)]
            flip_before_sum: None,
            #[cfg(test)]
            flip: None,
        })
    }
}

impl Kernel for K5 {
    fn iterate(&mut self, beat: &AtomicU64) -> Check {
        let quit = &self.shared.quit;
        let mut tick = || {
            beat.fetch_add(1, Ordering::Relaxed);
            !quit.load(Ordering::Relaxed)
        };
        for (w, i) in self.work.chunks_mut(CHUNK).zip(self.input.chunks(CHUNK)) {
            if !tick() {
                return Check::Ok;
            }
            w.copy_from_slice(i);
        }
        if !forward(&mut self.work, &self.tw, &mut tick) {
            return Check::Ok;
        }
        #[cfg(test)]
        if let Some((word, bit)) = self.flip_before_sum.take() {
            self.work[word] ^= 1 << bit;
        }
        let mut sum = 0;
        for chunk in self.work.chunks(CHUNK) {
            if !tick() {
                return Check::Ok;
            }
            sum = chunk.iter().fold(sum, |s, &x| add_mod(s, x));
        }
        let expected = mul_mod(self.work.len() as u64 % P, self.input[0]);
        if sum != expected {
            return Check::Mismatch {
                expected,
                actual: sum,
            };
        }
        #[cfg(test)]
        if let Some((word, bit)) = self.flip.take() {
            self.work[word] ^= 1 << bit;
        }
        let Some(d) = digest(&self.work, &mut tick) else {
            return Check::Ok;
        };
        if !inverse(&mut self.work, &self.tw, &mut tick) {
            return Check::Ok;
        }
        for (w, i) in self.work.chunks(CHUNK).zip(self.input.chunks(CHUNK)) {
            if !tick() {
                return Check::Ok;
            }
            if let Some(k) = (0..w.len()).find(|&k| w[k] != i[k]) {
                return Check::Mismatch {
                    expected: i[k],
                    actual: w[k],
                };
            }
        }
        Check::Digest(d)
    }
}

pub(crate) struct K5Factory;

impl KernelFactory for K5Factory {
    fn reference(&self, ctx: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        Some((|| {
            let mut k = K5::new(ctx)?;
            match k.iterate(&AtomicU64::new(0)) {
                Check::Digest(d) => Ok(vec![d]),
                // A stop during the reference: the engine is ending the phase anyway.
                Check::Ok => Err(RefFailure::Invalid("stopped".into())),
                // The exact checks failing in the reference are a defect of the code.
                _ => Err(RefFailure::Invalid(
                    "the NTT checks failed in the reference".into(),
                )),
            }
        })())
    }

    fn worker(&self, ctx: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        Ok(Box::new(K5::new(ctx)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::ThreadBudget;

    const KIB: u64 = 1024;

    fn ctx(size: DataSize, seed: u64) -> WorkerCtx {
        WorkerCtx {
            isa: oma_ipc::load::Isa::Sse2,
            size,
            budget: ThreadBudget {
                l1d: 32 * KIB,
                l2_thread: 64 * KIB, // 4096 points
                l3_share: 128 * KIB, // 8192 points
                ram_per_thread: 16384 * BYTES_PER_POINT,
            },
            seed,
            worker: 0,
            workers: 1,
            patterns: Vec::new(),
            shared: Arc::new(PhaseShared::default()),
        }
    }

    fn pow_u128(mut b: u128, mut e: u128, m: u128) -> u128 {
        let mut r = 1;
        b %= m;
        while e > 0 {
            if e & 1 == 1 {
                r = r * b % m;
            }
            b = b * b % m;
            e >>= 1;
        }
        r
    }

    fn is_prime(n: u64) -> bool {
        let n = n as u128;
        let (mut d, mut s) = (n - 1, 0);
        while d % 2 == 0 {
            d /= 2;
            s += 1;
        }
        [2u128, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37]
            .iter()
            .all(|&a| {
                let mut x = pow_u128(a, d, n);
                if x == 1 || x == n - 1 {
                    return true;
                }
                (1..s).any(|_| {
                    x = x * x % n;
                    x == n - 1
                })
            })
    }

    #[test]
    fn p_is_prime_and_g_generates() {
        assert_eq!(P, 0x7FFF_FFF9_0000_0001);
        assert_eq!(P, 2147483641 * (1u64 << 32) + 1);
        assert_eq!(2699u64 * 795659, 2147483641);
        assert!(is_prime(P));
        for q in [2u64, 2699, 795659] {
            assert_ne!(pow_mod(G, (P - 1) / q), 1, "q = {q}");
        }
        // The arithmetic agrees with plain 128-bit remainders.
        for (a, b) in [(P - 1, P - 1), (123456789012345, P - 2), (1, 0), (P / 2, 7)] {
            assert_eq!(mul_mod(a, b) as u128, a as u128 * b as u128 % P as u128);
        }
    }

    #[test]
    fn ntt_matches_naive_for_n_8() {
        let mut rng = Xoshiro256ss::new(5);
        let x: Vec<u64> = (0..8).map(|_| rng.next_u64() % P).collect();
        let w = root(8);
        let naive: Vec<u64> = (0..8u64)
            .map(|k| {
                (0..8u64).fold(0, |s, j| {
                    add_mod(s, mul_mod(x[j as usize], pow_mod(w, j * k)))
                })
            })
            .collect();
        let mut a = x.clone();
        ntt_forward(&mut a);
        assert_eq!(a, naive);
        ntt_inverse(&mut a);
        assert_eq!(a, x);
    }

    #[test]
    fn ntt_round_trip_is_exact() {
        let mut rng = Xoshiro256ss::new(9);
        let x: Vec<u64> = (0..1 << 12).map(|_| rng.next_u64() % P).collect();
        let mut a = x.clone();
        ntt_forward(&mut a);
        assert_ne!(a, x);
        ntt_inverse(&mut a);
        assert_eq!(a, x);
    }

    #[test]
    fn sum_identity_holds() {
        let mut rng = Xoshiro256ss::new(11);
        let x: Vec<u64> = (0..1 << 10).map(|_| rng.next_u64() % P).collect();
        let mut a = x.clone();
        ntt_forward(&mut a);
        let sum = a.iter().fold(0, |s, &v| add_mod(s, v));
        assert_eq!(sum, mul_mod(1 << 10, x[0]));
    }

    fn digest_of(k: &mut K5) -> u64 {
        match k.iterate(&AtomicU64::new(0)) {
            Check::Digest(d) => d,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn k5_digest_is_deterministic() {
        for size in [DataSize::L2, DataSize::L3, DataSize::Ram] {
            let mut a = K5::new(&ctx(size, 7)).unwrap();
            let first = digest_of(&mut a);
            assert_eq!(first, digest_of(&mut a), "{size:?}: again");
            let mut other = ctx(size, 7);
            other.worker = 3;
            other.workers = 4;
            assert_eq!(first, digest_of(&mut K5::new(&other).unwrap()), "{size:?}");
            assert_ne!(first, digest_of(&mut K5::new(&ctx(size, 8)).unwrap()));
            assert_eq!(K5Factory.reference(&ctx(size, 7)), Some(Ok(vec![first])));
        }
    }

    #[test]
    fn k5_sizes_follow_da9() {
        let c = ctx(DataSize::L2, 1);
        assert_eq!(
            (n_l2(&c), n_l3(&c), n_ram(&c).unwrap()),
            (4096, 8192, 16384)
        );
        let mut big = c.clone();
        big.budget.ram_per_thread = u64::MAX;
        assert_eq!(n_ram(&big).unwrap(), 1 << 24);
        // Below one l3-sized footprint there is nothing to run.
        big.budget.ram_per_thread = 24 * 4096;
        assert_eq!(n_ram(&big), Err(KernelError::Insufficient));
    }

    #[test]
    fn k5_bit_flip_is_a_mismatch() {
        let expected = digest_of(&mut K5::new(&ctx(DataSize::L2, 7)).unwrap());
        let mut k = K5::new(&ctx(DataSize::L2, 7)).unwrap();
        k.flip = Some((5, 40));
        assert!(matches!(
            k.iterate(&AtomicU64::new(0)),
            Check::Mismatch { .. }
        ));
        // The hook fires once: the next iteration is right again.
        assert_eq!(digest_of(&mut k), expected);
    }

    #[test]
    fn a_flip_before_the_sum_check_is_caught_by_it() {
        let c = ctx(DataSize::L2, 7);
        let mut k = K5::new(&c).unwrap();
        let n = k.work.len() as u64;
        let expected = mul_mod(n, k.input[0]);
        k.flip_before_sum = Some((5, 40));
        match k.iterate(&AtomicU64::new(0)) {
            Check::Mismatch {
                expected: e,
                actual,
            } => {
                assert_eq!(e, expected);
                assert_ne!(actual, expected);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_flip_after_the_sum_check_never_passes() {
        // The digest is taken from the damaged spectrum and differs from the right one
        // (the helper below), and the round trip fails too: never a passing digest.
        let c = ctx(DataSize::L2, 7);
        let mut spectrum = K5::new(&c).unwrap().input.clone();
        ntt_forward(&mut spectrum);
        let right = digest(&spectrum, &mut || true).unwrap();
        spectrum[5] ^= 1 << 40;
        assert_ne!(digest(&spectrum, &mut || true).unwrap(), right);
        let mut k = K5::new(&c).unwrap();
        k.flip = Some((5, 40));
        assert!(matches!(
            k.iterate(&AtomicU64::new(0)),
            Check::Mismatch { .. }
        ));
    }

    #[test]
    fn a_raised_quit_ends_the_iteration_early() {
        let c = ctx(DataSize::L2, 7);
        let mut k = K5::new(&c).unwrap();
        c.shared.quit.store(true, Ordering::Relaxed);
        let beat = AtomicU64::new(0);
        assert_eq!(k.iterate(&beat), Check::Ok);
        assert_eq!(beat.load(Ordering::Relaxed), 1);
    }

    /// `cargo test -p oma-load --release k5_iteration_time -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing measurement, run in release"]
    fn k5_iteration_time() {
        let mut c = ctx(DataSize::L2, 7);
        c.budget.l2_thread = 1024 * KIB;
        let mut k = K5::new(&c).unwrap();
        let beat = AtomicU64::new(0);
        k.iterate(&beat);
        let t = std::time::Instant::now();
        for _ in 0..100 {
            k.iterate(&beat);
        }
        eprintln!(
            "l2 n={}: {:?} per iteration",
            k.work.len(),
            t.elapsed() / 100
        );
    }
}
