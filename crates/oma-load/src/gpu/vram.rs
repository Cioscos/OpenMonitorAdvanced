// Adapted from memtest_vulkan (https://github.com/GpuZelenograd/memtest_vulkan),
// commit fd9ff59cde85cf11e25263e3f32d7adae0ba5e3b: address-derived rotated pattern, write once and re-read in
// rotated order, bit-error statistics.
// Original work: Copyright (c) 2022 galkinvv by GpuZelenograd, licensed under the zlib
// License (see THIRD_PARTY_LICENSES.txt).
// Modified for OpenMonitor Advanced: ported to HLSL cs_5_0 and D3D11, sized
// from the DXGI video memory budget, classic passes added.
// The modifications are part of OpenMonitor Advanced, GPL-3.0-or-later.

//! S4, the VRAM check (plan DG6). The VRAM target is allocated in chunks; each round
//! writes a pattern over all of them and reads it back. Every submission covers one window
//! of one chunk, sized to the submission target (DG4).

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Buffer, ID3D11ComputeShader, ID3D11DeviceContext, ID3D11UnorderedAccessView,
};

use super::device::{GpuDevice, GpuError};
use super::engine::{GpuCheck, GpuMismatch, GpuWorkload, PhaseCtx};
use super::shaders::S4_VRAM;
use super::sizing::{chunk_bytes, vram_target};
use super::submit::{calibrate, Submit};

const MIB: u64 = 1 << 20;
/// Below this the phase is skipped with `"vram"` (DG6).
pub const MIN_VRAM: u64 = 256 * MIB;
/// Elements (`uint4`) of one rotation group of the re-read, memtest_vulkan's
/// `TEST_WINDOW_READ_ADDR_ROTATION_GRANULARITY`; chunks and windows are multiples of it.
pub const ROTATION: u64 = 0x2000;
/// Bytes of one rotation group: chunks are rounded down to it.
const GRAIN: u64 = ROTATION * 16;
/// The smallest chunk a refused allocation is halved down to.
const MIN_CHUNK: u64 = 64 * MIB;

/// The address pattern: word `index` of the whole allocation in the round with `key`.
/// memtest_vulkan's `test_value_by_index`, with the high half of a 64-bit index folded in
/// like its per-window offset (`0x81`). Bit for bit the `vram_word` of `s4_vram.hlsl`.
pub fn vram_word(index: u64, key: u32) -> u32 {
    let a = (index as u32)
        .wrapping_add(((index >> 32) as u32).wrapping_mul(0x81))
        .wrapping_add(key)
        .wrapping_add(1);
    a.rotate_left(a % 31)
}

/// The key of address round `round` (from 1), memtest_vulkan's `calc_param`.
pub fn round_key(round: u32) -> u32 {
    round.wrapping_mul(0x0010_0107)
}

/// The classic pass of a round: every 4th round, in turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classic {
    WalkingOnes,
    /// Moving inversions with this pattern and its complement.
    MovingInversions(u32),
    Modulo20,
}

/// `None` for an address round.
pub fn classic_pass(round: u32) -> Option<Classic> {
    if round == 0 || !round.is_multiple_of(4) {
        return None;
    }
    let turn = round / 4 - 1;
    Some(match turn % 3 {
        0 => Classic::WalkingOnes,
        1 if (turn / 3).is_multiple_of(2) => Classic::MovingInversions(0),
        1 => Classic::MovingInversions(0x5555_5555),
        _ => Classic::Modulo20,
    })
}

/// What the words should hold: shader `pattern` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Pattern {
    /// [`vram_word`] with `key`.
    Address = 0,
    /// One bit set, `(index + key) % 32`.
    WalkingOnes = 1,
    /// `key` everywhere.
    Solid = 2,
    /// `key` where `index % 20 == aux`, `!key` elsewhere.
    Modulo20 = 3,
}

/// Shader `op` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Op {
    /// Writes the pattern, in reverse order within the window.
    Write = 0,
    /// Re-reads in rotated order and counts the wrong words.
    Verify = 1,
    /// [`Op::Verify`], then writes the complement of the pattern.
    VerifyInvert = 2,
    /// Test hook and fault injection: thread 0 XORs word `key` of the chunk with `aux`.
    Flip = 3,
}

/// One pass over the whole allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub pattern: Pattern,
    pub op: Op,
    pub key: u32,
    pub aux: u32,
}

