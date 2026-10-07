//! S5 and S6, the graphics loads (plan DG11). Both draw the spike's scene off screen at
//! 1920x1080: instanced, rotated, alpha-blended textured quads, with `CullMode = NONE`.
//! - **S5 (fur):** 32 texture reads per pixel, the instances calibrated to the submission
//!   target. No verification: a lost device, a hung submission and the rate count.
//! - **S6 (artifact scan):** 8 reads per pixel, the instances calibrated once per phase and
//!   then fixed. After each frame `tile_hash.hlsl` hashes every 16x16 tile and compares it
//!   with the first frame's; a frame that differs gives one mismatch and an
//!   `artifact_tiles` notice.
//!
//! Every submission binds all the pipeline state it uses and unbinds its render target and
//! views afterwards, so S5 can alternate with S1 on one context (DG9).

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11ComputeShader, ID3D11DeviceContext, ID3D11PixelShader,
    ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView,
    ID3D11Texture2D, ID3D11UnorderedAccessView, ID3D11VertexShader, D3D11_BIND_RENDER_TARGET,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
    D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BLEND_ZERO, D3D11_CULL_NONE, D3D11_FILL_SOLID,
    D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_RASTERIZER_DESC, D3D11_RENDER_TARGET_BLEND_DESC,
    D3D11_SAMPLER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_TEXTURE_ADDRESS_WRAP,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_IMMUTABLE, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};

use super::device::{GpuDevice, GpuError};
use super::engine::{GpuCheck, GpuMismatch, GpuWorkload, PhaseCtx};
use super::shaders::{SCENE_PS, SCENE_VS, TILE_HASH};
use super::submit::{calibrate, Submit};

/// Side of a hashed tile, in pixels.
pub const TILE: u32 = 16;
pub const WIDTH: u32 = 1920;
pub const HEIGHT: u32 = 1080;
/// Texture reads per pixel of each scene (DG11).
const FUR_READS: u32 = 32;
const ARTIFACT_READS: u32 = 8;
/// Result slots of S6, one per submission between two reads; a full ring is read early.
const SLOTS: u32 = 256;
/// A slot as reset: no differing tile, lowest differing tile `MAX`.
const SLOT_RESET: [u32; 4] = [0, u32::MAX, 0, 0];
/// The submission after which the injected fault flips the first reference hash (DA18).
const INJECT_AT: u64 = 3;
/// Modes of `tile_hash.hlsl`.
const HASH_REFERENCE: u32 = 0;
const HASH_COMPARE: u32 = 1;
const HASH_RECORD: u32 = 2;
const HASH_FLIP: u32 = 3;

/// Tiles across and down an image of `width` by `height`, the last ones partial.
pub fn tile_count(width: u32, height: u32) -> (u32, u32) {
    (width.div_ceil(TILE), height.div_ceil(TILE))
}

/// FNV-1a 32 of `bytes`, what `tile_hash.hlsl` computes over a tile's RGBA8 bytes.
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811C_9DC5, |h, &b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsKind {
    /// S5.
    Fur,
    /// S6.
    Artifact,
}

/// The tile hashes and per-submission results of S6.
struct Scan {
    shader: ID3D11ComputeShader,
    constants: ID3D11Buffer,
    target_srv: ID3D11ShaderResourceView,
    /// Read back only by the tests; the views keep the resource alive anyway.
    #[cfg_attr(not(test), allow(dead_code))]
    refs: ID3D11Buffer,
    refs_uav: ID3D11UnorderedAccessView,
    results: ID3D11Buffer,
    results_uav: ID3D11UnorderedAccessView,
}

pub struct GraphicsWorkload {
    kind: GraphicsKind,
    gpu: GpuDevice,
    /// The scene's parameter: the same frame and instances draw the same image.
    frame: u32,
    instances: u32,
    inject: bool,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    constants: ID3D11Buffer,
    texture_srv: ID3D11ShaderResourceView,
    sampler: ID3D11SamplerState,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    /// Read back only by the tests; the views keep the resource alive anyway.
    #[cfg_attr(not(test), allow(dead_code))]
    target: ID3D11Texture2D,
    target_rtv: ID3D11RenderTargetView,
    scan: Option<Scan>,
    /// Submissions after the reference frame.
    submitted: u64,
    checked: u64,
    /// Verdicts read early (a full ring) and not yet handed to the engine.
    pending: GpuCheck,
}

