//! S3, the memory stream (plan DH5): a fixed set of 1 GiB of VRAM, half sources and half
//! destinations, in pieces of at most `chunk_bytes`. The sources are filled once from the seed;
//! each submission copies whole pieces in turn, so in a phase every byte of the set is
//! read and written. No data check (§5.3): the stress test looks at the bandwidth.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Buffer, ID3D11ComputeShader, ID3D11DeviceContext, ID3D11UnorderedAccessView,
};

use super::device::{GpuDevice, GpuError};
use super::engine::{GpuCheck, GpuWorkload, PhaseCtx};
use super::shaders::S3_STREAM;
use super::sizing::{chunk_bytes, vram_target};
use super::submit::{calibrate, Submit};

const MIB: u64 = 1 << 20;
/// The set of the stream, sources and destinations together.
pub const STREAM_BYTES: u64 = 1 << 30;
/// Below this the phase is skipped with `"vram"`.
const MIN_SET: u64 = 256 * MIB;
/// Bytes one copied `float4` moves (DH2): 16 read and 16 written, as STREAM's "copy".
const BYTES_PER_ELEMENT: f64 = 32.0;
/// Thread groups along x of one dispatch, as in the shader (`GROUPS_X`).
const GROUPS_X: u64 = 16_384;
/// `calibrate` doubles from 64: its parameter counts 1/64 of a copy, so the first try is
/// one piece (64 pieces of 512 MiB would take seconds on an integrated GPU).
const CALIBRATION_STEPS: u32 = 64;

/// The size of the set and its count of pieces: `STREAM_BYTES` capped by the VRAM `target`,
/// in the fewest even pieces of at most `chunk`, each rounded down to 4 KiB, so the set is
/// `pieces` times the piece. `None` under 256 MiB.
pub fn stream_set(target: u64, chunk: u64) -> Option<(u64, u32)> {
    let bytes = STREAM_BYTES.min(target);
    let pieces = bytes.div_ceil(chunk).next_multiple_of(2).max(2);
    let set = ((bytes / pieces) & !4095) * pieces;
    (set >= MIN_SET).then_some((set, pieces as u32))
}

/// `vram_allocated`, and `vram_reduced` under [`STREAM_BYTES`].
fn stream_notices(set: u64) -> Vec<(String, u64)> {
    let mut notices = vec![("vram_allocated".to_owned(), set)];
    if set < STREAM_BYTES {
        notices.push(("vram_reduced".to_owned(), set));
    }
    notices
}

/// Copies done by a submission with calibration parameter `param`.
fn copies(param: u32) -> u32 {
    (param / CALIBRATION_STEPS).max(1)
}

pub struct StreamLoad {
    gpu: GpuDevice,
    shader: ID3D11ComputeShader,
    constants: ID3D11Buffer,
    seed: u32,
    set: u64,
    /// Pieces to allocate in `prepare`.
    wanted: u32,
    /// The sources, then their destinations: pair `i` is pieces `i` and `i + pairs`.
    pieces: Vec<(ID3D11Buffer, ID3D11UnorderedAccessView)>,
    /// Elements (`float4`) of a piece.
    count: u32,
    piece_bytes: u64,
    /// Pieces copied per submission (the calibrated parameter, DG4).
    copies: u32,
    /// The next pair to copy.
    next: usize,
    submitted: u64,
    checked: u64,
    notices: Vec<(String, u64)>,
}

impl StreamLoad {
    /// The stream sized from the phase's budget (DH5); `OutOfMemory` (skipped with
    /// `"vram"`) under 256 MiB.
    pub fn new(gpu: &GpuDevice, ctx: &PhaseCtx) -> Result<Self, GpuError> {
        let target = vram_target(ctx.budget.budget, ctx.integrated, ctx.budget.available_ram);
        let (set, pieces) =
            stream_set(target, chunk_bytes(gpu.dedicated_bytes())).ok_or(GpuError::OutOfMemory)?;
        Self::with_set(gpu, set, pieces, (ctx.seed ^ (ctx.seed >> 32)) as u32)
    }

    /// A set of `set` bytes in `pieces` pieces of `set / pieces` bytes, as [`stream_set`]
    /// gives them (an even count, each at most 512 MiB and a multiple of 4 KiB).
    fn with_set(gpu: &GpuDevice, set: u64, pieces: u32, seed: u32) -> Result<Self, GpuError> {
        let piece_bytes = set / u64::from(pieces);
        Ok(StreamLoad {
            shader: gpu.compute_shader(S3_STREAM)?,
            constants: gpu.constant_buffer()?,
            seed,
            set,
            wanted: pieces,
            pieces: Vec::new(),
            count: (piece_bytes / 16) as u32,
            piece_bytes,
            copies: 1,
            next: 0,
            submitted: 0,
            checked: 0,
            notices: stream_notices(set),
            gpu: gpu.clone(),
        })
    }