/// The passes of round `round` (from 1).
pub fn round_steps(round: u32) -> Vec<Step> {
    let step = |pattern, op, key, aux| Step {
        pattern,
        op,
        key,
        aux,
    };
    let turn = round / 4;
    match classic_pass(round) {
        None => {
            let key = round_key(round);
            vec![
                step(Pattern::Address, Op::Write, key, 0),
                step(Pattern::Address, Op::Verify, key, 0),
            ]
        }
        Some(Classic::WalkingOnes) => vec![
            step(Pattern::WalkingOnes, Op::Write, turn, 0),
            step(Pattern::WalkingOnes, Op::Verify, turn, 0),
        ],
        Some(Classic::MovingInversions(p)) => vec![
            step(Pattern::Solid, Op::Write, p, 0),
            step(Pattern::Solid, Op::VerifyInvert, p, 0),
            step(Pattern::Solid, Op::Verify, !p, 0),
        ],
        Some(Classic::Modulo20) => {
            let (key, offset) = (round_key(round), turn % 20);
            vec![
                step(Pattern::Modulo20, Op::Write, key, offset),
                step(Pattern::Modulo20, Op::Verify, key, offset),
            ]
        }
    }
}

/// The chunks the VRAM check got and the notices for the app.
#[derive(Debug)]
pub struct Allocation<A> {
    /// Each chunk and its size in bytes, a multiple of 128 KiB.
    pub chunks: Vec<(A, u64)>,
    pub notices: Vec<(String, u64)>,
}

/// Allocates `target` bytes in chunks of at most `chunk` (DG6). A failed allocation keeps
/// what is there, after halving the chunk down to 64 MiB: `vram_reduced` when short of the
/// target, `OutOfMemory` (the phase is skipped) under `min`. A lost device ends it; `stop` is looked at before each chunk.
pub fn allocate<A>(
    target: u64,
    chunk: u64,
    min: u64,
    alloc: &mut dyn FnMut(u64) -> Result<A, GpuError>,
    stop: &AtomicBool,
) -> Result<Allocation<A>, GpuError> {
    let target = target / GRAIN * GRAIN;
    let mut chunk = chunk / GRAIN * GRAIN;
    let mut chunks = Vec::new();
    let mut allocated = 0;
    if target >= min {
        while allocated < target {
            if stop.load(Ordering::Relaxed) {
                return Err(GpuError::Stopped);
            }
            let bytes = chunk.min(target - allocated);
            match alloc(bytes) {
                Ok(a) => chunks.push((a, bytes)),
                Err(e @ (GpuError::Lost(_) | GpuError::Hung | GpuError::Stopped)) => return Err(e),
                // A smaller resource may still fit (an integrated GPU guarantees less).
                Err(e) if bytes / 2 >= MIN_CHUNK => {
                    tracing::info!(error = ?e, bytes, "VRAM chunk refused, trying half");
                    chunk = bytes / 2 / GRAIN * GRAIN;
                    continue;
                }
                Err(e) => {
                    tracing::warn!(error = ?e, allocated, target, "VRAM allocation failed");
                    break;
                }
            }
            allocated += bytes;
        }
    }
    if allocated < min || allocated == 0 {
        return Err(GpuError::OutOfMemory);
    }
    let mut notices = vec![("vram_allocated".to_owned(), allocated)];
    if allocated < target {
        notices.push(("vram_reduced".to_owned(), allocated));
    }
    Ok(Allocation { chunks, notices })
}

/// Thread groups along x of one dispatch, as in the shader (`GROUPS_X`).
const GROUPS_X: u64 = 16_384;

struct Chunk {
    _buffer: ID3D11Buffer,
    uav: ID3D11UnorderedAccessView,
    /// Elements (`uint4`), a multiple of [`ROTATION`].
    elements: u64,
    /// Word index of the chunk's first word in the whole allocation.
    base: u64,
}

pub struct VramWorkload {
    gpu: GpuDevice,
    shader: ID3D11ComputeShader,
    constants: ID3D11Buffer,
    stats: ID3D11Buffer,
    stats_uav: ID3D11UnorderedAccessView,
    target: u64,
    chunk: u64,
    min: u64,
    chunks: Vec<Chunk>,
    /// Elements per submission, a multiple of [`ROTATION`].
    window: u64,
    /// The round in progress (from 1), its passes and where the next submission starts.
    round: u32,
    steps: Vec<Step>,
    step: usize,
    chunk_at: usize,
    start: u64,
    /// The last round that gave a `GpuMismatch`: one per round.
    reported: u32,
    /// Word whose bit 0 is flipped after the writes of round 1 (fault injection, tests).
    flip: Option<u64>,
    /// Word index of the last first wrong word read, for the log and the tests.
    last_index: Option<u64>,
    pending: GpuCheck,
}

