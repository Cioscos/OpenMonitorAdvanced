//! The benchmark loads that K8 does not have on its own (DB2): `Hash` (SHA-256 and CRC32C of
//! one 1 MiB buffer), `Compress` (the LZ round trip of four 256 KiB blocks) and `Sort` (the
//! quicksort of 262 144 `u32`). They reuse K8's code and its known vectors; the size is
//! fixed and the instruction set does not change the work. Every iteration starts again
//! from the seed's data, so the digest depends only on the seed.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::{Check, Kernel, KernelError, KernelFactory, RefFailure, WorkerCtx};
use crate::kernels::k1::crc::{crc32c, crc32c_u64};
use crate::kernels::k8::{
    check_vectors, fold_bytes, has_sha, lz, make_text, mix, sha256, sort, TEXT_BYTES,
};
use crate::rng::Xoshiro256ss;

/// Bytes hashed per `Hash` iteration.
pub const BENCH_BYTES: usize = 1 << 20;
/// Elements sorted per `Sort` iteration.
pub const BENCH_SORT_LEN: usize = 262_144;
/// Blocks of `TEXT_BYTES` per `Compress` iteration: `BENCH_BYTES` in all.
const BLOCKS: usize = BENCH_BYTES / TEXT_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Hash,
    Compress,
    Sort,
}

pub(crate) struct Bench {
    kind: Kind,
    sha: bool,
    /// `Hash`: one buffer. `Compress`: the blocks, one after the other.
    bytes: Vec<u8>,
    src: Vec<u32>,
    work: Vec<u32>,
    /// Test hook: (index, bit) of the input flipped once, before the work.
    #[cfg(test)]
    pub(crate) flip: Option<(usize, u32)>,
}

impl Bench {
    pub(crate) fn new(kind: Kind, ctx: &WorkerCtx) -> Self {
        let mut rng = Xoshiro256ss::new(ctx.seed);
        let (mut bytes, mut src) = (Vec::new(), Vec::new());
        match kind {
            Kind::Hash => {
                bytes = vec![0u8; BENCH_BYTES];
                bytes
                    .chunks_exact_mut(8)
                    .for_each(|c| c.copy_from_slice(&rng.next_u64().to_le_bytes()));
            }
            Kind::Compress => (0..BLOCKS).for_each(|_| bytes.extend(make_text(&mut rng))),
            Kind::Sort => src = (0..BENCH_SORT_LEN).map(|_| rng.next_u64() as u32).collect(),
        }
        Self {
            kind,
            sha: has_sha(),
            bytes,
            work: vec![0; src.len()],
            src,
            #[cfg(test)]
            flip: None,
        }
    }

    fn hash(&self) -> Check {
        let crc = self.bytes.chunks_exact(8).fold(0xFFFF_FFFFu32, |c, w| {
            crc32c_u64(c, u64::from_le_bytes(w.try_into().unwrap()))
        });
        let sha = fold_bytes(&sha256::sha256(&self.bytes, self.sha));
        Check::Digest(mix(1, sha) ^ mix(3, u64::from(crc)))
    }

    fn compress(&self) -> Check {
        let mut digest = 0;
        for (i, block) in self.bytes.chunks_exact(TEXT_BYTES).enumerate() {
            let packed = lz::compress(block);
            match lz::decompress(&packed, TEXT_BYTES) {
                Ok(out) if out == block => {}
                Ok(out) => {
                    return Check::Mismatch {
                        expected: u64::from(crc32c(block)),
                        actual: u64::from(crc32c(&out)),
                    }
                }
                Err(_) => {
                    return Check::Mismatch {
                        expected: u64::from(crc32c(block)),
                        actual: 0,
                    }
                }
            }
            digest ^= mix(
                i as u64,
                u64::from(crc32c(&packed)) | (packed.len() as u64) << 32,
            );
        }
        Check::Digest(digest)
    }

