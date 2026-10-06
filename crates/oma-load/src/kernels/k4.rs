//! K4, a blend of FFT sizes (§4.1, DA9): the K2 transform on every size from K2 `l1` up to
//! K3's, doubling, 20 s on each, then from the start again. One allocation of the largest
//! size serves them all, so a size change never allocates.

use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use super::k2::{n_l1, n_ram, FftCore, RAM_CAP};
use crate::kernel::{Check, Kernel, KernelError, KernelFactory, RefFailure, WorkerCtx};

/// Time on one size, by the worker's own clock.
const DWELL: Duration = Duration::from_secs(20);

/// The sizes of the cycle: K2 `l1` doubling up to K3's (or just `l1` with little memory).
fn sizes(ctx: &WorkerCtx) -> Vec<usize> {
    let (first, last) = (n_l1(ctx), n_l1(ctx).max(n_ram(ctx.budget.ram_per_thread)));
    std::iter::successors(Some(first), |&n| (n < last).then_some(n * 2)).collect()
}

pub(crate) struct K4 {
    core: FftCore,
    sizes: Vec<usize>,
    index: usize,
    since: Instant,
    dwell: Duration,
}

impl K4 {
    pub(crate) fn new(ctx: &WorkerCtx) -> Result<Self, KernelError> {
        let sizes = sizes(ctx);
        let tried = ctx.budget.ram_per_thread.min(RAM_CAP);
        let mut core = FftCore::new(ctx, *sizes.last().unwrap(), Some(tried))?;
        core.set_size(sizes[0]);
        Ok(Self {
            core,
            sizes,
            index: 0,
            since: Instant::now(),
            dwell: DWELL,
        })
    }
}

impl Kernel for K4 {
    fn iterate(&mut self, beat: &AtomicU64) -> Check {
        if self.since.elapsed() >= self.dwell {
            self.index = (self.index + 1) % self.sizes.len();
            self.core.set_size(self.sizes[self.index]);
            self.since = Instant::now();
        }
        match self.core.run(beat, false) {
            Ok(Some(digest)) => Check::DigestOf {
                variant: self.index as u32,
                digest,
            },
            // The only error is the reference's; `Ok` is how a stop leaves an iteration.
            _ => Check::Ok,
        }
    }
}

pub(crate) struct K4Factory;

impl KernelFactory for K4Factory {
    /// One digest per size, all of them now, one size at a time.
    fn reference(&self, ctx: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        Some((|| {
            let mut k = K4::new(ctx)?;
            let beat = AtomicU64::new(0);
            let mut digests = Vec::new();
            for (i, &n) in k.sizes.iter().enumerate() {
                k.core.set_size(n);
                match k.core.run(&beat, true) {
                    Ok(Some(d)) => digests.push(d),
                    Ok(None) => return Err(KernelError::Unsupported.into()),
                    Err(why) => {
                        tracing::error!(why, size = i, "FFT reference is invalid");
                        return Err(RefFailure::Invalid("reference_invalid".into()));
                    }
                }
            }
            Ok(digests)
        })())
    }

    fn worker(&self, ctx: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        Ok(Box::new(K4::new(ctx)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernels::fft::tests::available;
    use crate::kernels::k2::tests::ctx;
    use oma_ipc::load::DataSize;

    #[test]
    fn k4_cycles_sizes_from_l1_up() {
        for isa in available() {
            let c = ctx(isa, 7, DataSize::Auto);
            // l1 = 1024 points, K3 = 8192: four sizes.
            assert_eq!(sizes(&c), [1024, 2048, 4096, 8192]);
            let reference = K4Factory.reference(&c).unwrap().unwrap();
            assert_eq!(reference.len(), 4);
            assert!(reference.windows(2).all(|w| w[0] != w[1]));

            let mut k = K4::new(&c).unwrap();
            let beat = AtomicU64::new(0);
            // The first size is `l1`, kept until the dwell passes.
            assert_eq!(
                k.iterate(&beat),
                Check::DigestOf {
                    variant: 0,
                    digest: reference[0]
                }
            );
            // Then every iteration moves on, and goes back to the start after the last.
            k.dwell = Duration::ZERO;
            for expected in [1u32, 2, 3, 0, 1] {
                match k.iterate(&beat) {
                    Check::DigestOf { variant, digest } => {
                        assert_eq!(variant, expected, "{isa:?}");
                        assert_eq!(digest, reference[variant as usize], "{isa:?}");
                    }
                    other => panic!("{other:?}"),
                }
            }
        }
    }

    #[test]
    fn k4_bit_flip_changes_the_digest() {
        for isa in available() {
            let c = ctx(isa, 7, DataSize::Auto);
            let reference = K4Factory.reference(&c).unwrap().unwrap();
            let mut k = K4::new(&c).unwrap();
            k.core.flip = Some((3, 40));
            match k.iterate(&AtomicU64::new(0)) {
                Check::DigestOf { variant, digest } => {
                    assert_ne!(digest, reference[variant as usize], "{isa:?}")
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn k4_with_little_memory_stays_on_l1() {
        let mut c = ctx(Isa::Sse2, 7, DataSize::Auto);
        c.budget.ram_per_thread = 1024;
        assert_eq!(sizes(&c), [1024]);
    }

    use oma_ipc::load::Isa;
}