impl VramWorkload {
    /// The VRAM check sized from the phase's budget (DG6).
    pub fn new(gpu: &GpuDevice, ctx: &PhaseCtx) -> Result<Self, GpuError> {
        let target = vram_target(ctx.budget.budget, ctx.integrated, ctx.budget.available_ram);
        let chunk = chunk_bytes(gpu.dedicated_bytes());
        let mut load = Self::with_size(gpu, target, chunk, MIN_VRAM)?;
        // Fault injection (DA18): bit 0 of the first word, after the writes of round 1.
        load.flip = ctx.inject.is_some().then_some(0);
        Ok(load)
    }

    /// `target` bytes in chunks of `chunk`, skipped under `min`.
    fn with_size(gpu: &GpuDevice, target: u64, chunk: u64, min: u64) -> Result<Self, GpuError> {
        let (stats, stats_uav, _) = gpu.structured_buffer(32, 4)?;
        Ok(VramWorkload {
            shader: gpu.compute_shader(S4_VRAM)?,
            constants: gpu.constant_buffer_of(32)?,
            stats,
            stats_uav,
            target,
            chunk,
            min,
            chunks: Vec::new(),
            window: ROTATION,
            round: 1,
            steps: round_steps(1),
            step: 0,
            chunk_at: 0,
            start: 0,
            reported: 0,
            flip: None,
            last_index: None,
            pending: GpuCheck::default(),
            gpu: gpu.clone(),
        })
    }

    /// One dispatch of `step` over `count` elements of chunk `c` from element `start`.
    /// `ctx` must be `self.gpu.context()`: the constants are written through the device's
    /// context and must land in order with the dispatch recorded on `ctx`.
    fn dispatch(&self, ctx: &ID3D11DeviceContext, step: Step, c: usize, start: u64, count: u64) {
        debug_assert_eq!(
            ctx.as_raw(),
            self.gpu.context().as_raw(),
            "the submission queue must wrap the load's own device context"
        );
        let chunk = &self.chunks[c];
        self.gpu.write_words(
            &self.constants,
            &[
                chunk.base as u32,
                (chunk.base >> 32) as u32,
                start as u32,
                count as u32,
                step.key,
                step.aux,
                step.pattern as u32 | (step.op as u32) << 8,
                self.round,
            ],
        );
        let groups = count.div_ceil(256);
        let (x, y) = (groups.min(GROUPS_X), groups.div_ceil(GROUPS_X));
        let uavs = [Some(chunk.uav.clone()), Some(self.stats_uav.clone())];
        // SAFETY: `ctx` is this device's immediate context (asserted above), used only on
        // this thread; resources of this device; the shader returns for threads past
        // `count`, and `start + count` is inside the chunk; `uavs` outlives the call that
        // reads its 2 entries; the UAVs are unbound afterwards.
        unsafe {
            ctx.CSSetShader(&self.shader, None);
            ctx.CSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            ctx.CSSetUnorderedAccessViews(0, 2, Some(uavs.as_ptr()), None);
            ctx.Dispatch(x as u32, y as u32, 1);
            ctx.CSSetUnorderedAccessViews(0, 2, Some([None, None].as_ptr()), None);
        }
    }

    /// Bit 0 of word `word` of the whole allocation.
    fn flip_word(&self, sub: &mut dyn Submit, word: u64) -> Result<(), GpuError> {
        let c = self
            .chunks
            .iter()
            .rposition(|c| c.base <= word)
            .expect("chunk 0 starts at word 0");
        let step = Step {
            pattern: Pattern::Address,
            op: Op::Flip,
            key: (word - self.chunks[c].base) as u32,
            aux: 1,
        };
        tracing::warn!(word, "fault injected into the VRAM");
        sub.submit(&mut |ctx| self.dispatch(ctx, step, c, 0, 1))
    }

