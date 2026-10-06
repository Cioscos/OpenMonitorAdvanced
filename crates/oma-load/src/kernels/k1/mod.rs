/******************************************************************************
 * FIRESTARTER - A Processor Stress Test Utility
 * Copyright (C) 2020-2023 TU Dresden, Center for Information Services and High
 * Performance Computing
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/\>.
 *
 * Contact: daniel.hackenberg@tu-dresden.de
 *****************************************************************************/

// Adapted from FIRESTARTER (https://github.com/tud-zih-energy/FIRESTARTER), commit 927ae17e55f3f90f7575f6a68630a366fde9c94e, src/firestarter/X86/Payload/FMAPayload.cpp and src/firestarter/X86/Payload/AVX512Payload.cpp: register roles and their initial values, L1 and L2 buffers, the L2 pointer reset after getL2LoopCount passes.
// Adapted from FIRESTARTER (https://github.com/tud-zih-energy/FIRESTARTER), commit 927ae17e55f3f90f7575f6a68630a366fde9c94e, src/firestarter/X86/Payload/X86Payload.cpp: initMemory.
// Modified for OpenMonitor Advanced: Rust intrinsics generated at build time instead of asmjit at run time; accumulators reset every block; L3/RAM items dropped.
// The modifications are part of OpenMonitor Advanced, GPL-3.0-or-later.

//! K1, maximum power (§4.1, DA6, DA8): FIRESTARTER's FMA instruction groups, unrolled to
//! about 1536 lines by `build.rs` into one function per instruction set.

pub mod crc;
#[cfg(test)]
mod groups;

use std::arch::x86_64::*;
use std::sync::atomic::{AtomicU64, Ordering};

use oma_ipc::load::Isa;

use crate::kernel::{Check, Kernel, KernelError, KernelFactory, RefFailure, WorkerCtx};
use crate::rng::Xoshiro256ss;
use crate::verify;

/// Vector accumulators: `x0`..`x6` take the lines in turn, `x7`..`x9` the second FMA of
/// the `REG` lines.
const ACCS: usize = 10;
/// Words past each zone that the displacements reach (at most 24): four cache lines.
const K1_PAD: usize = 32;
/// Passes of one block (DA6); SSE2 does fewer, being slower.
const PASSES: u32 = 16_384;
const PASSES_SSE2: u32 = 4096;
/// The beat moves every this many passes.
const BEAT_PASSES: u32 = 1024;
/// `initMemory`: the first `INIT_BLOCK` words are `0.25 + i * 8 * INIT_STEP`, the rest
/// copies of them. `INIT_STEP` is the value FMAPayload and AVX512Payload pass to it.
const INIT_BLOCK: usize = 1024;
const INIT_STEP: f64 = 0.279_489_959_82e-4;
const KIB: u64 = 1024;

// `k1_block_avx512`, `k1_block_avx2`, `k1_block_sse2` and their `L2_PER_PASS_*`.
include!(concat!(env!("OUT_DIR"), "/k1_payload.rs"));

/// The state a pass reads and writes: accumulators, the operands `a`, `b`, `c` and the
/// pass counter. Lanes past the instruction set's width are unused.
#[repr(C, align(64))]
#[derive(Clone)]
pub(crate) struct K1State {
    acc: [[f64; 8]; ACCS],
    a: [f64; 8],
    b: [f64; 8],
    c: [f64; 8],
    count: u64,
    /// The L1 zone in f64 words, minus 1 (a power of 2).
    l1_mask: usize,
}

/// One cache line.
#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct Line([f64; 8]);

const _: () = assert!(std::mem::size_of::<Line>() == 64);

type Pass = unsafe fn(&mut K1State, *mut f64, *mut f64);

/// One worker's K1. What its digest catches, and what it cannot:
/// - the upper 32 bits hash the accumulators, so a wrong product or a wrong loaded value
///   shows, unless it only flips low-order mantissa bits late in a block: the additions
///   that follow can round such a difference away, which is inherent to the scheme;
/// - the lower 32 bits hash the L1 zone, where only the `L1_LS` lines store: with the
///   AVX-512 groups (no L1 stores) they are the same for every block.
pub(crate) struct K1 {
    pass: Pass,
    lanes: usize,
    passes: u32,
    init: K1State,
    st: K1State,
    l1: Vec<Line>,
    l2: Vec<Line>,
    /// L2 words one pass moves on.
    l2_stride: usize,
    /// Passes before the L2 pointer goes back to the start (`getL2LoopCount`).
    l2_loops: u64,
    /// Test hook: (word of the L2 zone, bit) flipped once halfway through a block.
    #[cfg(test)]
    flip: Option<(usize, u32)>,
}

