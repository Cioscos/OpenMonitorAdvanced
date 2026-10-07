//! K7, blocked GEMM in f64 (§4.1, DA9), Linpack-like: C = A·B with 64-wide blocks and
//! a register-tiled micro-kernel per instruction set. A and B hold integers in [-8, 8]
//! (never 0), so every product and every partial sum is an exact integer in f64 up to
//! n = 2048 (64·n < 2^53): FMA and mul+add give the same bits, and the digest of C is the
//! same on every instruction set. Each iteration also checks itself with Freivalds:
//! A·(B·r) must equal C·r exactly for a random 0/1 vector r.

use std::arch::x86_64::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use oma_ipc::load::{DataSize, Isa};

use crate::kernel::{
    Check, Kernel, KernelError, KernelFactory, PhaseShared, RefFailure, WorkerCtx,
};
use crate::rng::Xoshiro256ss;

const BLOCK: usize = 64;
/// DB2: the matrix order of `DataSize::Fixed`, the same on every machine.
pub const FIXED_GEMM_N: usize = 256;

/// Bytes of A, B and C together for an n×n problem: 3·8·n².
fn footprint(n: usize) -> u64 {
    24 * (n as u64) * (n as u64)
}

#[target_feature(enable = "sse2")]
unsafe fn mul_add_sse2(a: __m128d, b: __m128d, c: __m128d) -> __m128d {
    _mm_add_pd(c, _mm_mul_pd(a, b))
}

macro_rules! micro {
    ($name:ident, $feat:literal, $lanes:expr, $load:ident, $store:ident, $set1:ident, $madd:ident) => {
        /// c[0..8·G] += Σₖ a[k]·b[k·ldb ..][0..8·G].
        ///
        /// # Safety
        /// The CPU has the feature; `a` reads `kb` values, `b` reads `kb` rows of `8·G`
        /// values at stride `ldb`, `c` reads and writes `8·G` values.
        #[target_feature(enable = $feat)]
        unsafe fn $name<const G: usize>(
            a: *const f64,
            b: *const f64,
            ldb: usize,
            c: *mut f64,
            kb: usize,
        ) {
            const VPG: usize = 8 / $lanes;
            // SAFETY: the offsets stay inside the ranges listed in the function contract.
            unsafe {
                let mut acc = [[$set1(0.0); VPG]; G];
                for g in 0..G {
                    for v in 0..VPG {
                        acc[g][v] = $load(c.add(g * 8 + v * $lanes));
                    }
                }
                for k in 0..kb {
                    let av = $set1(*a.add(k));
                    let row = b.add(k * ldb);
                    for g in 0..G {
                        for v in 0..VPG {
                            let bv = $load(row.add(g * 8 + v * $lanes));
                            acc[g][v] = $madd(av, bv, acc[g][v]);
                        }
                    }
                }
                for g in 0..G {
                    for v in 0..VPG {
                        $store(c.add(g * 8 + v * $lanes), acc[g][v]);
                    }
                }
            }
        }
    };
}

micro!(
    micro_avx512,
    "avx512f",
    8,
    _mm512_loadu_pd,
    _mm512_storeu_pd,
    _mm512_set1_pd,
    _mm512_fmadd_pd
);
micro!(
    micro_avx2,
    "avx2,fma",
    4,
    _mm256_loadu_pd,
    _mm256_storeu_pd,
    _mm256_set1_pd,
    _mm256_fmadd_pd
);
micro!(
    micro_sse2,
    "sse2",
    2,
    _mm_loadu_pd,
    _mm_storeu_pd,
    _mm_set1_pd,
    mul_add_sse2
);

