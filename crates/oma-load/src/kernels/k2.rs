//! K2, FFT in cache, and K3, FFT in RAM (§4.1, DA9, DA10): the same kernel with two
//! sizes. Each iteration copies the stored input into the work buffer, transforms it
//! forward, hashes the spectrum, transforms it back and hashes that. `FftCore` is also
//! what K4 runs, one size at a time.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use oma_ipc::load::DataSize;

use super::fft::{alloc_f64, digest, Fft};
use crate::kernel::{
    Check, Kernel, KernelError, KernelFactory, PhaseShared, RefFailure, WorkerCtx,
};
use crate::rng::Xoshiro256ss;
use crate::verify;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
/// K3 never uses more than this per thread (DA9), and DA10 never reduces below the floor.
pub(crate) const RAM_CAP: u64 = 1024 * MIB;
pub(crate) const RAM_FLOOR: u64 = 256 * MIB;
/// Bytes per point of the whole footprint: input, work buffer and twiddles, 16 each.
const BYTES_PER_POINT: u64 = 48;
/// DA7: the tolerances of the reference's own checks.
const SUM_TOLERANCE: f64 = 1e-9;
const ROUND_TRIP_TOLERANCE: f64 = 1e-9;

#[cfg(test)]
thread_local! {
    /// Test seam: the (cap, floor) in force on this thread. Tests default to no floor, so
    /// they can run with small sizes; the floor tests set the real one.
    pub(crate) static LIMITS: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((RAM_CAP, 0)) };
}

/// The (cap, floor) of the memory per thread of K3 and K4.
fn limits() -> (u64, u64) {
    #[cfg(test)]
    return LIMITS.with(|l| l.get());
    #[cfg(not(test))]
    (RAM_CAP, RAM_FLOOR)
}

/// DA9 and DA10: the memory per thread K3 and K4 use, at most 1 GiB. Below 256 MiB there
/// is no size to run at: `Insufficient`, so the engine asks for one thread per physical
/// core and then skips the phase, before anything is allocated.
pub(crate) fn ram_share(ram_per_thread: u64) -> Result<u64, KernelError> {
    let (cap, floor) = limits();
    match ram_per_thread.min(cap) {
        share if share < floor => Err(KernelError::Insufficient),
        share => Ok(share),
    }
}

/// The largest power of 2 not above `v`, and at least `min`.
fn pow2_floor(v: u64, min: u64) -> usize {
    (1u64 << v.max(min).ilog2()) as usize
}

/// DA9, K2 `l1`: 16·N <= L1d/2, at least 64 points.
pub(crate) fn n_l1(ctx: &WorkerCtx) -> usize {
    pow2_floor(ctx.budget.l1d / 2 / 16, 64)
}

/// DA9, K2 `l2`: 16·N <= 0.6 of the L2 of a thread, at least 1024 points.
fn n_l2(ctx: &WorkerCtx) -> usize {
    pow2_floor((ctx.budget.l2_thread as f64 * 0.6) as u64 / 16, 1024)
}

/// DA9, K3: the memory per thread (at most 1 GiB) holds the whole footprint, which is three
/// times the 16·N of the work buffer: the stored input and the twiddles come with it, so
/// the plan's RAM share stays true.
pub(crate) fn n_ram(share: u64) -> usize {
    pow2_floor(share / BYTES_PER_POINT, 64)
}

/// DA10: half of what was tried, or `Insufficient` when that would go below the floor.
pub(crate) fn memory_error(tried: u64) -> KernelError {
    match tried / 2 {
        next if next >= limits().1 => KernelError::Memory(next),
        _ => KernelError::Insufficient,
    }
}

/// The data of one worker: the stored input and the work buffer for up to `max_n` points,
/// and the transform. The current size `n` can change (K4); the buffers are prefixes.
pub(crate) struct FftCore {
    fft: Fft,
    in_re: Vec<f64>,
    in_im: Vec<f64>,
    re: Vec<f64>,
    im: Vec<f64>,
    n: usize,
    seed: u64,
    shared: Arc<PhaseShared>,
    /// Test hook: (word of the spectrum, bit) flipped once between the two transforms.
    #[cfg(test)]
    pub(crate) flip: Option<(usize, u32)>,
}