impl K1 {
    pub(crate) fn new(ctx: &WorkerCtx) -> Result<Self, KernelError> {
        let (pass, lanes, passes, l2_per_pass) = match ctx.isa {
            Isa::Avx512 if is_x86_feature_detected!("avx512f") => {
                (k1_block_avx512 as Pass, 8, PASSES, L2_PER_PASS_AVX512)
            }
            Isa::Avx2 if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") => {
                (k1_block_avx2 as Pass, 4, PASSES, L2_PER_PASS_AVX2)
            }
            Isa::Sse2 => (k1_block_sse2 as Pass, 2, PASSES_SSE2, L2_PER_PASS_SSE2),
            _ => return Err(KernelError::Unsupported),
        };
        // DA9: L1 zone = L1d / 2, L2 zone = l2_thread / 2.
        let l1_words = zone(ctx.budget.l1d / 2, 4 * KIB) / 8;
        let l2_bytes = zone(ctx.budget.l2_thread / 2, 64 * KIB);
        // getL2LoopCount: the passes that fit in 80% of the L2 zone.
        let l2_loops = ((0.8 * l2_bytes as f64 / 64.0 / l2_per_pass as f64) as u64).max(1);
        let l2_stride = l2_per_pass * 8;
        let l2_words = (l2_loops as usize)
            .checked_mul(l2_stride)
            .ok_or(KernelError::Insufficient)?
            .max(l2_bytes / 8);

        // The seed moves every value by less than 1e-3.
        let offset = (Xoshiro256ss::new(ctx.seed).next_u64() >> 11) as f64 / (1u64 << 63) as f64;
        let l1 = memory(l1_words + K1_PAD, offset)?;
        let l2 = memory(l2_words + K1_PAD, offset)?;

        // FIRESTARTER loads the registers from the start of its buffer: the operands from
        // word 0, the accumulator of register `r` (3..=12) from byte 256 + r * width.
        let mem = flat(&l1);
        let reg = |word: usize| -> [f64; 8] {
            std::array::from_fn(|lane| if lane < lanes { mem[word + lane] } else { 0.0 })
        };
        let init = K1State {
            acc: std::array::from_fn(|i| reg(32 + (i + 3) * lanes)),
            a: reg(0),
            b: reg(lanes),
            c: reg(2 * lanes),
            count: 0,
            l1_mask: l1_words - 1,
        };
        Ok(Self {
            pass,
            lanes,
            passes,
            st: init.clone(),
            init,
            l1,
            l2,
            l2_stride,
            l2_loops,
            #[cfg(test)]
            flip: None,
        })
    }

    /// FIRESTARTER's register hash (CRC32C of every 64-bit lane from `0xffffffff`) in the
    /// upper half; the digest of the L1 zone, where the `L1_LS` lines store, in the lower.
    fn digest(&self) -> u64 {
        let crc = self
            .st
            .acc
            .iter()
            .flat_map(|acc| &acc[..self.lanes])
            .fold(0xFFFF_FFFF, |c, v| crc::crc32c_u64(c, v.to_bits()));
        (u64::from(crc) << 32) | (verify::digest_f64(flat(&self.l1)) >> 32)
    }
}

impl Kernel for K1 {
    fn iterate(&mut self, beat: &AtomicU64) -> Check {
        // DA6: every block starts again from the same state.
        self.st.clone_from(&self.init);
        for pass in 0..self.passes {
            if pass % BEAT_PASSES == 0 {
                beat.fetch_add(1, Ordering::Relaxed);
            }
            #[cfg(test)]
            if pass == self.passes / 2 {
                if let Some((word, bit)) = self.flip.take() {
                    let w = &mut flat_mut(&mut self.l2)[word];
                    *w = f64::from_bits(w.to_bits() ^ (1 << bit));
                }
            }
            let l2_at = (self.st.count % self.l2_loops) as usize * self.l2_stride;
            // SAFETY: `new` picked `pass` for an instruction set this CPU has, and sized
            // `l1` to the masked zone plus K1_PAD and `l2` to `l2_loops` strides plus
            // K1_PAD, so `l2_at` plus one stride and K1_PAD stays inside. The fields are
            // private to this module and nothing changes them after `new` (the test hook
            // only flips a bit of the data).
            unsafe {
                (self.pass)(
                    &mut self.st,
                    self.l1.as_mut_ptr().cast(),
                    self.l2.as_mut_ptr().cast::<f64>().add(l2_at),
                );
            }
            self.st.count += 1;
        }
        Check::Digest(self.digest())
    }
}