/// One row of C's block: c[0..w] += a[0..kb]·B-block, with `b` starting at the block's
/// first row and column; `w` is a multiple of 8. The ISA must be `supported`.
fn row_update(isa: Isa, a: &[f64], b: &[f64], ldb: usize, c: &mut [f64], kb: usize, w: usize) {
    assert!(supported(isa) && w.is_multiple_of(8) && a.len() >= kb && c.len() >= w);
    assert!(kb == 0 || b.len() >= (kb - 1) * ldb + w);
    let (ap, bp, cp) = (a.as_ptr(), b.as_ptr(), c.as_mut_ptr());
    let mut j = 0;
    macro_rules! run {
        ($f:ident) => {{
            while j + 32 <= w {
                // SAFETY: the ISA is supported (asserted above); the asserts bound the 32
                // columns at j inside `b` and `c`, and `a` has `kb` values.
                unsafe { $f::<4>(ap, bp.add(j), ldb, cp.add(j), kb) };
                j += 32;
            }
            while j < w {
                // SAFETY: as above, for 8 columns.
                unsafe { $f::<1>(ap, bp.add(j), ldb, cp.add(j), kb) };
                j += 8;
            }
        }};
    }
    match isa {
        Isa::Avx512 => run!(micro_avx512),
        Isa::Avx2 => run!(micro_avx2),
        Isa::Sse2 => run!(micro_sse2),
    }
}

fn supported(isa: Isa) -> bool {
    match isa {
        Isa::Avx512 => is_x86_feature_detected!("avx512f"),
        Isa::Avx2 => is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma"),
        Isa::Sse2 => is_x86_feature_detected!("sse2"),
    }
}

/// C = A·B for n×n row-major matrices (n a multiple of 8). `tick` runs before every block
/// row of C (n = 2048 is about 0.5 GFLOP per block row) and `false` from it stops the
/// product (C is then unusable). `mid` runs once, before about half of the block rows.
fn gemm(
    isa: Isa,
    n: usize,
    a: &[f64],
    b: &[f64],
    c: &mut [f64],
    tick: &mut impl FnMut() -> bool,
    mut mid: impl FnMut(&mut [f64]),
) -> bool {
    c.fill(0.0);
    let rows = n.div_ceil(BLOCK);
    for (bi, ii) in (0..n).step_by(BLOCK).enumerate() {
        if !tick() {
            return false;
        }
        if bi == rows / 2 {
            mid(c);
        }
        let ib = BLOCK.min(n - ii);
        for jj in (0..n).step_by(BLOCK) {
            let jb = BLOCK.min(n - jj);
            for kk in (0..n).step_by(BLOCK) {
                let kb = BLOCK.min(n - kk);
                for i in ii..ii + ib {
                    row_update(
                        isa,
                        &a[i * n + kk..i * n + kk + kb],
                        &b[kk * n + jj..],
                        n,
                        &mut c[i * n + jj..i * n + jj + jb],
                        kb,
                        jb,
                    );
                }
            }
        }
    }
    true
}

/// M·v for an n×n row-major matrix.
fn mat_vec(m: &[f64], v: &[f64], n: usize) -> Vec<f64> {
    m.chunks_exact(n)
        .map(|row| row.iter().zip(v).map(|(x, y)| x * y).sum())
        .collect()
}

/// Freivalds: is A·(B·r) = C·r, exactly? The values are small integers, so the sums are
/// exact in any order.
fn freivalds(a: &[f64], b: &[f64], c: &[f64], n: usize, r: &[f64]) -> Result<(), (f64, f64)> {
    let want = mat_vec(a, &mat_vec(b, r, n), n);
    let got = mat_vec(c, r, n);
    match want.iter().zip(&got).find(|(w, g)| w != g) {
        Some((w, g)) => Err((*w, *g)),
        None => Ok(()),
    }
}

/// An integer in [-8, 8] without 0.
fn small_int(x: u64) -> f64 {
    let v = (x % 16) as i64;
    (if v < 8 { v - 8 } else { v - 7 }) as f64
}

fn digest(c: &[f64]) -> u64 {
    c.iter().fold(0xcbf2_9ce4_8422_2325u64, |d, &x| {
        (d ^ (x as i64 as u64))
            .wrapping_mul(0x0000_0100_0000_01b3)
            .rotate_left(23)
    })
}