    /// Reads and resets the statistics; a round with wrong words gives one `GpuMismatch`
    /// (the first read of the round) and, at each read, the `vram_bits` / `vram_words`
    /// notices (ruling 2).
    fn flush(&mut self, sub: &mut dyn Submit) -> Result<(), GpuError> {
        sub.finish()?;
        let s: Vec<u32> = self
            .gpu
            .read_buffer(&self.stats, 32)?
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        if s[0] == 0 {
            return Ok(());
        }
        self.gpu.write_words(&self.stats, &[0; 8]);
        let index = u64::from(s[1]) | u64::from(s[2]) << 32;
        tracing::error!(
            round = self.round,
            words = s[0],
            index,
            bits = format_args!("{:#010x}", s[5]),
            "wrong words in the VRAM"
        );
        self.last_index = Some(index);
        let pending = &mut self.pending;
        pending.notices.push(("vram_bits".to_owned(), s[5].into()));
        pending.notices.push(("vram_words".to_owned(), s[0].into()));
        if self.reported != self.round {
            self.reported = self.round;
            pending.mismatches.push(GpuMismatch {
                iteration: self.round.into(),
                expected: s[3].into(),
                actual: s[4].into(),
            });
        }
        Ok(())
    }
}

impl GpuWorkload for VramWorkload {
    fn prepare(
        &mut self,
        sub: &mut dyn Submit,
        target_ms: f64,
        stop: &AtomicBool,
    ) -> Result<(), GpuError> {
        let gpu = self.gpu.clone();
        // Chunks are at most 512 MiB (`chunk_bytes`), so the size fits a `u32`.
        let allocation = allocate(
            self.target,
            self.chunk,
            self.min,
            &mut |bytes| gpu.raw_buffer(bytes as u32),
            stop,
        )?;
        let mut base = 0;
        for ((buffer, uav), bytes) in allocation.chunks {
            self.chunks.push(Chunk {
                _buffer: buffer,
                uav,
                elements: bytes / 16,
                base,
            });
            base += bytes / 4;
        }
        // Sent with the first check of the phase.
        self.pending.notices = allocation.notices;
        if stop.load(Ordering::Relaxed) {
            return Err(GpuError::Stopped);
        }
        // The window, in rotation groups, on the writes of round 1 over chunk 0.
        let groups = self.chunks[0].elements / ROTATION;
        let step = self.steps[0];
        let param = calibrate(sub, target_ms, &mut |ctx, param| {
            self.dispatch(ctx, step, 0, 0, u64::from(param).min(groups) * ROTATION)
        })?;
        self.window = u64::from(param).clamp(1, groups) * ROTATION;
        tracing::info!(
            chunks = self.chunks.len(),
            window_bytes = self.window * 16,
            target_ms,
            "VRAM check calibrated"
        );
        self.gpu.write_words(&self.stats, &[0; 8]);
        sub.finish()
    }

    fn submit(&mut self, sub: &mut dyn Submit) -> Result<(), GpuError> {
        let step = self.steps[self.step];
        let (c, start) = (self.chunk_at, self.start);
        let count = self.window.min(self.chunks[c].elements - start);
        sub.submit(&mut |ctx| self.dispatch(ctx, step, c, start, count))?;
        if step.op != Op::Write {
            self.pending.checks += 1;
        }
        self.start += count;
        if self.start < self.chunks[c].elements {
            return Ok(());
        }
        self.start = 0;
        self.chunk_at += 1;
        if self.chunk_at < self.chunks.len() {
            return Ok(());
        }
        // The pass is over the whole allocation.
        self.chunk_at = 0;
        if step.op == Op::Write {
            if let Some(word) = self.flip.take() {
                self.flip_word(sub, word)?;
            }
        }
        self.step += 1;
        if self.step == self.steps.len() {
            self.flush(sub)?;
            self.round += 1;
            self.steps = round_steps(self.round);
            self.step = 0;
        }
        Ok(())
    }