    fn pairs(&self) -> usize {
        self.pieces.len() / 2
    }

    /// One dispatch over pair `pair`: the copy, or with `fill` the seeded pattern into its
    /// source. `ctx` must be `self.gpu.context()`: the constants are written through the
    /// device's context and must land in order with the dispatch recorded on `ctx`.
    fn dispatch(&self, ctx: &ID3D11DeviceContext, pair: usize, fill: bool) {
        debug_assert_eq!(
            ctx.as_raw(),
            self.gpu.context().as_raw(),
            "the submission queue must wrap the load's own device context"
        );
        let (src, dst) = (&self.pieces[pair].1, &self.pieces[pair + self.pairs()].1);
        let uavs = if fill {
            [None, Some(src.clone())]
        } else {
            [Some(src.clone()), Some(dst.clone())]
        };
        self.gpu
            .set_constants(&self.constants, [self.count, u32::from(fill), self.seed, 0]);
        let groups = u64::from(self.count).div_ceil(256);
        let (x, y) = (groups.min(GROUPS_X), groups.div_ceil(GROUPS_X));
        // SAFETY: `ctx` is this device's immediate context (asserted above), used only on
        // this thread; resources of this device. Every piece's UAV spans `piece_bytes`, and
        // `count` = `piece_bytes / 16` comes from the same value (`with_set`), so the shader,
        // which returns for threads past `count`, stays inside both pieces. `uavs` outlives
        // the call that reads its 2 entries. Only the UAVs are unbound afterwards, so the
        // pieces can be bound elsewhere; shader and constant buffer are set by every load.
        unsafe {
            ctx.CSSetShader(&self.shader, None);
            ctx.CSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            ctx.CSSetUnorderedAccessViews(0, 2, Some(uavs.as_ptr()), None);
            ctx.Dispatch(x as u32, y as u32, 1);
            ctx.CSSetUnorderedAccessViews(0, 2, Some([None, None].as_ptr()), None);
        }
    }

    /// `n` copies from pair `from`, in turn.
    fn copy(&self, ctx: &ID3D11DeviceContext, from: usize, n: u32) {
        for i in 0..n as usize {
            self.dispatch(ctx, (from + i) % self.pairs(), false);
        }
    }
}

impl GpuWorkload for StreamLoad {
    fn prepare(
        &mut self,
        sub: &mut dyn Submit,
        target_ms: f64,
        stop: &AtomicBool,
    ) -> Result<(), GpuError> {
        let stopped = || {
            if stop.load(Ordering::Relaxed) {
                Err(GpuError::Stopped)
            } else {
                Ok(())
            }
        };
        for _ in 0..self.wanted {
            stopped()?;
            // Pieces are at most 512 MiB (`chunk_bytes`), so the size fits a `u32`.
            self.pieces
                .push(self.gpu.raw_buffer(self.piece_bytes as u32)?);
        }
        for pair in 0..self.pairs() {
            sub.submit(&mut |ctx| self.dispatch(ctx, pair, true))?;
        }
        sub.finish()?;
        stopped()?;
        let param = calibrate(sub, target_ms, &mut |ctx, param| {
            self.copy(ctx, 0, copies(param))
        })?;
        self.copies = copies(param);
        tracing::info!(
            set_bytes = self.set,
            pieces = self.wanted,
            copies = self.copies,
            target_ms,
            "memory stream calibrated"
        );
        sub.finish()
    }

    fn submit(&mut self, sub: &mut dyn Submit) -> Result<(), GpuError> {
        let (from, n) = (self.next, self.copies);
        sub.submit(&mut |ctx| self.copy(ctx, from, n))?;
        self.next = (from + n as usize) % self.pairs();
        self.submitted += 1;
        Ok(())
    }

    fn check(&mut self, _sub: &mut dyn Submit) -> Result<GpuCheck, GpuError> {
        // No data check (§5.3): the submissions count as checked, as for S5.
        let checks = self.submitted - self.checked;
        self.checked = self.submitted;
        Ok(GpuCheck {
            checks,
            mismatches: Vec::new(),
            notices: std::mem::take(&mut self.notices),
        })
    }

