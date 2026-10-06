//! K10, RAM test patterns (§4.1, DA7, DA9, DA10): one pass of one pattern per iteration,
//! rotating over `ctx.patterns` (all five when it is empty), on one `Region` of the
//! thread's share of the RAM quota. Self-checking: a wrong word is a `Mismatch`, there is
//! no reference.
//!
//! The memory is walked in blocks of 1 MiB; `beat` is bumped and `ctx.shared.quit` polled
//! once per block, so a pass over GiBs ends within a fraction of a second of a stop (the
//! iteration then returns `Ok`). Writes are non-temporal stores (`_mm256_stream_si256` for
//! AVX2 and AVX-512, which uses the AVX2 path; `_mm_stream_si128` for SSE2), followed by
//! `_mm_sfence` before the data is read back, so the reads come from DRAM and not from the
//! cache. Streaming stores need 32-byte alignment: `Region` chunks start on a page and a
//! block is 1 MiB, so every block is aligned.
//!
//! Patterns (v comes from the seed, `!v` is its complement):
//! - `moving_inversions`: write v; going up check v and write !v; going down check !v and
//!   write v;
//! - `modulo20`: for each offset 0..20 write v at the words whose index is that offset
//!   modulo 20 and !v elsewhere, then check (20 write and read sweeps: the longest pass);
//! - `random`: fill from the seed's generator, read back regenerating the same stream;
//! - `address`: every word holds its own address XOR the seed, written going up and
//!   checked going down (the blocks; inside a block both go up);
//! - `crc_copy`: fill the lower half from the seed's generator, copy it to the upper half
//!   in blocks of 1 MiB with a CRC32C taken while reading the source (compared with the
//!   CRC of what was generated) and again over the destination (compared with the one of
//!   the copy). `expected` and `actual` are then CRCs.

use std::arch::x86_64::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use oma_ipc::load::{Isa, RamPattern};

use super::k1::crc::crc32c_u64;
use super::k2;
use crate::kernel::{
    Check, Kernel, KernelError, KernelFactory, PhaseShared, RefFailure, WorkerCtx,
};
use crate::rng::Xoshiro256ss;
use crate::sys::memory::Region;

const MIB: u64 = 1 << 20;
const BLOCK_WORDS: usize = (MIB / 8) as usize;
/// A multiple of two blocks: `crc_copy` needs two equal halves.
const GRANULE: u64 = 2 * MIB;

type AllocFn = fn(u64) -> Result<Region, KernelError>;

#[derive(Clone, Copy)]
enum Path {
    Avx2,
    Sse2,
}

fn path_for(isa: Isa) -> Result<Path, KernelError> {
    match isa {
        Isa::Avx512 if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx2") => {
            Ok(Path::Avx2)
        }
        Isa::Avx2 if is_x86_feature_detected!("avx2") => Ok(Path::Avx2),
        Isa::Sse2 => Ok(Path::Sse2),
        _ => Err(KernelError::Unsupported),
    }
}

#[target_feature(enable = "avx2")]
unsafe fn stream_avx2<F: FnMut(usize) -> u64>(dst: &mut [u64], f: &mut F) {
    let p = dst.as_mut_ptr();
    for i in (0..dst.len()).step_by(4) {
        let (a, b, c, d) = (f(i), f(i + 1), f(i + 2), f(i + 3));
        let v = _mm256_set_epi64x(d as i64, c as i64, b as i64, a as i64);
        // SAFETY: `i + 4 <= len` (the caller checked a multiple of 4) and the pointer is
        // 32-byte aligned (checked by `stream_words`).
        _mm256_stream_si256(p.add(i).cast(), v);
    }
}

#[target_feature(enable = "sse2")]
unsafe fn stream_sse2<F: FnMut(usize) -> u64>(dst: &mut [u64], f: &mut F) {
    let p = dst.as_mut_ptr();
    for i in (0..dst.len()).step_by(2) {
        let (a, b) = (f(i), f(i + 1));
        let v = _mm_set_epi64x(b as i64, a as i64);
        // SAFETY: `i + 2 <= len` and the pointer is 16-byte aligned (see `stream_words`).
        _mm_stream_si128(p.add(i).cast(), v);
    }
}