impl GraphicsWorkload {
    pub fn new(kind: GraphicsKind, gpu: &GpuDevice, ctx: &PhaseCtx) -> Result<Self, GpuError> {
        let dev = gpu.device();
        let failed = |e: windows::core::Error| gpu.error(&e);
        let (mut vs, mut ps) = (None, None);
        // A procedural 256x256 texture, as in the spike.
        let texels: Vec<u32> = (0..256 * 256u32)
            .map(|i| i.wrapping_mul(2_654_435_761) | 0xFF00_0000)
            .collect();
        let texture_desc = D3D11_TEXTURE2D_DESC {
            Width: 256,
            Height: 256,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: texels.as_ptr().cast(),
            SysMemPitch: 256 * 4,
            SysMemSlicePitch: 0,
        };
        let target_desc = D3D11_TEXTURE2D_DESC {
            Width: WIDTH,
            Height: HEIGHT,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            ..texture_desc
        };
        let sampler_desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
            AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
            AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        let mut blend_desc = D3D11_BLEND_DESC::default();
        blend_desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D11_BLEND_SRC_ALPHA,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_ZERO,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: 0xF,
        };
        let raster_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            DepthClipEnable: true.into(),
            ..Default::default()
        };
        // A creation that succeeds without an object: E_FAIL, as in `device.rs`.
        let missing = GpuError::Create(0x8000_4005_u32 as i32);
        let (mut texture, mut texture_srv, mut sampler, mut blend, mut raster) =
            (None, None, None, None, None);
        let (mut target, mut target_rtv, mut target_srv) = (None, None, None);
        // SAFETY: fxc output for vs_5_0/ps_5_0 without class linkage; complete
        // descriptions; the texture's initial data is `texels`, 256 rows of 1024 bytes,
        // alive for the call; resources of this device for the views; live out pointers.
        unsafe {
            dev.CreateVertexShader(SCENE_VS, None, Some(&mut vs))
                .map_err(failed)?;
            dev.CreatePixelShader(SCENE_PS, None, Some(&mut ps))
                .map_err(failed)?;
            dev.CreateTexture2D(&texture_desc, Some(&init), Some(&mut texture))
                .map_err(failed)?;
            let texture = texture.ok_or(missing)?;
            dev.CreateShaderResourceView(&texture, None, Some(&mut texture_srv))
                .map_err(failed)?;
            dev.CreateSamplerState(&sampler_desc, Some(&mut sampler))
                .map_err(failed)?;
            dev.CreateBlendState(&blend_desc, Some(&mut blend))
                .map_err(failed)?;
            dev.CreateRasterizerState(&raster_desc, Some(&mut raster))
                .map_err(failed)?;
            dev.CreateTexture2D(&target_desc, None, Some(&mut target))
                .map_err(failed)?;
            let t = target.as_ref().ok_or(missing)?;
            dev.CreateRenderTargetView(t, None, Some(&mut target_rtv))
                .map_err(failed)?;
            dev.CreateShaderResourceView(t, None, Some(&mut target_srv))
                .map_err(failed)?;
        }
        let scan = match kind {
            GraphicsKind::Fur => None,
            GraphicsKind::Artifact => {
                let (cols, rows) = tile_count(WIDTH, HEIGHT);
                let (refs, refs_uav, _) = gpu.structured_buffer(cols * rows * 4, 4)?;
                let (results, results_uav, _) = gpu.structured_buffer(SLOTS * 16, 4)?;
                Some(Scan {
                    shader: gpu.compute_shader(TILE_HASH)?,
                    constants: gpu.constant_buffer()?,
                    target_srv: target_srv.ok_or(missing)?,
                    refs,
                    refs_uav,
                    results,
                    results_uav,
                })
            }
        };
        Ok(GraphicsWorkload {
            kind,
            gpu: gpu.clone(),
            frame: (ctx.seed ^ (ctx.seed >> 32)) as u32,
            instances: 0,
            inject: ctx.inject.is_some() && kind == GraphicsKind::Artifact,
            vs: vs.ok_or(missing)?,
            ps: ps.ok_or(missing)?,
            constants: gpu.constant_buffer()?,
            texture_srv: texture_srv.ok_or(missing)?,
            sampler: sampler.ok_or(missing)?,
            blend: blend.ok_or(missing)?,
            raster: raster.ok_or(missing)?,
            target: target.ok_or(missing)?,
            target_rtv: target_rtv.ok_or(missing)?,
            scan,
            submitted: 0,
            checked: 0,
            pending: GpuCheck::default(),
        })
    }

    fn reads(&self) -> u32 {
        match self.kind {
            GraphicsKind::Fur => FUR_READS,
            GraphicsKind::Artifact => ARTIFACT_READS,
        }
    }

    /// Clears the render target and draws `instances` quads. `ctx` must be
    /// `self.gpu.context()`: the constants are written through the device's context and
    /// must land in order with the draw.
    fn draw(&self, ctx: &ID3D11DeviceContext, instances: u32) {
        debug_assert_eq!(
            ctx.as_raw(),
            self.gpu.context().as_raw(),
            "the submission queue must wrap the load's own device context"
        );
        self.gpu
            .set_constants(&self.constants, [self.frame, self.reads(), 0, 0]);
        let viewport = D3D11_VIEWPORT {
            Width: WIDTH as f32,
            Height: HEIGHT as f32,
            MaxDepth: 1.0,
            ..Default::default()
        };
        // SAFETY: `ctx` is this device's immediate context (asserted above), used only on
        // this thread; every object is of this device. All the state the draw reads is set
        // here (no vertex buffers: the vertex shader builds the quads from the vertex and
        // instance ids), and the render target is unbound afterwards so S6 can read it.
        unsafe {
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&self.vs, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            ctx.PSSetShaderResources(0, Some(&[Some(self.texture_srv.clone())]));
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            ctx.RSSetState(&self.raster);
            ctx.RSSetViewports(Some(&[viewport]));
            ctx.OMSetBlendState(&self.blend, None, 0xFFFF_FFFF);
            ctx.OMSetDepthStencilState(None, 0);
            ctx.OMSetRenderTargets(Some(&[Some(self.target_rtv.clone())]), None);
            ctx.ClearRenderTargetView(&self.target_rtv, &[0.0, 0.0, 0.0, 1.0]);
            ctx.DrawInstanced(6, instances, 0, 0);
            ctx.OMSetRenderTargets(None, None);
        }
    }

    /// One pass of `tile_hash.hlsl` in `mode` on result slot `slot`.
    fn hash(&self, ctx: &ID3D11DeviceContext, mode: u32, slot: u32) {
        let scan = self.scan.as_ref().expect("only S6 hashes its frames");
        self.gpu
            .set_constants(&scan.constants, [mode, slot, WIDTH, HEIGHT]);
        let (cols, rows) = tile_count(WIDTH, HEIGHT);
        let groups = match mode {
            HASH_REFERENCE | HASH_COMPARE => (cols * rows).div_ceil(64),
            _ => 1,
        };
        let uavs = [Some(scan.refs_uav.clone()), Some(scan.results_uav.clone())];
        // SAFETY: `ctx` is this device's context (the callers come from `draw`'s queue, see
        // its assert), so the constants written above land before the dispatch; resources
        // of this device; the render target is not bound for output (`draw` unbinds it);
        // two UAVs from `uavs`, alive for the call; views unbound afterwards so the render
        // target can be drawn to again and other loads find the slots free.
        unsafe {
            ctx.CSSetShader(&scan.shader, None);
            ctx.CSSetConstantBuffers(0, Some(&[Some(scan.constants.clone())]));
            ctx.CSSetShaderResources(0, Some(&[Some(scan.target_srv.clone())]));
            ctx.CSSetUnorderedAccessViews(0, 2, Some(uavs.as_ptr()), None);
            ctx.Dispatch(groups, 1, 1);
            ctx.CSSetShaderResources(0, Some(&[None]));
            ctx.CSSetUnorderedAccessViews(0, 2, Some([None, None].as_ptr()), None);
        }
    }

    fn reset_results(&self) {
        if let Some(scan) = &self.scan {
            self.gpu
                .write_words(&scan.results, &SLOT_RESET.repeat(SLOTS as usize));
        }
    }

    /// Reads the results of the submissions since the last read into `pending`, then
    /// resets them.
    fn collect(&mut self, sub: &mut dyn Submit) -> Result<(), GpuError> {
        sub.finish()?;
        if let Some(scan) = &self.scan {
            let words: Vec<u32> = self
                .gpu
                .read_buffer(&scan.results, SLOTS * 16)?
                .chunks_exact(4)
                .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
                .collect();
            for n in self.checked + 1..=self.submitted {
                let slot = &words[slot_of(n) as usize * 4..][..4];
                if slot[0] > 0 {
                    self.pending.mismatches.push(GpuMismatch {
                        iteration: n,
                        expected: slot[2].into(),
                        actual: slot[3].into(),
                    });
                    self.pending
                        .notices
                        .push(("artifact_tiles".to_owned(), slot[0].into()));
                }
            }
            self.reset_results();
        }
        self.pending.checks += self.submitted - self.checked;
        self.checked = self.submitted;
        Ok(())
    }
}