impl FftCore {
    /// `tried` is the memory per thread this size comes from (K3, K4); an allocation
    /// that fails then gives `Memory` with half of it (DA10). Without it (the caches) a
    /// failure is `Insufficient`.
    pub(crate) fn new(
        ctx: &WorkerCtx,
        max_n: usize,
        tried: Option<u64>,
    ) -> Result<Self, KernelError> {
        let fail = |e| match (e, tried) {
            (KernelError::Insufficient, Some(t)) => memory_error(t),
            (e, _) => e,
        };
        let fft = Fft::new(max_n, ctx.isa).map_err(fail)?;
        let buf = || alloc_f64(max_n).map_err(fail);
        let mut core = Self {
            fft,
            in_re: buf()?,
            in_im: buf()?,
            re: buf()?,
            im: buf()?,
            n: max_n,
            seed: ctx.seed,
            shared: ctx.shared.clone(),
            #[cfg(test)]
            flip: None,
        };
        core.set_size(max_n);
        Ok(core)
    }

    /// Works on the first `n` points from now on, with their input from the seed. Values
    /// are in 0.25..1 in magnitude with either sign: never zero, denormal or infinite.
    pub(crate) fn set_size(&mut self, n: usize) {
        assert!(n.is_power_of_two() && n <= self.in_re.len());
        self.n = n;
        let mut rng = Xoshiro256ss::new(self.seed);
        let mut value = move || {
            let u = rng.next_u64();
            let magnitude = 0.25 + 0.75 * ((u >> 12) as f64 / (1u64 << 52) as f64);
            if u & 1 == 0 {
                magnitude
            } else {
                -magnitude
            }
        };
        self.in_re[..n].fill_with(&mut value);
        self.in_im[..n].fill_with(&mut value);
    }

    /// One iteration: the digest of both transforms, or `None` when `quit` was raised
    /// (the work buffer is then unusable until the next call). With `verify` (the
    /// reference only) it also checks X0 against the sum of the input and the round trip
    /// against the input, and `Err` means `reference_invalid`.
    pub(crate) fn run(&mut self, beat: &AtomicU64, verify: bool) -> Result<Option<u64>, String> {
        let n = self.n;
        let quit = &self.shared.quit;
        let mut tick = || {
            beat.fetch_add(1, Ordering::Relaxed);
            !quit.load(Ordering::Relaxed)
        };
        tick();
        self.re[..n].copy_from_slice(&self.in_re[..n]);
        self.im[..n].copy_from_slice(&self.in_im[..n]);
        let (re, im) = (&mut self.re[..n], &mut self.im[..n]);
        if !self.fft.forward_with(re, im, &mut tick) {
            return Ok(None);
        }
        let spectrum = digest(re, im);
        if verify {
            let max = max_abs(&self.in_re[..n]).max(max_abs(&self.in_im[..n]));
            let limit = SUM_TOLERANCE * n as f64 * max;
            let sum = |v: &[f64]| v[..n].iter().sum::<f64>();
            if (re[0] - sum(&self.in_re)).abs() > limit || (im[0] - sum(&self.in_im)).abs() > limit
            {
                return Err("X0 differs from the sum of the input".into());
            }
        }
        #[cfg(test)]
        if let Some((word, bit)) = self.flip.take() {
            re[word] = f64::from_bits(re[word].to_bits() ^ (1 << bit));
        }
        if !self.fft.inverse_with(re, im, &mut tick) {
            return Ok(None);
        }
        if verify {
            let max = max_abs(&self.in_re[..n]).max(max_abs(&self.in_im[..n]));
            let far = |a: &[f64], b: &[f64]| {
                a.iter()
                    .zip(b)
                    .any(|(x, y)| (x - y).abs() > ROUND_TRIP_TOLERANCE * max)
            };
            if far(re, &self.in_re[..n]) || far(im, &self.in_im[..n]) {
                return Err("the round trip differs from the input".into());
            }
        }
        Ok(Some(verify::digest_words(&[spectrum, digest(re, im)])))
    }
}

fn max_abs(v: &[f64]) -> f64 {
    v.iter().fold(0.0, |m, x| m.max(x.abs()))
}

/// K2 or K3, by `ram`.
pub(crate) struct K2 {
    core: FftCore,
}

impl K2 {
    pub(crate) fn new(ctx: &WorkerCtx, ram: bool) -> Result<Self, KernelError> {
        let (n, tried) = if ram {
            let share = ram_share(ctx.budget.ram_per_thread)?;
            (n_ram(share), Some(share))
        } else if matches!(ctx.size, DataSize::L1 | DataSize::Auto) {
            (n_l1(ctx), None)
        } else {
            (n_l2(ctx), None)
        };
        Self::with_points(ctx, n, tried)
    }