/// Writes `f(i)` to word `i` of `dst` with non-temporal stores, in order, then fences.
fn stream_words<F: FnMut(usize) -> u64>(dst: &mut [u64], path: Path, mut f: F) {
    assert!((dst.as_ptr() as usize).is_multiple_of(32) && dst.len().is_multiple_of(4));
    match path {
        // SAFETY: `path_for` saw AVX2 on this CPU; the assert above gives the alignment
        // and length the stores need.
        Path::Avx2 => unsafe { stream_avx2(dst, &mut f) },
        // SAFETY: SSE2 is part of x86-64; alignment and length as above.
        Path::Sse2 => unsafe { stream_sse2(dst, &mut f) },
    }
    // SAFETY: SSE2 is part of x86-64. Orders the stores before the reads that follow.
    unsafe { _mm_sfence() };
}

/// The first word of `blk` that differs from `f(i)`, as a mismatch.
fn check_words<F: FnMut(usize) -> u64>(blk: &[u64], mut f: F) -> Result<(), Check> {
    for (i, &actual) in blk.iter().enumerate() {
        let expected = f(i);
        if actual != expected {
            return Err(Check::Mismatch { expected, actual });
        }
    }
    Ok(())
}

fn crc_of(blk: &[u64]) -> u32 {
    blk.iter().fold(0, |c, &w| crc32c_u64(c, w))
}

fn crc_mismatch(expected: u32, actual: u32) -> Check {
    Check::Mismatch {
        expected: expected as u64,
        actual: actual as u64,
    }
}

/// One pass over the blocks. An `Err` is the verdict; `Err(Check::Ok)` means a stop.
struct Pass<'a> {
    blocks: Vec<&'a mut [u64]>,
    path: Path,
    seed: u64,
    beat: &'a AtomicU64,
    quit: &'a AtomicBool,
    /// Test hook: (word over the whole region, bit) flipped once between the writes and the
    /// reads of the first sweep.
    #[cfg(test)]
    flip: Option<(usize, u32)>,
}

impl Pass<'_> {
    fn tick(&self) -> Result<(), Check> {
        self.beat.fetch_add(1, Ordering::Relaxed);
        if self.quit.load(Ordering::Relaxed) {
            Err(Check::Ok)
        } else {
            Ok(())
        }
    }

    fn hook(&mut self) {
        #[cfg(test)]
        if let Some((word, bit)) = self.flip.take() {
            self.blocks[word / BLOCK_WORDS][word % BLOCK_WORDS] ^= 1 << bit;
        }
    }

    fn write<F: FnMut(usize) -> u64>(&mut self, b: usize, f: F) -> Result<(), Check> {
        self.tick()?;
        stream_words(self.blocks[b], self.path, f);
        Ok(())
    }

    fn check<F: FnMut(usize) -> u64>(&self, b: usize, f: F) -> Result<(), Check> {
        self.tick()?;
        check_words(self.blocks[b], f)
    }

    fn run(&mut self, pattern: RamPattern) -> Result<(), Check> {
        let n = self.blocks.len();
        let v = Xoshiro256ss::new(self.seed).next_u64();
        let nv = !v;
        match pattern {
            RamPattern::MovingInversions => {
                for b in 0..n {
                    self.write(b, |_| v)?;
                }
                self.hook();
                for b in 0..n {
                    self.check(b, |_| v)?;
                    self.write(b, |_| nv)?;
                }
                for b in (0..n).rev() {
                    self.check(b, |_| nv)?;
                    self.write(b, |_| v)?;
                }
            }
            RamPattern::Modulo20 => {
                for offset in 0..20 {
                    let pick = |b: usize, j: usize| {
                        if (b * BLOCK_WORDS + j) % 20 == offset {
                            v
                        } else {
                            nv
                        }
                    };
                    for b in 0..n {
                        self.write(b, |j| pick(b, j))?;
                    }
                    self.hook();
                    for b in 0..n {
                        self.check(b, |j| pick(b, j))?;
                    }
                }
            }
            RamPattern::Random => {
                let mut rng = Xoshiro256ss::new(self.seed);
                for b in 0..n {
                    self.write(b, |_| rng.next_u64())?;
                }
                self.hook();
                let mut rng = Xoshiro256ss::new(self.seed);
                for b in 0..n {
                    self.check(b, |_| rng.next_u64())?;
                }
            }
            RamPattern::Address => {
                let seed = self.seed;
                for b in 0..n {
                    let base = self.blocks[b].as_ptr() as u64;
                    self.write(b, |j| (base + 8 * j as u64) ^ seed)?;
                }
                self.hook();
                for b in (0..n).rev() {
                    let base = self.blocks[b].as_ptr() as u64;
                    self.check(b, |j| (base + 8 * j as u64) ^ seed)?;
                }
            }
            RamPattern::CrcCopy => {
                let half = n / 2;
                let mut rng = Xoshiro256ss::new(self.seed);
                let mut generated = Vec::with_capacity(half);
                for b in 0..half {
                    let mut crc = 0;
                    self.write(b, |_| {
                        let w = rng.next_u64();
                        crc = crc32c_u64(crc, w);
                        w
                    })?;
                    generated.push(crc);
                }
                let mut copied = Vec::with_capacity(half);
                for b in 0..half {
                    self.tick()?;
                    let (lo, hi) = self.blocks.split_at_mut(half);
                    let src: &[u64] = lo[b];
                    let mut crc = 0;
                    stream_words(hi[b], self.path, |j| {
                        crc = crc32c_u64(crc, src[j]);
                        src[j]
                    });
                    if crc != generated[b] {
                        return Err(crc_mismatch(generated[b], crc));
                    }
                    copied.push(crc);
                }
                self.hook();
                for (b, &expected) in copied.iter().enumerate() {
                    self.tick()?;
                    let actual = crc_of(self.blocks[half + b]);
                    if actual != expected {
                        return Err(crc_mismatch(expected, actual));
                    }
                }
            }
        }
        Ok(())
    }
}