/// The result slot of submission `n` (from 1).
fn slot_of(n: u64) -> u32 {
    ((n - 1) % u64::from(SLOTS)) as u32
}

impl GpuWorkload for GraphicsWorkload {
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
        stopped()?;
        // S6 calibrates the frame with its hash pass, so a submission is what it measures.
        let artifact = self.kind == GraphicsKind::Artifact;
        self.instances = calibrate(sub, target_ms, &mut |ctx, instances| {
            self.draw(ctx, instances);
            if artifact {
                self.hash(ctx, HASH_REFERENCE, 0);
            }
        })?;
        tracing::info!(kind = ?self.kind, instances = self.instances, target_ms, "graphics load calibrated");
        stopped()?;
        if artifact {
            // The reference frame: the hashes the later frames are compared with.
            let instances = self.instances;
            sub.submit(&mut |ctx| {
                self.draw(ctx, instances);
                self.hash(ctx, HASH_REFERENCE, 0);
            })?;
            self.reset_results();
        }
        sub.finish()
    }

    fn submit(&mut self, sub: &mut dyn Submit) -> Result<(), GpuError> {
        if self.scan.is_some() && self.submitted - self.checked == u64::from(SLOTS) {
            self.collect(sub)?;
        }
        self.submitted += 1;
        let (n, instances) = (self.submitted, self.instances);
        sub.submit(&mut |ctx| {
            self.draw(ctx, instances);
            if self.scan.is_some() {
                self.hash(ctx, HASH_COMPARE, slot_of(n));
                self.hash(ctx, HASH_RECORD, slot_of(n));
            }
        })?;
        if self.inject && n == INJECT_AT {
            // Bit 0 of the first tile's reference hash: every later frame differs (DA18).
            sub.submit(&mut |ctx| self.hash(ctx, HASH_FLIP, 0))?;
            tracing::warn!("fault injected into the reference tile hashes");
        }
        Ok(())
    }

    fn check(&mut self, sub: &mut dyn Submit) -> Result<GpuCheck, GpuError> {
        self.collect(sub)?;
        Ok(std::mem::take(&mut self.pending))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Inject;
    use crate::gpu::submit::Submitter;
    use oma_ipc::load::KernelId;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_USAGE_STAGING,
    };

    #[test]
    fn tile_grid_covers_1080p() {
        assert_eq!(tile_count(WIDTH, HEIGHT), (120, 68));
        assert_eq!(tile_count(16, 16), (1, 1));
        assert_eq!(tile_count(17, 1), (2, 1));
    }

    #[test]
    fn cpu_tile_hash_matches_known_vector() {
        // The published FNV-1a 32 test vectors.
        assert_eq!(fnv1a32(b""), 0x811C_9DC5);
        assert_eq!(fnv1a32(b"a"), 0xE40C_292C);
        assert_eq!(fnv1a32(b"foobar"), 0xBF9C_F968);
    }

    fn first_gpu() -> (GpuDevice, bool) {
        let adapter = oma_win::gpu::stress_adapters()
            .into_iter()
            .next()
            .expect("no hardware GPU");
        (GpuDevice::open(adapter.luid).unwrap(), adapter.integrated)
    }

    fn ctx(integrated: bool, inject: bool) -> PhaseCtx {
        PhaseCtx {
            seed: 0x0123_4567_89AB_CDEF,
            integrated,
            inject: inject.then_some(Inject {
                kernel: KernelId::S6,
                core: None,
            }),
            budget: Default::default(),
        }
    }

    /// The render target's RGBA8 bytes, `WIDTH * 4` per row.
    fn read_target(load: &GraphicsWorkload) -> Vec<u8> {
        let dev = load.gpu.device();
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        let mut staging = None;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: test-only readback on the test's thread: a staging copy of the render
        // target's description, mapped for reading; each row is copied out before Unmap
        // and lies within `RowPitch * HEIGHT` bytes of the mapping.
        unsafe {
            load.target.GetDesc(&mut desc);
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            dev.CreateTexture2D(&desc, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            let ctx = load.gpu.context();
            ctx.CopyResource(&staging, &load.target);
            ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
            let mut bytes = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
            for y in 0..HEIGHT as usize {
                let row = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize);
                bytes.extend_from_slice(std::slice::from_raw_parts(row, WIDTH as usize * 4));
            }
            ctx.Unmap(&staging, 0);
            bytes
        }
    }

    /// The bytes of tile `t` of `image`, row by row, cut at the image edge.
    fn tile_bytes(image: &[u8], t: u32) -> Vec<u8> {
        let (cols, _) = tile_count(WIDTH, HEIGHT);
        let (x0, y0) = ((t % cols) * TILE, (t / cols) * TILE);
        let mut out = Vec::new();
        for y in y0..(y0 + TILE).min(HEIGHT) {
            let start = ((y * WIDTH + x0) * 4) as usize;
            out.extend_from_slice(&image[start..start + (TILE.min(WIDTH - x0) * 4) as usize]);
        }
        out
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn artifact_scene_is_deterministic() {
        // Every adapter: the integrated GPU is where an uncalibrated scene took seconds.
        for adapter in oma_win::gpu::stress_adapters() {
            let gpu = GpuDevice::open(adapter.luid).unwrap();
            let started = std::time::Instant::now();
            deterministic_on(&gpu, adapter.integrated);
            println!("{}: {:?}", adapter.name, started.elapsed());
        }
    }

    fn deterministic_on(gpu: &GpuDevice, integrated: bool) {
        let mut sub = Submitter::new(gpu).unwrap();
        let mut load =
            GraphicsWorkload::new(GraphicsKind::Artifact, gpu, &ctx(integrated, false)).unwrap();
        // Short frames: the test stays far below its GPU budget.
        load.prepare(&mut sub, 10.0, &AtomicBool::new(false))
            .unwrap();
        // The reference hashes are those of the CPU on the frame's bytes, edge tiles too.
        let image = read_target(&load);
        let scan = load.scan.as_ref().unwrap();
        let refs: Vec<u32> = gpu
            .read_buffer(&scan.refs, 120 * 68 * 4)
            .unwrap()
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let lit = image.chunks(4).filter(|p| p[0] | p[1] | p[2] != 0).count();
        assert!(lit > 0, "the scene drew nothing");
        for t in [0, 1, 119, 120, 4000, 120 * 67, 120 * 68 - 1] {
            assert_eq!(
                refs[t as usize],
                fnv1a32(&tile_bytes(&image, t)),
                "tile {t}"
            );
        }
        for _ in 0..5 {
            load.submit(&mut sub).unwrap();
        }
        let check = load.check(&mut sub).unwrap();
        assert_eq!(check.checks, 5);
        assert_eq!(check.mismatches, []);
        assert_eq!(check.notices, []);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn artifact_scan_detects_a_changed_frame() {
        let (gpu, integrated) = first_gpu();
        let mut sub = Submitter::new(&gpu).unwrap();
        // A changed scene parameter: every later frame draws another image.
        let mut load =
            GraphicsWorkload::new(GraphicsKind::Artifact, &gpu, &ctx(integrated, false)).unwrap();
        load.prepare(&mut sub, 10.0, &AtomicBool::new(false))
            .unwrap();
        load.frame ^= 1;
        for _ in 0..3 {
            load.submit(&mut sub).unwrap();
        }
        let check = load.check(&mut sub).unwrap();
        assert_eq!(check.checks, 3);
        assert_eq!(check.mismatches.len(), 3, "one mismatch per bad frame");
        assert_eq!(check.mismatches[0].iteration, 1);
        assert_ne!(check.mismatches[0].expected, check.mismatches[0].actual);
        assert_eq!(check.notices.len(), 3);
        for (code, tiles) in &check.notices {
            assert_eq!(code, "artifact_tiles");
            assert!(*tiles > 0 && *tiles <= 120 * 68, "{tiles}");
        }
        // The fault injection: the first tile's reference flipped after submission 3.
        let mut load =
            GraphicsWorkload::new(GraphicsKind::Artifact, &gpu, &ctx(integrated, true)).unwrap();
        load.prepare(&mut sub, 10.0, &AtomicBool::new(false))
            .unwrap();
        for _ in 0..5 {
            load.submit(&mut sub).unwrap();
        }
        let check = load.check(&mut sub).unwrap();
        let iterations: Vec<u64> = check.mismatches.iter().map(|m| m.iteration).collect();
        assert_eq!(iterations, [4, 5]);
        assert_eq!(check.mismatches[0].expected ^ check.mismatches[0].actual, 1);
        assert_eq!(check.notices[0], ("artifact_tiles".to_owned(), 1));
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn fur_load_calibrates_near_the_target() {
        let (gpu, integrated) = first_gpu();
        let mut sub = Submitter::new(&gpu).unwrap();
        let mut load =
            GraphicsWorkload::new(GraphicsKind::Fur, &gpu, &ctx(integrated, false)).unwrap();
        let target = 20.0;
        load.prepare(&mut sub, target, &AtomicBool::new(false))
            .unwrap();
        let instances = load.instances;
        let ms = sub.gpu_ms(&mut |ctx| load.draw(ctx, instances)).unwrap();
        assert!(
            (target * 0.5..=target * 2.0).contains(&ms),
            "{instances} instances took {ms} ms"
        );
        load.submit(&mut sub).unwrap();
        let check = load.check(&mut sub).unwrap();
        assert_eq!((check.checks, check.mismatches.len()), (1, 0));
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn fur_alternates_with_s1_on_one_context() {
        // The normal profile's S5 + S1 (DG9): neither load may depend on state left bound
        // by the other, so S1 still matches its golden output and S6 its reference.
        let (gpu, integrated) = first_gpu();
        let mut sub = Submitter::new(&gpu).unwrap();
        let phase = ctx(integrated, false);
        let stop = AtomicBool::new(false);
        let mut s1 = crate::gpu::compute::ComputeLoad::new(KernelId::S1, &gpu, &phase).unwrap();
        let mut fur = GraphicsWorkload::new(GraphicsKind::Fur, &gpu, &phase).unwrap();
        let mut scan = GraphicsWorkload::new(GraphicsKind::Artifact, &gpu, &phase).unwrap();
        s1.prepare(&mut sub, 5.0, &stop).unwrap();
        fur.prepare(&mut sub, 5.0, &stop).unwrap();
        scan.prepare(&mut sub, 5.0, &stop).unwrap();
        for _ in 0..4 {
            s1.submit(&mut sub).unwrap();
            fur.submit(&mut sub).unwrap();
            scan.submit(&mut sub).unwrap();
        }
        for load in [&mut s1 as &mut dyn GpuWorkload, &mut fur, &mut scan] {
            let check = load.check(&mut sub).unwrap();
            assert_eq!((check.checks, check.mismatches), (4, vec![]));
        }
    }
}