/// The largest power of 2 not above `bytes`, and at least `min`.
fn zone(bytes: u64, min: u64) -> usize {
    (1u64 << bytes.max(min).ilog2()) as usize
}

/// `words` f64 of FIRESTARTER's `initMemory` (with the seed's `offset`), in whole lines.
fn memory(words: usize, offset: f64) -> Result<Vec<Line>, KernelError> {
    let mut lines = Vec::new();
    lines
        .try_reserve_exact(words.div_ceil(8))
        .map_err(|_| KernelError::Insufficient)?;
    lines.resize(words.div_ceil(8), Line([0.0; 8]));
    init_memory(flat_mut(&mut lines), offset);
    Ok(lines)
}

/// `initMemory`: the first block from the formula, copies of it, then the tail from the
/// formula with its own index.
fn init_memory(mem: &mut [f64], offset: f64) {
    let value = |i: usize| 0.25 + offset + i as f64 * 8.0 * INIT_STEP;
    for (i, m) in mem.iter_mut().enumerate().take(INIT_BLOCK) {
        *m = value(i);
    }
    let mut i = INIT_BLOCK;
    while i + INIT_BLOCK <= mem.len() {
        mem.copy_within(i - INIT_BLOCK..i, i);
        i += INIT_BLOCK;
    }
    for (j, m) in mem.iter_mut().enumerate().skip(i) {
        *m = value(j);
    }
}

fn flat(lines: &[Line]) -> &[f64] {
    // SAFETY: `Line` is `repr(C)` around `[f64; 8]` and 64 bytes (asserted above), so the
    // lines are contiguous f64 values.
    unsafe { std::slice::from_raw_parts(lines.as_ptr().cast(), lines.len() * 8) }
}

fn flat_mut(lines: &mut [Line]) -> &mut [f64] {
    // SAFETY: as in `flat`, and the borrow is exclusive.
    unsafe { std::slice::from_raw_parts_mut(lines.as_mut_ptr().cast(), lines.len() * 8) }
}

pub(crate) struct K1Factory;

impl KernelFactory for K1Factory {
    fn reference(&self, ctx: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        Some(
            K1::new(ctx)
                .map(|mut k| match k.iterate(&AtomicU64::new(0)) {
                    Check::Digest(d) => vec![d],
                    _ => unreachable!("K1 returns a digest"),
                })
                .map_err(RefFailure::from),
        )
    }