    fn work_per_submission(&self) -> f64 {
        f64::from(self.copies) * f64::from(self.count) * BYTES_PER_ELEMENT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::submit::Submitter;
    use std::time::Instant;

    const GIB: u64 = 1 << 30;

    #[test]
    fn stream_set_is_1_gib_when_it_fits() {
        assert_eq!(stream_set(14 * GIB, 512 * MIB), Some((GIB, 2)));
        assert_eq!(stream_set(14 * GIB, 256 * MIB), Some((GIB, 4)));
        // A chunk that does not divide 1 GiB: smaller pieces, still the whole set.
        assert_eq!(stream_set(14 * GIB, 300 * MIB), Some((GIB, 4)));
        assert_eq!(stream_notices(GIB), [("vram_allocated".to_owned(), GIB)]);
    }

    #[test]
    fn stream_set_is_capped_by_the_vram_target() {
        // The fewest even pieces of at most a chunk, each rounded down to 4 KiB.
        assert_eq!(stream_set(700 * MIB, 256 * MIB), Some((700 * MIB, 4)));
        assert_eq!(stream_set(900 * MIB, 512 * MIB), Some((900 * MIB, 2)));
        assert_eq!(
            stream_set(900 * MIB + 12_345, 512 * MIB),
            Some((900 * MIB + 8192, 2))
        );
        assert_eq!(
            stream_notices(700 * MIB),
            [
                ("vram_allocated".to_owned(), 700 * MIB),
                ("vram_reduced".to_owned(), 700 * MIB),
            ]
        );
    }

    #[test]
    fn under_256_mib_skips_the_stream() {
        assert_eq!(stream_set(256 * MIB, 512 * MIB), Some((256 * MIB, 2)));
        assert_eq!(stream_set(256 * MIB - 1, 512 * MIB), None);
        assert_eq!(stream_set(0, 256 * MIB), None);
    }

    #[test]
    fn calibration_parameter_counts_sixty_fourths_of_a_copy() {
        assert_eq!(copies(64), 1);
        assert_eq!(copies(1), 1);
        assert_eq!(copies(1706), 26);
    }

    /// A small set on the first adapter, never the real 1 GiB in a test: 2 pieces of
    /// 128 MiB, 8 Mi elements, so each dispatch has 2 rows of groups (`GROUPS_X`).
    fn small_stream() -> (GpuDevice, Submitter, StreamLoad) {
        let adapter = oma_win::gpu::stress_adapters()
            .into_iter()
            .next()
            .expect("no hardware GPU");
        let gpu = GpuDevice::open(adapter.luid).unwrap();
        let mut sub = Submitter::new(&gpu).unwrap();
        let mut load = StreamLoad::with_set(&gpu, 256 * MIB, 2, 0x5EED).unwrap();
        load.prepare(&mut sub, 5.0, &AtomicBool::new(false))
            .unwrap();
        (gpu, sub, load)
    }

    /// The whole of `buffer`, `bytes` long.
    fn read(gpu: &GpuDevice, buffer: &ID3D11Buffer, bytes: u64) -> Vec<u8> {
        gpu.read_buffer(buffer, bytes as u32).unwrap()
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn stream_copies_every_piece() {
        let t0 = Instant::now();
        let (gpu, mut sub, mut load) = small_stream();
        for _ in 0..load.pieces.len() {
            load.submit(&mut sub).unwrap();
        }
        sub.finish().unwrap();
        let (pairs, bytes) = (load.pairs(), load.piece_bytes);
        // 4096 words at the head and at the tail, the tail in the second row of groups.
        const WINDOW: usize = 4096 * 4;
        for pair in 0..pairs {
            let src = read(&gpu, &load.pieces[pair].0, bytes);
            let dst = read(&gpu, &load.pieces[pair + pairs].0, bytes);
            let tail = src.len() - WINDOW;
            for at in [0, tail] {
                let window = &src[at..at + WINDOW];
                assert!(
                    window.iter().any(|&b| b != 0),
                    "source {pair} at {at} not filled"
                );
                assert_eq!(window, &dst[at..at + WINDOW], "pair {pair} at {at}");
            }
        }
        let check = load.check(&mut sub).unwrap();
        assert_eq!(check.checks, load.pieces.len() as u64);
        assert_eq!(
            check.notices,
            [
                ("vram_allocated".to_owned(), 256 * MIB),
                ("vram_reduced".to_owned(), 256 * MIB),
            ]
        );
        assert!(t0.elapsed().as_secs_f64() < 2.0, "{:?}", t0.elapsed());
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn stream_bandwidth_is_plausible() {
        const SUBMISSIONS: u32 = 10;
        let t0 = Instant::now();
        let (_gpu, mut sub, mut load) = small_stream();
        sub.window_begin().unwrap();
        for _ in 0..SUBMISSIONS {
            load.submit(&mut sub).unwrap();
        }
        let ms = sub.window_end().unwrap().expect("disjoint window");
        let rate = load.work_per_submission() * f64::from(SUBMISSIONS) * 1e3 / ms;
        println!(
            "{} copies a submission: {:.1} GB/s",
            load.copies,
            rate / 1e9
        );
        assert!((1e9..=5e12).contains(&rate), "{rate}");
        assert!(t0.elapsed().as_secs_f64() < 2.0, "{:?}", t0.elapsed());
    }
}