/// DA9: n a multiple of 8 with 24·n² <= 0.75·`budget` (at least 64).
fn n_for(budget: u64) -> usize {
    let n = ((0.75 * budget as f64 / 24.0).sqrt() as usize) / 8 * 8;
    n.max(64)
}

/// DA9, `l3`: the L2 of a thread stands in without an L3.
fn n_l3(ctx: &WorkerCtx) -> usize {
    n_for(match ctx.budget.l3_share {
        0 => ctx.budget.l2_thread,
        s => s,
    })
}

/// DA9, `ram`: n = 2048 (96 MiB), else 1024 (24 MiB), else `Insufficient`.
fn n_ram(ctx: &WorkerCtx) -> Result<usize, KernelError> {
    [2048, 1024]
        .into_iter()
        .find(|&n| footprint(n) <= ctx.budget.ram_per_thread)
        .ok_or(KernelError::Insufficient)
}

/// `len` zeroed values, or `Insufficient` (K7 is not in DA10's list; the size already
/// comes from the quota). Never aborts.
fn alloc(len: usize) -> Result<Vec<f64>, KernelError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len)
        .map_err(|_| KernelError::Insufficient)?;
    v.resize(len, 0.0);
    Ok(v)
}

pub(crate) struct K7 {
    isa: Isa,
    n: usize,
    a: Vec<f64>,
    b: Vec<f64>,
    c: Vec<f64>,
    seed: u64,
    iteration: u64,
    shared: Arc<PhaseShared>,
    /// Test hook: (word of C, bit) flipped once halfway through the product.
    #[cfg(test)]
    pub(crate) flip: Option<(usize, u32)>,
}

impl K7 {
    pub(crate) fn new(ctx: &WorkerCtx) -> Result<Self, KernelError> {
        if !supported(ctx.isa) {
            return Err(KernelError::Unsupported);
        }
        let n = match ctx.size {
            DataSize::Fixed => FIXED_GEMM_N,
            DataSize::L3 => n_l3(ctx),
            DataSize::Ram => n_ram(ctx)?,
            _ => n_for(ctx.budget.l2_thread),
        };
        let (mut a, mut b) = (alloc(n * n)?, alloc(n * n)?);
        let c = alloc(n * n)?;
        let mut rng = Xoshiro256ss::new(ctx.seed);
        a.iter_mut().for_each(|x| *x = small_int(rng.next_u64()));
        b.iter_mut().for_each(|x| *x = small_int(rng.next_u64()));
        Ok(Self {
            isa: ctx.isa,
            n,
            a,
            b,
            c,
            seed: ctx.seed,
            iteration: 0,
            shared: ctx.shared.clone(),
            #[cfg(test)]
            flip: None,
        })
    }
}

impl Kernel for K7 {
    fn iterate(&mut self, beat: &AtomicU64) -> Check {
        let quit = &self.shared.quit;
        let mut tick = || {
            beat.fetch_add(1, Ordering::Relaxed);
            !quit.load(Ordering::Relaxed)
        };
        #[cfg(test)]
        let mut flip = self.flip.take();
        let mid = |_c: &mut [f64]| {
            #[cfg(test)]
            if let Some((word, bit)) = flip.take() {
                _c[word] = f64::from_bits(_c[word].to_bits() ^ (1 << bit));
            }
        };
        if !gemm(
            self.isa,
            self.n,
            &self.a,
            &self.b,
            &mut self.c,
            &mut tick,
            mid,
        ) {
            return Check::Ok;
        }
        // r varies per iteration; the check is exact, so it never changes the digest.
        self.iteration += 1;
        let mut rng =
            Xoshiro256ss::new(self.seed ^ self.iteration.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let r: Vec<f64> = (0..self.n).map(|_| (rng.next_u64() & 1) as f64).collect();
        if let Err((expected, actual)) = freivalds(&self.a, &self.b, &self.c, self.n, &r) {
            return Check::Mismatch {
                expected: expected as i64 as u64,
                actual: actual as i64 as u64,
            };
        }
        Check::Digest(digest(&self.c))
    }
}

pub(crate) struct K7Factory;

impl KernelFactory for K7Factory {
    fn reference(&self, ctx: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        Some((|| {
            let mut k = K7::new(ctx)?;
            match k.iterate(&AtomicU64::new(0)) {
                Check::Digest(d) => Ok(vec![d]),
                // A stop during the reference: the engine is ending the phase anyway.
                Check::Ok => Err(RefFailure::Invalid("stopped".into())),
                _ => Err(RefFailure::Invalid(
                    "the Freivalds check failed in the reference".into(),
                )),
            }
        })())
    }