pub(crate) struct K10 {
    region: Region,
    path: Path,
    patterns: Vec<RamPattern>,
    next: usize,
    seed: u64,
    shared: Arc<PhaseShared>,
    /// Test hook: (word over the whole region, bit), handed to the next pass.
    #[cfg(test)]
    pub(crate) flip: Option<(usize, u32)>,
}

impl K10 {
    pub(crate) fn new(ctx: &WorkerCtx) -> Result<Self, KernelError> {
        Self::with_alloc(ctx, Region::alloc)
    }

    /// `alloc` is the test seam for a failing allocator. DA10: a failure at `tried` bytes
    /// gives `Memory` with half of it, `Insufficient` at the floor.
    fn with_alloc(ctx: &WorkerCtx, alloc: AllocFn) -> Result<Self, KernelError> {
        let path = path_for(ctx.isa)?;
        let tried = ctx.budget.ram_per_thread;
        if tried < k2::floor() {
            return Err(KernelError::Insufficient);
        }
        let region =
            alloc((tried & !(GRANULE - 1)).max(GRANULE)).map_err(|_| k2::memory_error(tried))?;
        Ok(Self {
            region,
            path,
            patterns: if ctx.patterns.is_empty() {
                vec![
                    RamPattern::MovingInversions,
                    RamPattern::Modulo20,
                    RamPattern::Random,
                    RamPattern::Address,
                    RamPattern::CrcCopy,
                ]
            } else {
                ctx.patterns.clone()
            },
            next: 0,
            seed: ctx.seed,
            shared: ctx.shared.clone(),
            #[cfg(test)]
            flip: None,
        })
    }
}

impl Kernel for K10 {
    fn iterate(&mut self, beat: &AtomicU64) -> Check {
        let pattern = self.patterns[self.next % self.patterns.len()];
        self.next += 1;
        let mut pass = Pass {
            blocks: self
                .region
                .chunks_mut()
                .flat_map(|c| c.chunks_exact_mut(BLOCK_WORDS))
                .collect(),
            path: self.path,
            seed: self.seed,
            beat,
            quit: &self.shared.quit,
            #[cfg(test)]
            flip: self.flip.take(),
        };
        pass.run(pattern).err().unwrap_or(Check::Ok)
    }
}

pub(crate) struct K10Factory;

impl KernelFactory for K10Factory {
    /// Self-checking: no reference.
    fn reference(&self, _ctx: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        None
    }