    fn worker(&self, ctx: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        Ok(Box::new(K1::new(ctx)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{PhaseShared, ThreadBudget};
    use oma_ipc::load::{DataSize, KernelId};
    use std::sync::Arc;

    const KIB: u64 = 1024;
    /// Passes per block in most tests: a debug build runs the full block too slowly.
    const TEST_PASSES: u32 = 256;

    fn ctx(isa: Isa, seed: u64, worker: u32) -> WorkerCtx {
        WorkerCtx {
            isa,
            size: DataSize::Auto,
            budget: ThreadBudget {
                l1d: 32 * KIB,
                l2_thread: 256 * KIB,
                l3_share: 1024 * KIB,
                ram_per_thread: 1024 * KIB,
            },
            seed,
            worker,
            workers: 4,
            patterns: Vec::new(),
            shared: Arc::new(PhaseShared::default()),
        }
    }

    /// The instruction sets this machine has, saying which are skipped.
    fn available() -> Vec<Isa> {
        [Isa::Avx512, Isa::Avx2, Isa::Sse2]
            .into_iter()
            .filter(|&isa| match K1::new(&ctx(isa, 1, 0)) {
                Err(KernelError::Unsupported) => {
                    eprintln!("skipped: {isa:?} not available");
                    false
                }
                _ => true,
            })
            .collect()
    }

    fn kernel(isa: Isa, seed: u64, worker: u32) -> K1 {
        let mut k = K1::new(&ctx(isa, seed, worker)).expect("supported");
        k.passes = TEST_PASSES;
        k
    }

    fn digest(k: &mut K1) -> u64 {
        match k.iterate(&AtomicU64::new(0)) {
            Check::Digest(d) => d,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn groups_parse_and_fill_up_to_1536_lines() {
        assert_eq!(
            groups::parse_groups(groups::AVX512_GROUPS).unwrap(),
            [("REG", 140), ("L1_L", 40), ("L2_L", 70), ("L2_S", 4)]
        );
        assert_eq!(
            groups::parse_groups(groups::AVX2_GROUPS).unwrap(),
            [("REG", 40), ("L1_LS", 90), ("L2_LS", 9)]
        );
        // FIRESTARTER spreads the later groups among the first.
        assert_eq!(
            groups::sequence(&[("REG", 2), ("L1_L", 2)]),
            ["REG", "L1_L", "REG", "L1_L"]
        );
        for (text, len, l2) in [
            (groups::AVX512_GROUPS, 254, L2_PER_PASS_AVX512),
            (groups::AVX2_GROUPS, 139, L2_PER_PASS_AVX2),
        ] {
            let lines = groups::unroll(text).unwrap();
            assert_eq!(lines.len(), groups::LINES / len * len);
            assert!(lines.len() <= groups::LINES && lines.len() + len > groups::LINES);
            let count = |item: &str| lines.iter().filter(|&&l| l == item).count();
            for (item, val) in groups::parse_groups(text).unwrap() {
                assert_eq!(count(item), val * (groups::LINES / len), "{item}");
            }
            assert_eq!(count("L2_L") + count("L2_S") + count("L2_LS"), l2);
        }
        assert_eq!(L2_PER_PASS_SSE2, L2_PER_PASS_AVX2);
    }

    #[test]
    fn zero_value_group_is_rejected() {
        for bad in ["REG:0,L1_L:4", "REG:4,L2_S:0", "REG:x", "REG", "L3_L:2", ""] {
            assert!(groups::parse_groups(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn k1_digest_is_deterministic_per_isa() {
        assert!(crate::kernel::factory(KernelId::K1).is_some());
        let mut seen = Vec::new();
        for isa in available() {
            let d = digest(&mut kernel(isa, 7, 0));
            assert_eq!(
                d,
                digest(&mut kernel(isa, 7, 3)),
                "{isa:?}: same seed, any worker"
            );
            assert_ne!(d, digest(&mut kernel(isa, 8, 0)), "{isa:?}: another seed");
            assert!(!seen.contains(&d), "{isa:?}: each set has its own digest");
            seen.push(d);
        }
    }

    #[test]
    fn k1_blocks_repeat_the_same_hash() {
        for isa in available() {
            let mut k = kernel(isa, 7, 0);
            let first = digest(&mut k);
            assert_eq!(digest(&mut k), first, "{isa:?}");
        }
    }

    #[test]
    fn k1_accumulators_stay_finite() {
        for isa in available() {
            // A full block: the accumulators must not reach infinity on the way.
            let mut k = K1::new(&ctx(isa, 7, 0)).unwrap();
            k.iterate(&AtomicU64::new(0));
            let lanes = k.lanes;
            for acc in &k.st.acc {
                for &v in &acc[..lanes] {
                    assert!(v.is_finite() && v != 0.0, "{isa:?}: {v}");
                }
            }
        }
    }

    #[test]
    fn k1_bit_flip_changes_the_digest() {
        for isa in available() {
            // The word the first L2 load reads: L2_L and L2_LS load one line ahead.
            let text = match isa {
                Isa::Avx512 => groups::AVX512_GROUPS,
                _ => groups::AVX2_GROUPS,
            };
            let l2_items: Vec<&str> = groups::unroll(text)
                .unwrap()
                .into_iter()
                .filter(|l| l.starts_with("L2"))
                .collect();
            let first_load = l2_items.iter().position(|l| *l != "L2_S").unwrap();
            let expected = digest(&mut kernel(isa, 7, 0));
            let mut k = kernel(isa, 7, 0);
            k.flip = Some((first_load * 8 + 8, 40));
            let flipped = digest(&mut k);
            // The upper half hashes the accumulators: they took the wrong value in.
            assert_ne!(flipped >> 32, expected >> 32, "{isa:?}");
        }
    }
}