    fn worker(&self, ctx: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        Ok(Box::new(K7::new(ctx)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::ThreadBudget;

    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const ISAS: [(Isa, &str); 3] = [
        (Isa::Avx512, "avx512"),
        (Isa::Avx2, "avx2"),
        (Isa::Sse2, "sse2"),
    ];

    fn available() -> Vec<Isa> {
        ISAS.iter()
            .filter_map(|&(isa, name)| {
                if supported(isa) {
                    Some(isa)
                } else {
                    eprintln!("skipped: {name} not available");
                    None
                }
            })
            .collect()
    }

    fn ctx(isa: Isa, seed: u64, size: DataSize) -> WorkerCtx {
        WorkerCtx {
            isa,
            size,
            budget: ThreadBudget {
                l1d: 32 * KIB,
                l2_thread: 256 * KIB, // n = 88
                l3_share: 1024 * KIB, // n = 176
                ram_per_thread: 24 * MIB,
            },
            seed,
            worker: 0,
            workers: 1,
            patterns: Vec::new(),
            shared: Arc::new(PhaseShared::default()),
        }
    }

    fn digest_of(k: &mut K7) -> u64 {
        match k.iterate(&AtomicU64::new(0)) {
            Check::Digest(d) => d,
            other => panic!("{other:?}"),
        }
    }

    fn matrices(n: usize, seed: u64) -> (Vec<f64>, Vec<f64>) {
        let mut rng = Xoshiro256ss::new(seed);
        let mut m = || {
            (0..n * n)
                .map(|_| small_int(rng.next_u64()))
                .collect::<Vec<_>>()
        };
        (m(), m())
    }

    fn naive(a: &[f64], b: &[f64], n: usize) -> Vec<f64> {
        let mut c = vec![0.0; n * n];
        for i in 0..n {
            for k in 0..n {
                for j in 0..n {
                    c[i * n + j] += a[i * n + k] * b[k * n + j];
                }
            }
        }
        c
    }

    #[test]
    fn gemm_matches_naive_for_n_16() {
        // 16 exercises the 8-column tail; 72 the 64-wide block edge and the 32-column path.
        for n in [16, 72] {
            let (a, b) = matrices(n, 3);
            let want = naive(&a, &b, n);
            for isa in available() {
                let mut c = vec![9.0; n * n];
                assert!(gemm(isa, n, &a, &b, &mut c, &mut || true, |_| {}));
                assert_eq!(c, want, "{isa:?} n={n}");
            }
        }
    }

    #[test]
    fn freivalds_accepts_a_correct_product() {
        let n = 32;
        let (a, b) = matrices(n, 4);
        let c = naive(&a, &b, n);
        let r: Vec<f64> = (0..n).map(|i| (i % 2) as f64).collect();
        assert_eq!(freivalds(&a, &b, &c, n, &r), Ok(()));
    }

    #[test]
    fn freivalds_catches_a_single_wrong_element() {
        let n = 32;
        let (a, b) = matrices(n, 4);
        let mut c = naive(&a, &b, n);
        c[5 * n + 7] += 1.0;
        // With r = 1 in the damaged column the check cannot pass.
        assert!(freivalds(&a, &b, &c, n, &vec![1.0; n]).is_err());
    }

    #[test]
    fn k7_digest_is_identical_across_isas() {
        let mut seen: Option<(Isa, u64)> = None;
        for isa in available() {
            for size in [DataSize::L2, DataSize::L3] {
                let mut k = K7::new(&ctx(isa, 7, size)).unwrap();
                let d = digest_of(&mut k);
                assert_eq!(d, digest_of(&mut k), "{isa:?} {size:?}: again");
                assert_ne!(d, digest_of(&mut K7::new(&ctx(isa, 8, size)).unwrap()));
                assert_eq!(K7Factory.reference(&ctx(isa, 7, size)), Some(Ok(vec![d])));
                if size == DataSize::L2 {
                    match seen {
                        Some((first, fd)) => assert_eq!(d, fd, "{isa:?} vs {first:?}"),
                        None => seen = Some((isa, d)),
                    }
                }
            }
        }
    }

    #[test]
    fn k7_bit_flip_is_a_mismatch() {
        for isa in available() {
            let c = ctx(isa, 7, DataSize::L2);
            let expected = digest_of(&mut K7::new(&c).unwrap());
            let mut k = K7::new(&c).unwrap();
            // A high exponent bit, in a row finished before the half.
            k.flip = Some((3, 60));
            // Freivalds may miss it when r is 0 in that column; the digest never does.
            match k.iterate(&AtomicU64::new(0)) {
                Check::Mismatch { .. } => {}
                Check::Digest(d) => assert_ne!(d, expected, "{isa:?}"),
                other => panic!("{other:?}"),
            }
            assert_eq!(digest_of(&mut k), expected, "{isa:?}: the hook fires once");
        }
    }

    #[test]
    fn k7_sizes_follow_da9() {
        let c = ctx(Isa::Sse2, 1, DataSize::L2);
        assert_eq!(
            (n_for(c.budget.l2_thread), n_l3(&c), n_ram(&c)),
            (88, 176, Ok(1024))
        );
        let mut big = c.clone();
        big.budget.ram_per_thread = 96 * MIB;
        assert_eq!(n_ram(&big), Ok(2048));
        big.budget.ram_per_thread = 24 * MIB - 1;
        assert_eq!(n_ram(&big), Err(KernelError::Insufficient));
        big.budget.l3_share = 0;
        assert_eq!(n_l3(&big), 88);
        assert_eq!(n_for(1), 64);
    }

    #[test]
    fn a_raised_quit_ends_the_iteration_early() {
        let c = ctx(Isa::Sse2, 7, DataSize::L2);
        let mut k = K7::new(&c).unwrap();
        c.shared.quit.store(true, Ordering::Relaxed);
        let beat = AtomicU64::new(0);
        assert_eq!(k.iterate(&beat), Check::Ok);
        assert_eq!(beat.load(Ordering::Relaxed), 1);
    }

    /// `cargo test -p oma-load --release k7_iteration_time -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing measurement, run in release"]
    fn k7_iteration_time() {
        for isa in available() {
            for name in ["l2", "ram"] {
                let mut c = ctx(
                    isa,
                    7,
                    if name == "l2" {
                        DataSize::L2
                    } else {
                        DataSize::Ram
                    },
                );
                if name == "l2" {
                    c.budget.l2_thread = 1024 * KIB;
                } else {
                    c.budget.ram_per_thread = 96 * MIB;
                }
                let mut k = K7::new(&c).unwrap();
                let beat = AtomicU64::new(0);
                let reps = if name == "l2" { 20 } else { 1 };
                k.iterate(&beat);
                let t = std::time::Instant::now();
                for _ in 0..reps {
                    k.iterate(&beat);
                }
                eprintln!(
                    "{isa:?} {name} n={}: {:?} per iteration",
                    k.n,
                    t.elapsed() / reps
                );
            }
        }
    }
}