    fn worker(&self, ctx: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        Ok(Box::new(K10::new(ctx)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::ThreadBudget;
    use crate::kernels::k2::{LIMITS, RAM_CAP, RAM_FLOOR};
    use oma_ipc::load::DataSize;
    use std::time::Instant;

    const ALL: [RamPattern; 5] = [
        RamPattern::MovingInversions,
        RamPattern::Modulo20,
        RamPattern::Random,
        RamPattern::Address,
        RamPattern::CrcCopy,
    ];

    fn ctx(isa: Isa, pattern: RamPattern, ram: u64) -> WorkerCtx {
        WorkerCtx {
            isa,
            size: DataSize::Auto,
            budget: ThreadBudget {
                l1d: 32 << 10,
                l2_thread: 256 << 10,
                l3_share: MIB,
                ram_per_thread: ram,
            },
            seed: 5,
            worker: 0,
            workers: 1,
            patterns: vec![pattern],
            shared: Arc::new(PhaseShared::default()),
        }
    }

    fn isas() -> Vec<Isa> {
        [Isa::Avx512, Isa::Avx2, Isa::Sse2]
            .into_iter()
            .filter(|&i| path_for(i).is_ok())
            .collect()
    }

    #[test]
    fn each_pattern_passes_on_good_memory() {
        for isa in isas() {
            for p in ALL {
                let mut k = K10::new(&ctx(isa, p, 8 * MIB)).unwrap();

                assert_eq!(k.iterate(&AtomicU64::new(0)), Check::Ok, "{isa:?} {p:?}");
            }
        }
    }

    #[test]
    fn each_pattern_catches_a_flipped_bit() {
        for isa in isas() {
            for p in ALL {
                let mut k = K10::new(&ctx(isa, p, 8 * MIB)).unwrap();
                // A word of block 4: the destination half for `crc_copy`.
                k.flip = Some((4 * BLOCK_WORDS + 5, 17));
                let got = k.iterate(&AtomicU64::new(0));
                assert!(
                    matches!(got, Check::Mismatch { .. }),
                    "{isa:?} {p:?}: {got:?}"
                );
            }
        }
    }

    #[test]
    fn crc_copy_detects_a_corrupted_destination() {
        let mut k = K10::new(&ctx(Isa::Sse2, RamPattern::CrcCopy, 8 * MIB)).unwrap();
        k.flip = Some((7 * BLOCK_WORDS + 100, 0));
        match k.iterate(&AtomicU64::new(0)) {
            Check::Mismatch { expected, actual } => assert_ne!(expected, actual),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_stop_ends_the_pass() {
        let c = ctx(Isa::Sse2, RamPattern::Modulo20, 8 * MIB);
        c.shared.quit.store(true, Ordering::Relaxed);
        let mut k = K10::new(&c).unwrap();
        let t = Instant::now();
        assert_eq!(k.iterate(&AtomicU64::new(0)), Check::Ok);
        assert!(t.elapsed().as_millis() < 500);
    }

    #[test]
    fn allocation_failure_halves_then_skips() {
        const GIB: u64 = 1024 * MIB;
        LIMITS.with(|l| l.set((RAM_CAP, RAM_FLOOR)));
        // Fails above 300 MiB, so 1 GiB and 512 MiB fail; a success allocates only 2 MiB.
        let fails: AllocFn = |bytes| {
            if bytes > 300 * MIB {
                Err(KernelError::Insufficient)
            } else {
                Region::alloc(GRANULE)
            }
        };
        let try_with = |ram| K10::with_alloc(&ctx(Isa::Sse2, RamPattern::Random, ram), fails).err();
        assert_eq!(try_with(GIB), Some(KernelError::Memory(512 * MIB)));
        assert_eq!(try_with(512 * MIB), Some(KernelError::Memory(256 * MIB)));
        assert_eq!(try_with(256 * MIB), None);
        let fail_all: AllocFn = |_| Err(KernelError::Insufficient);
        let r = K10::with_alloc(&ctx(Isa::Sse2, RamPattern::Random, 256 * MIB), fail_all).err();
        assert_eq!(r, Some(KernelError::Insufficient));
        // Below the floor nothing is even tried.
        assert_eq!(try_with(128 * MIB), Some(KernelError::Insufficient));
        LIMITS.with(|l| l.set((RAM_CAP, 0)));
    }

    #[test]
    #[ignore = "throughput measurement, run with --release --nocapture"]
    fn pass_throughput() {
        for isa in isas() {
            for p in [RamPattern::Random, RamPattern::MovingInversions] {
                let mut k = K10::new(&ctx(isa, p, 8 * MIB)).unwrap();
                k.iterate(&AtomicU64::new(0));
                let t = Instant::now();
                for _ in 0..20 {
                    k.iterate(&AtomicU64::new(0));
                }
                let gbs = 20.0 * 8.0 * MIB as f64 / t.elapsed().as_secs_f64() / 1e9;
                eprintln!("{isa:?} {p:?}: {gbs:.2} GB/s of region per pass");
            }
        }
    }
}