    fn sort(&mut self) -> Check {
        self.work.copy_from_slice(&self.src);
        sort::sort(&mut self.work);
        let sum = |v: &[u32]| v.iter().map(|&x| u64::from(x)).sum::<u64>();
        let (want, got) = (sum(&self.src), sum(&self.work));
        if want != got || self.work.windows(2).any(|w| w[0] > w[1]) {
            return Check::Mismatch {
                expected: want,
                actual: got,
            };
        }
        let crc = self.work.chunks_exact(2).fold(0xFFFF_FFFFu32, |c, p| {
            crc32c_u64(c, u64::from(p[0]) | u64::from(p[1]) << 32)
        });
        Check::Digest(mix(5, u64::from(crc) | got << 32))
    }
}

impl Kernel for Bench {
    fn iterate(&mut self, beat: &AtomicU64) -> Check {
        #[cfg(test)]
        if let Some((i, bit)) = self.flip.take() {
            match self.kind {
                Kind::Sort => self.src[i] ^= 1 << bit,
                _ => self.bytes[i] ^= 1 << bit,
            }
        }
        beat.fetch_add(1, Ordering::Relaxed);
        let check = match self.kind {
            Kind::Hash => self.hash(),
            Kind::Compress => self.compress(),
            Kind::Sort => self.sort(),
        };
        beat.fetch_add(1, Ordering::Relaxed);
        check
    }
}

pub(crate) struct BenchFactory(pub(crate) Kind);

impl KernelFactory for BenchFactory {
    fn reference(&self, ctx: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        Some((|| {
            // The known vectors of K8: a failure is a defect of the code, not of a core.
            check_vectors().map_err(RefFailure::Invalid)?;
            match Bench::new(self.0, ctx).iterate(&AtomicU64::new(0)) {
                Check::Digest(d) => Ok(vec![d]),
                _ => Err(RefFailure::Invalid(
                    "the benchmark load's self-checks failed in the reference".into(),
                )),
            }
        })())
    }