    fn check(&mut self, sub: &mut dyn Submit) -> Result<GpuCheck, GpuError> {
        // The round in progress too, so a phase that ends mid-round loses nothing.
        self.flush(sub)?;
        Ok(std::mem::take(&mut self.pending))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    #[test]
    fn vram_word_is_deterministic_and_key_dependent() {
        assert_eq!(vram_word(12_345, 7), vram_word(12_345, 7));
        assert_ne!(vram_word(12_345, 7), vram_word(12_345, 8));
        assert_ne!(
            vram_word(12_345, round_key(1)),
            vram_word(12_345, round_key(2))
        );
        // memtest_vulkan's first word of a window: index 0, `calc_param` 0, a = 1, shift 1.
        assert_eq!(vram_word(0, 0), 2);
        // a = 31 rotates by 0.
        assert_eq!(vram_word(30, 0), 31);
        // The high half of the index changes the word.
        assert_ne!(vram_word(5, 0), vram_word(5 + (1 << 32), 0));
    }

    #[test]
    fn vram_word_differs_between_neighbours() {
        for key in [0, round_key(1), round_key(77)] {
            for i in 0..1_000_000u64 {
                assert_ne!(vram_word(i, key), vram_word(i + 1, key), "{i} {key}");
            }
        }
    }

    #[test]
    fn classic_pass_order_rotates() {
        use Classic::*;
        let order: Vec<_> = (1..=28).filter_map(classic_pass).collect();
        assert_eq!(
            order,
            [
                WalkingOnes,
                MovingInversions(0),
                Modulo20,
                WalkingOnes,
                MovingInversions(0x5555_5555),
                Modulo20,
                WalkingOnes,
            ]
        );
        assert_eq!(classic_pass(3), None);
        assert_eq!(classic_pass(4), Some(WalkingOnes));
        // Address rounds write and verify; moving inversions verify twice.
        let ops = |round| round_steps(round).iter().map(|s| s.op).collect::<Vec<_>>();
        assert_eq!(ops(1), [Op::Write, Op::Verify]);
        assert_eq!(ops(8), [Op::Write, Op::VerifyInvert, Op::Verify]);
        assert_eq!(round_steps(8)[2].key, u32::MAX);
        assert_eq!(round_steps(1)[0].pattern, Pattern::Address);
        assert_eq!(round_steps(1)[0].key, round_key(1));
        assert_eq!(round_steps(12)[0].pattern, Pattern::Modulo20);
    }

    fn fake(ok: usize) -> impl FnMut(u64) -> Result<u64, GpuError> {
        let mut calls = 0;
        move |bytes| {
            calls += 1;
            if calls > ok {
                Err(GpuError::OutOfMemory)
            } else {
                Ok(bytes)
            }
        }
    }

    #[test]
    fn allocation_shortfall_reports_vram_reduced() {
        let go = AtomicBool::new(false);
        let full = allocate(1100 * MIB, 512 * MIB, MIN_VRAM, &mut fake(9), &go).unwrap();
        let sizes: Vec<u64> = full.chunks.iter().map(|c| c.1).collect();
        assert_eq!(sizes, [512 * MIB, 512 * MIB, 76 * MIB]);
        assert_eq!(full.notices, [("vram_allocated".to_owned(), 1100 * MIB)]);

        let short = allocate(1100 * MIB, 512 * MIB, MIN_VRAM, &mut fake(1), &go).unwrap();
        assert_eq!(short.chunks.len(), 1);
        assert_eq!(
            short.notices,
            [
                ("vram_allocated".to_owned(), 512 * MIB),
                ("vram_reduced".to_owned(), 512 * MIB),
            ]
        );
        // A target that is not a multiple of 128 KiB is rounded, not reported short.
        let odd = allocate(300 * MIB + 5, GIB, MIN_VRAM, &mut fake(9), &go).unwrap();
        assert_eq!(odd.notices, [("vram_allocated".to_owned(), 300 * MIB)]);
        // A lost device is not a shortfall.
        let mut lost = |_| -> Result<u64, GpuError> { Err(GpuError::Lost(5)) };
        assert_eq!(
            allocate(GIB, MIN_VRAM, MIN_VRAM, &mut lost, &go).unwrap_err(),
            GpuError::Lost(5)
        );
        let stop = AtomicBool::new(true);
        assert_eq!(
            allocate(GIB, MIN_VRAM, MIN_VRAM, &mut fake(9), &stop).unwrap_err(),
            GpuError::Stopped
        );
    }

    #[test]
    fn failed_chunks_are_retried_at_half_size() {
        let go = AtomicBool::new(false);
        // Nothing over 128 MiB fits: 512 and 256 fail, then 128 MiB chunks to the end.
        let mut tried = Vec::new();
        let mut small = |bytes| {
            tried.push(bytes);
            if bytes > 128 * MIB {
                Err(GpuError::Create(0x8007_0057_u32 as i32))
            } else {
                Ok(bytes)
            }
        };
        let got = allocate(600 * MIB, 512 * MIB, MIN_VRAM, &mut small, &go).unwrap();
        let sizes: Vec<u64> = got.chunks.iter().map(|c| c.1).collect();
        assert_eq!(
            sizes,
            [128 * MIB, 128 * MIB, 128 * MIB, 128 * MIB, 88 * MIB]
        );
        assert_eq!(got.notices, [("vram_allocated".to_owned(), 600 * MIB)]);
        assert_eq!(tried[..3], [512 * MIB, 256 * MIB, 128 * MIB]);
        // The halving stops at 64 MiB, then it is a shortfall.
        let mut tried = Vec::new();
        let mut none = |bytes| -> Result<u64, GpuError> {
            tried.push(bytes);
            Err(GpuError::OutOfMemory)
        };
        assert_eq!(
            allocate(GIB, 512 * MIB, MIN_VRAM, &mut none, &go).unwrap_err(),
            GpuError::OutOfMemory
        );
        assert_eq!(tried, [512 * MIB, 256 * MIB, 128 * MIB, 64 * MIB]);
    }

    #[test]
    fn under_256_mib_skips_the_phase() {
        let go = AtomicBool::new(false);
        let mut never = |_| -> Result<u64, GpuError> { panic!("nothing to allocate") };
        assert_eq!(
            allocate(200 * MIB, MIN_VRAM, MIN_VRAM, &mut never, &go).unwrap_err(),
            GpuError::OutOfMemory
        );
        // 128 MiB allocated out of 1 GiB.
        assert_eq!(
            allocate(GIB, 128 * MIB, MIN_VRAM, &mut fake(1), &go).unwrap_err(),
            GpuError::OutOfMemory
        );
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn every_adapter_allocates_one_chunk() {
        // Allocation only, no dispatch; the buffer is released at once.
        for adapter in oma_win::gpu::stress_adapters() {
            let gpu = GpuDevice::open(adapter.luid).unwrap();
            let chunk = chunk_bytes(gpu.dedicated_bytes());
            let got = allocate(
                chunk,
                chunk,
                MIN_CHUNK,
                &mut |bytes| gpu.raw_buffer(bytes as u32),
                &AtomicBool::new(false),
            )
            .unwrap();
            let first = got.chunks[0].1;
            println!(
                "{}: dedicated {} MiB, chunk {} MiB, first allocation {} MiB",
                adapter.name,
                gpu.dedicated_bytes() / MIB,
                chunk / MIB,
                first / MIB
            );
            assert!(first >= MIN_CHUNK);
        }
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn vram_detects_a_flipped_word() {
        use crate::gpu::submit::Submitter;
        use std::time::Instant;
        const FLIP: u64 = 0x0012_3457;
        let t0 = Instant::now();
        let adapter = oma_win::gpu::stress_adapters()
            .into_iter()
            .next()
            .expect("no hardware GPU");
        let gpu = GpuDevice::open(adapter.luid).unwrap();
        let mut sub = Submitter::new(&gpu).unwrap();
        // One 64 MiB chunk: never the real target in a test.
        let mut load = VramWorkload::with_size(&gpu, 64 * MIB, 64 * MIB, 0).unwrap();
        load.flip = Some(FLIP);
        load.prepare(&mut sub, 5.0, &AtomicBool::new(false))
            .unwrap();
        while load.round == 1 {
            load.submit(&mut sub).unwrap();
        }
        let check = load.check(&mut sub).unwrap();
        let expected = vram_word(FLIP, round_key(1));
        assert_eq!(
            check.mismatches,
            [GpuMismatch {
                iteration: 1,
                expected: expected.into(),
                actual: (expected ^ 1).into(),
            }]
        );
        assert_eq!(load.last_index, Some(FLIP));
        assert!(check.checks >= 1);
        for (code, value) in [
            ("vram_allocated", 64 * MIB),
            ("vram_bits", 1),
            ("vram_words", 1),
        ] {
            assert!(
                check.notices.contains(&(code.to_owned(), value)),
                "{:?}",
                check.notices
            );
        }
        // Every classic pass, with both moving-inversion patterns, finds nothing.
        while load.round <= 24 {
            load.submit(&mut sub).unwrap();
        }
        let clean = load.check(&mut sub).unwrap();
        assert_eq!(clean.mismatches, [], "{:?}", clean.notices);
        assert!(t0.elapsed().as_secs_f64() < 2.0, "{:?}", t0.elapsed());
    }
}