    fn with_points(ctx: &WorkerCtx, n: usize, tried: Option<u64>) -> Result<Self, KernelError> {
        Ok(Self {
            core: FftCore::new(ctx, n, tried)?,
        })
    }
}

impl Kernel for K2 {
    fn iterate(&mut self, beat: &AtomicU64) -> Check {
        match self.core.run(beat, false) {
            Ok(Some(d)) => Check::Digest(d),
            // A stop left the iteration: nothing to compare.
            Ok(None) => Check::Ok,
            Err(_) => unreachable!("only the reference verifies"),
        }
    }
}

/// K2 (`ram` false) and K3 (`ram` true).
pub(crate) struct K2Factory {
    pub(crate) ram: bool,
}

impl KernelFactory for K2Factory {
    fn reference(&self, ctx: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        Some((|| {
            let mut k = K2::new(ctx, self.ram)?;
            match k.core.run(&AtomicU64::new(0), true) {
                Ok(Some(d)) => Ok(vec![d]),
                // A stop during the reference: the engine is ending the phase anyway, and
                // `Invalid` is not `Unsupported`, which would skip the phase with a notice.
                Ok(None) => Err(RefFailure::Invalid("stopped".into())),
                Err(why) => Err(RefFailure::Invalid(why)),
            }
        })())
    }