    fn worker(&self, ctx: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        Ok(Box::new(Bench::new(self.0, ctx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{PhaseShared, ThreadBudget};
    use crate::kernels::{k2, k5, k7};
    use oma_ipc::load::{DataSize, Isa};
    use std::sync::Arc;

    const KINDS: [Kind; 3] = [Kind::Hash, Kind::Compress, Kind::Sort];

    fn ctx(isa: Isa, seed: u64, l2_thread: u64) -> WorkerCtx {
        WorkerCtx {
            isa,
            size: DataSize::Fixed,
            budget: ThreadBudget {
                l1d: 32 * 1024,
                l2_thread,
                l3_share: 4 * l2_thread,
                ram_per_thread: 1 << 30,
            },
            seed,
            worker: 0,
            workers: 1,
            patterns: Vec::new(),
            shared: Arc::new(PhaseShared::default()),
        }
    }

    fn digest(k: &mut dyn Kernel) -> u64 {
        match k.iterate(&AtomicU64::new(0)) {
            Check::Digest(d) => d,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn fixed_size_ignores_the_budget() {
        let small = ctx(Isa::Sse2, 7, 64 * 1024);
        let big = ctx(Isa::Sse2, 7, 8 * 1024 * 1024);
        assert_eq!(k2::FIXED_FFT_N, 4096);
        assert_eq!(k5::FIXED_NTT_N, 1 << 15);
        assert_eq!(k7::FIXED_GEMM_N, 256);
        let digests = |c: &WorkerCtx| {
            [
                digest(&mut k2::K2::new(c, false).unwrap()),
                digest(&mut k5::K5::new(c).unwrap()),
                digest(&mut k7::K7::new(c).unwrap()),
            ]
        };
        assert_eq!(digests(&small), digests(&big));
        // The cache-sized K2 at the same budgets does depend on it.
        let mut sized = big.clone();
        sized.size = DataSize::L2;
        assert_ne!(
            digest(&mut k2::K2::new(&sized, false).unwrap()),
            digests(&big)[0]
        );
    }

    #[test]
    fn hash_kernel_matches_known_vectors() {
        // SHA-256 "abc" and CRC32C "123456789", with and without SHA-NI.
        check_vectors().unwrap();
        let hex = |d: [u8; 32]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let want = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(hex(sha256::sha256(b"abc", false)), want);
        if has_sha() {
            assert_eq!(hex(sha256::sha256(b"abc", true)), want);
        }
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
        // The two SHA paths agree on the benchmark buffer.
        let k = Bench::new(Kind::Hash, &ctx(Isa::Sse2, 1, 1 << 18));
        assert_eq!(k.bytes.len(), BENCH_BYTES);
        assert_eq!(
            sha256::sha256(&k.bytes, false),
            sha256::sha256(&k.bytes, has_sha())
        );
    }

    #[test]
    fn compress_kernel_round_trips() {
        let k = Bench::new(Kind::Compress, &ctx(Isa::Sse2, 2, 1 << 18));
        assert_eq!(k.bytes.len(), BENCH_BYTES);
        for block in k.bytes.chunks_exact(TEXT_BYTES) {
            let packed = lz::compress(block);
            assert!(packed.len() < block.len() / 2);
            assert_eq!(lz::decompress(&packed, TEXT_BYTES).unwrap(), block);
        }
        assert!(matches!(k.compress(), Check::Digest(_)));
    }

    #[test]
    fn sort_kernel_sorts_and_keeps_the_sum() {
        let mut k = Bench::new(Kind::Sort, &ctx(Isa::Sse2, 3, 1 << 18));
        assert_eq!(k.src.len(), BENCH_SORT_LEN);
        assert!(matches!(k.sort(), Check::Digest(_)));
        let mut want = k.src.clone();
        want.sort_unstable();
        assert_eq!(k.work, want);
    }

    #[test]
    fn bench_kernels_are_deterministic() {
        for kind in KINDS {
            let first = digest(&mut Bench::new(kind, &ctx(Isa::Sse2, 7, 1 << 18)));
            let mut k = Bench::new(kind, &ctx(Isa::Avx2, 7, 1 << 22));
            assert_eq!(first, digest(&mut k), "{kind:?}");
            assert_eq!(first, digest(&mut k), "{kind:?} again");
            assert_ne!(
                first,
                digest(&mut Bench::new(kind, &ctx(Isa::Sse2, 8, 1 << 18)))
            );
            let reference = BenchFactory(kind).reference(&ctx(Isa::Sse2, 7, 1 << 18));
            assert_eq!(reference, Some(Ok(vec![first])), "{kind:?}");
        }
    }

    #[test]
    fn bench_kernel_bit_flip_is_a_mismatch() {
        for kind in KINDS {
            let reference = digest(&mut Bench::new(kind, &ctx(Isa::Sse2, 9, 1 << 18)));
            let mut k = Bench::new(kind, &ctx(Isa::Sse2, 9, 1 << 18));
            k.flip = Some((100, 5));
            let flipped = k.iterate(&AtomicU64::new(0));
            assert_ne!(flipped, Check::Digest(reference), "{kind:?}");
        }
    }

    /// `cargo test -p oma-load --release bench_iteration_times -- --ignored --nocapture`.
    /// One thread, the best instruction set, the fixed size; about 0.4 s per load.
    #[test]
    #[ignore = "timing measurement, run in release"]
    fn bench_iteration_times() {
        let isa = crate::kernels::fft::tests::available()[0];
        let c = ctx(isa, 7, 1 << 20);
        let beat = AtomicU64::new(0);
        let mut loads: Vec<(&str, Box<dyn Kernel>)> = vec![
            ("ntt (K5)", Box::new(k5::K5::new(&c).unwrap())),
            ("hash", Box::new(Bench::new(Kind::Hash, &c))),
            ("compress", Box::new(Bench::new(Kind::Compress, &c))),
            ("sort", Box::new(Bench::new(Kind::Sort, &c))),
            ("fft (K2)", Box::new(k2::K2::new(&c, false).unwrap())),
            ("gemm (K7)", Box::new(k7::K7::new(&c).unwrap())),
        ];
        for (name, k) in &mut loads {
            k.iterate(&beat);
            let (t, mut n) = (std::time::Instant::now(), 0u32);
            while t.elapsed() < std::time::Duration::from_millis(400) {
                k.iterate(&beat);
                n += 1;
            }
            let ms = t.elapsed().as_secs_f64() * 1e3 / f64::from(n);
            eprintln!("{isa:?} {name}: {ms:.3} ms per iteration");
        }
    }
}