    fn worker(&self, ctx: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        Ok(Box::new(K2::new(ctx, self.ram)?))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::kernel::ThreadBudget;
    use crate::kernels::fft::tests::available;
    use oma_ipc::load::Isa;

    pub(crate) fn ctx(isa: Isa, seed: u64, size: DataSize) -> WorkerCtx {
        WorkerCtx {
            isa,
            size,
            budget: ThreadBudget {
                l1d: 32 * KIB,
                l2_thread: 256 * KIB,
                l3_share: MIB,
                // 8192 points for K3.
                ram_per_thread: 8192 * BYTES_PER_POINT,
            },
            seed,
            worker: 0,
            workers: 1,
            patterns: Vec::new(),
            shared: Arc::new(PhaseShared::default()),
        }
    }

    fn digest_of(k: &mut K2) -> u64 {
        match k.iterate(&AtomicU64::new(0)) {
            Check::Digest(d) => d,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn k2_digest_is_deterministic_per_isa() {
        for isa in available() {
            for (size, ram) in [
                (DataSize::L1, false),
                (DataSize::L2, false),
                (DataSize::Ram, true),
            ] {
                let mut a = K2::new(&ctx(isa, 7, size), ram).unwrap();
                let first = digest_of(&mut a);
                assert_eq!(first, digest_of(&mut a), "{isa:?} {size:?}: again");
                let mut other = ctx(isa, 7, size);
                other.worker = 3;
                other.workers = 4;
                let mut b = K2::new(&other, ram).unwrap();
                assert_eq!(first, digest_of(&mut b), "{isa:?} {size:?}: another worker");
                let mut c = K2::new(&ctx(isa, 8, size), ram).unwrap();
                assert_ne!(first, digest_of(&mut c), "{isa:?} {size:?}: another seed");
                let reference = K2Factory { ram }.reference(&ctx(isa, 7, size));
                assert_eq!(reference, Some(Ok(vec![first])), "{isa:?} {size:?}");
            }
        }
    }

    #[test]
    fn k2_sizes_follow_da9() {
        let c = ctx(Isa::Sse2, 1, DataSize::L1);
        assert_eq!(n_l1(&c), 1024); // 16 KiB / 16
        assert_eq!(n_l2(&c), 8192); // 0.6 * 256 KiB = 157286 B / 16 = 9830
        assert_eq!(n_ram(8192 * BYTES_PER_POINT), 8192);
        assert_eq!(n_ram(ram_share(u64::MAX).unwrap()), 1 << 24); // capped at 1 GiB
    }

    #[test]
    fn k2_bit_flip_changes_the_digest() {
        for isa in available() {
            let expected = digest_of(&mut K2::new(&ctx(isa, 7, DataSize::L1), false).unwrap());
            let mut k = K2::new(&ctx(isa, 7, DataSize::L1), false).unwrap();
            k.core.flip = Some((5, 40));
            assert_ne!(digest_of(&mut k), expected, "{isa:?}");
            // The hook fires once: the next iteration is right again.
            assert_eq!(digest_of(&mut k), expected, "{isa:?}");
        }
    }

    #[test]
    fn reference_fails_when_the_transform_is_wrong() {
        // The checks run inside `reference()`: a corrupted twiddle must be caught there.
        let c = ctx(Isa::Sse2, 7, DataSize::L1);
        let mut k = K2::new(&c, false).unwrap();
        k.core.fft.corrupt_for_test();
        let Err(why) = k.core.run(&AtomicU64::new(0), true) else {
            panic!("a wrong transform passed the reference checks");
        };
        assert!(why.contains("differs"), "{why}");
        // The factory reports the reason as `Invalid`; a sound one gives a digest.
        assert!(matches!(
            K2Factory { ram: false }.reference(&c),
            Some(Ok(v)) if v.len() == 1
        ));
        // A reference interrupted by a stop is `Invalid("stopped")`, not `Unsupported`.
        c.shared.quit.store(true, Ordering::Relaxed);
        assert_eq!(
            K2Factory { ram: false }.reference(&c),
            Some(Err(RefFailure::Invalid("stopped".into())))
        );
    }

    #[test]
    fn a_raised_quit_ends_the_iteration_early() {
        let c = ctx(Isa::Sse2, 7, DataSize::L1);
        let mut k = K2::new(&c, false).unwrap();
        let beat = AtomicU64::new(0);
        assert!(matches!(k.iterate(&beat), Check::Digest(_)));
        let full = beat.load(Ordering::Relaxed);
        c.shared.quit.store(true, Ordering::Relaxed);
        beat.store(0, Ordering::Relaxed);
        assert_eq!(k.iterate(&beat), Check::Ok);
        // It stopped after the first tick (the bit reversal's), far short of a full run.
        assert!(beat.load(Ordering::Relaxed) <= 2 && full > 10);
    }

    #[test]
    fn k3_allocation_failure_is_memory_error() {
        // The real sizing, with the 1 GiB cap lifted so that a budget of u64::MAX / 2
        // asks for 2^57 points: the reservation fails up front, nothing is allocated.
        LIMITS.with(|l| l.set((u64::MAX, 0)));
        let mut c = ctx(Isa::Sse2, 7, DataSize::Ram);
        c.budget.ram_per_thread = u64::MAX / 2;
        assert!(matches!(
            K2::new(&c, true),
            Err(KernelError::Memory(m)) if m == u64::MAX / 4
        ));
        // At the floor there is nothing left to give up.
        LIMITS.with(|l| l.set((RAM_CAP, RAM_FLOOR)));
        assert_eq!(memory_error(RAM_FLOOR), KernelError::Insufficient);
        assert_eq!(memory_error(2 * RAM_FLOOR), KernelError::Memory(RAM_FLOOR));
    }

    #[test]
    fn k3_below_the_floor_is_insufficient() {
        LIMITS.with(|l| l.set((RAM_CAP, RAM_FLOOR)));
        let mut c = ctx(Isa::Sse2, 7, DataSize::Ram);
        c.budget.ram_per_thread = 100 * MIB;
        assert!(matches!(K2::new(&c, true), Err(KernelError::Insufficient)));
        assert_eq!(
            K2Factory { ram: true }.reference(&c),
            Some(Err(KernelError::Insufficient.into()))
        );
        // The caches have no floor.
        assert!(K2::new(&c, false).is_ok());
    }

    /// Iteration times of K2 `l1` and `l2` with the sizes of a typical CPU (48 KiB L1d,
    /// 1 MiB L2 per thread): `cargo test -p oma-load --release k2_iteration_times -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing measurement, run in release"]
    fn k2_iteration_times() {
        for isa in available() {
            for size in [DataSize::L1, DataSize::L2] {
                let mut c = ctx(isa, 7, size);
                c.budget.l1d = 48 * KIB;
                c.budget.l2_thread = MIB;
                let mut k = K2::new(&c, false).unwrap();
                let beat = AtomicU64::new(0);
                k.iterate(&beat);
                let t = std::time::Instant::now();
                for _ in 0..200 {
                    k.iterate(&beat);
                }
                eprintln!(
                    "{isa:?} {size:?} n={}: {:?} per iteration",
                    k.core.n,
                    t.elapsed() / 200
                );
            }
        }
    }
}
