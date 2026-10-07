//! The graphics loads of the GPU benchmark (plan DH2): `fill`, `texture` and `overdraw`,
//! drawn off screen on a render target of 1920x1080 (RGBA8, RGBA16F for `overdraw`) with
//! instanced quads as large as the target, so a submission of `n` instances covers exactly
//! `n * 1920 * 1080` pixels.
//! - **fill:** opaque quads of a constant color, no texture, no blending (Gpixel/s);
//! - **texture:** opaque quads with [`TEXTURE_READS`] independent bilinear reads per pixel
//!   from a 1024x1024 RGBA8 texture (Gtexel/s, texels = pixels x 8);
//! - **overdraw:** quads of a constant color with alpha blending into an RGBA16F target
//!   (Gpixel/s): on RGBA8 a GPU that blends at full rate runs it at the same ROP limit as
//!   `fill` (user decision of 2026-10-07).
//!
//! The instances are calibrated to the submission target (DG4). No verification, as for
//! S5: a lost device and a hung submission count. Every submission binds the pipeline
//! state it uses and unbinds its render target and texture afterwards.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11DeviceContext, ID3D11PixelShader, ID3D11RasterizerState,
    ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11VertexShader,
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BLEND_DESC,
    D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE, D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA,
    D3D11_BLEND_ZERO, D3D11_CULL_NONE, D3D11_FILL_SOLID, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
    D3D11_RASTERIZER_DESC, D3D11_RENDER_TARGET_BLEND_DESC, D3D11_SAMPLER_DESC,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_TEXTURE_ADDRESS_WRAP, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_IMMUTABLE, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};

use super::device::{GpuDevice, GpuError};
use super::engine::{GpuCheck, GpuWorkload};
use super::graphics::{HEIGHT, WIDTH};
use super::shaders::{BENCH_GFX_PS, BENCH_GFX_VS};
use super::submit::{calibrate, Submit};

/// Bilinear texture reads per pixel of `texture` (DH2).
pub const TEXTURE_READS: u32 = 8;
/// Side of the texture of `texture`, in texels.
const TEXTURE_SIDE: u32 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchGfxKind {
    Fill,
    Texture,
    Overdraw,
}

/// Pixels (`fill`, `overdraw`) or texels (`texture`) of a submission of `instances` quads.
pub fn gfx_work(kind: BenchGfxKind, instances: u32) -> f64 {
    let pixels = f64::from(instances) * f64::from(WIDTH) * f64::from(HEIGHT);
    match kind {
        BenchGfxKind::Texture => pixels * f64::from(TEXTURE_READS),
        BenchGfxKind::Fill | BenchGfxKind::Overdraw => pixels,
    }
}

pub struct BenchGfxLoad {
    kind: BenchGfxKind,
    gpu: GpuDevice,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    constants: ID3D11Buffer,
    texture_srv: ID3D11ShaderResourceView,
    sampler: ID3D11SamplerState,
    /// Alpha blending for `overdraw`; `None` (no blending) for the others.
    blend: Option<ID3D11BlendState>,
    raster: ID3D11RasterizerState,
    /// The view keeps the render target alive.
    target_rtv: ID3D11RenderTargetView,
    /// Quads per submission (the calibrated parameter, DG4).
    instances: u32,
    submitted: u64,
    checked: u64,
}

impl BenchGfxLoad {
    pub fn new(kind: BenchGfxKind, gpu: &GpuDevice) -> Result<Self, GpuError> {
        let dev = gpu.device();
        let failed = |e: windows::core::Error| gpu.error(&e);
        let texels: Vec<u32> = (0..TEXTURE_SIDE * TEXTURE_SIDE)
            .map(|i| i.wrapping_mul(2_654_435_761) | 0xFF00_0000)
            .collect();
        let texture_desc = D3D11_TEXTURE2D_DESC {
            Width: TEXTURE_SIDE,
            Height: TEXTURE_SIDE,
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
            SysMemPitch: TEXTURE_SIDE * 4,
            SysMemSlicePitch: 0,
        };
        let target_desc = D3D11_TEXTURE2D_DESC {
            Width: WIDTH,
            Height: HEIGHT,
            Format: if kind == BenchGfxKind::Overdraw {
                DXGI_FORMAT_R16G16B16A16_FLOAT
            } else {
                DXGI_FORMAT_R8G8B8A8_UNORM
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            ..texture_desc
        };
        // One mip level, so MIN_MAG_MIP_LINEAR is bilinear.
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
        let (mut vs, mut ps, mut texture, mut texture_srv) = (None, None, None, None);
        let (mut sampler, mut blend, mut raster, mut target, mut target_rtv) =
            (None, None, None, None, None);
        // SAFETY: fxc output for vs_5_0/ps_5_0 without class linkage; complete
        // descriptions; the texture's initial data is `texels`, 1024 rows of 4096 bytes,
        // alive for the call; the render target is 1920x1080 in the format of `kind`;
        // resources of this device for the views; live out pointers.
        unsafe {
            dev.CreateVertexShader(BENCH_GFX_VS, None, Some(&mut vs))
                .map_err(failed)?;
            dev.CreatePixelShader(BENCH_GFX_PS, None, Some(&mut ps))
                .map_err(failed)?;
            dev.CreateTexture2D(&texture_desc, Some(&init), Some(&mut texture))
                .map_err(failed)?;
            let texture = texture.ok_or(missing)?;
            dev.CreateShaderResourceView(&texture, None, Some(&mut texture_srv))
                .map_err(failed)?;
            dev.CreateSamplerState(&sampler_desc, Some(&mut sampler))
                .map_err(failed)?;
            if kind == BenchGfxKind::Overdraw {
                dev.CreateBlendState(&blend_desc, Some(&mut blend))
                    .map_err(failed)?;
                blend.as_ref().ok_or(missing)?;
            }
            dev.CreateRasterizerState(&raster_desc, Some(&mut raster))
                .map_err(failed)?;
            dev.CreateTexture2D(&target_desc, None, Some(&mut target))
                .map_err(failed)?;
            let target = target.ok_or(missing)?;
            dev.CreateRenderTargetView(&target, None, Some(&mut target_rtv))
                .map_err(failed)?;
        }
        Ok(BenchGfxLoad {
            kind,
            gpu: gpu.clone(),
            vs: vs.ok_or(missing)?,
            ps: ps.ok_or(missing)?,
            constants: gpu.constant_buffer()?,
            texture_srv: texture_srv.ok_or(missing)?,
            sampler: sampler.ok_or(missing)?,
            blend,
            raster: raster.ok_or(missing)?,
            target_rtv: target_rtv.ok_or(missing)?,
            instances: 0,
            submitted: 0,
            checked: 0,
        })
    }

    /// Draws `instances` quads. `ctx` must be `self.gpu.context()`: the constants are
    /// written through the device's context and must land in order with the draw.
    fn draw(&self, ctx: &ID3D11DeviceContext, instances: u32) {
        debug_assert_eq!(
            ctx.as_raw(),
            self.gpu.context().as_raw(),
            "the submission queue must wrap the load's own device context"
        );
        let textured = u32::from(self.kind == BenchGfxKind::Texture);
        self.gpu.set_constants(&self.constants, [textured, 0, 0, 0]);
        let viewport = D3D11_VIEWPORT {
            Width: WIDTH as f32,
            Height: HEIGHT as f32,
            MaxDepth: 1.0,
            ..Default::default()
        };
        // SAFETY: `ctx` is this device's immediate context (asserted above), used only on
        // this thread; every object is of this device. All the state the draw reads is set
        // here (no vertex buffers: the vertex shader builds the quads from the vertex id;
        // no depth buffer), and the render target and texture are unbound afterwards, so
        // other loads on the context find the slots free.
        unsafe {
            ctx.IASetInputLayout(None);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            ctx.PSSetShaderResources(0, Some(&[Some(self.texture_srv.clone())]));
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            ctx.RSSetState(&self.raster);
            ctx.RSSetViewports(Some(&[viewport]));
            ctx.OMSetBlendState(self.blend.as_ref(), None, 0xFFFF_FFFF);
            ctx.OMSetDepthStencilState(None, 0);
            ctx.OMSetRenderTargets(Some(&[Some(self.target_rtv.clone())]), None);
            ctx.DrawInstanced(6, instances, 0, 0);
            ctx.OMSetRenderTargets(None, None);
            ctx.PSSetShaderResources(0, Some(&[None]));
        }
    }
}

impl GpuWorkload for BenchGfxLoad {
    fn prepare(
        &mut self,
        sub: &mut dyn Submit,
        target_ms: f64,
        stop: &AtomicBool,
    ) -> Result<(), GpuError> {
        if stop.load(Ordering::Relaxed) {
            return Err(GpuError::Stopped);
        }
        self.instances = calibrate(sub, target_ms, &mut |ctx, instances| {
            self.draw(ctx, instances)
        })?;
        tracing::info!(kind = ?self.kind, instances = self.instances, target_ms, "bench graphics load calibrated");
        sub.finish()
    }

    fn submit(&mut self, sub: &mut dyn Submit) -> Result<(), GpuError> {
        let instances = self.instances;
        sub.submit(&mut |ctx| self.draw(ctx, instances))?;
        self.submitted += 1;
        Ok(())
    }

    fn check(&mut self, _sub: &mut dyn Submit) -> Result<GpuCheck, GpuError> {
        // No verification (DH2): the submissions count as checked, as for S5.
        let checks = self.submitted - self.checked;
        self.checked = self.submitted;
        Ok(GpuCheck {
            checks,
            ..GpuCheck::default()
        })
    }

    fn work_per_submission(&self) -> f64 {
        gfx_work(self.kind, self.instances)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::submit::Submitter;
    use std::time::{Duration, Instant};

    const KINDS: [BenchGfxKind; 3] = [
        BenchGfxKind::Fill,
        BenchGfxKind::Texture,
        BenchGfxKind::Overdraw,
    ];

    #[test]
    fn gfx_work_counts_full_screen_pixels() {
        assert_eq!(gfx_work(BenchGfxKind::Fill, 10), 20_736_000.0);
        assert_eq!(gfx_work(BenchGfxKind::Overdraw, 10), 20_736_000.0);
        assert_eq!(gfx_work(BenchGfxKind::Texture, 10), 165_888_000.0);
    }

    fn first_gpu() -> GpuDevice {
        let adapter = oma_win::gpu::stress_adapters()
            .into_iter()
            .next()
            .expect("no hardware GPU");
        GpuDevice::open(adapter.luid).unwrap()
    }

    /// The rate of `kind` in units a second: 10 ms submissions for `warm` untimed, then
    /// one window of `window`.
    fn rate(gpu: &GpuDevice, kind: BenchGfxKind, warm: Duration, window: Duration) -> f64 {
        let mut sub = Submitter::new(gpu).unwrap();
        let mut load = BenchGfxLoad::new(kind, gpu).unwrap();
        load.prepare(&mut sub, 10.0, &AtomicBool::new(false))
            .unwrap();
        let t0 = Instant::now();
        while t0.elapsed() < warm {
            load.submit(&mut sub).unwrap();
        }
        sub.window_begin().unwrap();
        let (t0, mut n) = (Instant::now(), 0u32);
        while t0.elapsed() < window {
            load.submit(&mut sub).unwrap();
            n += 1;
        }
        let ms = sub.window_end().unwrap().expect("disjoint timestamps");
        load.check(&mut sub).unwrap();
        load.work_per_submission() * f64::from(n) * 1e3 / ms
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn bench_gfx_calibrates_near_the_target() {
        let gpu = first_gpu();
        let mut sub = Submitter::new(&gpu).unwrap();
        let target = 20.0;
        for kind in KINDS {
            let mut load = BenchGfxLoad::new(kind, &gpu).unwrap();
            load.prepare(&mut sub, target, &AtomicBool::new(false))
                .unwrap();
            let instances = load.instances;
            let ms = sub.gpu_ms(&mut |ctx| load.draw(ctx, instances)).unwrap();
            assert!(
                (target * 0.5..=target * 2.0).contains(&ms),
                "{kind:?}: {instances} instances took {ms} ms"
            );
            load.submit(&mut sub).unwrap();
            assert_eq!(load.check(&mut sub).unwrap().checks, 1);
        }
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn overdraw_is_slower_than_fill() {
        let gpu = first_gpu();
        let (warm, window) = (Duration::from_millis(300), Duration::from_millis(300));
        // Overdraw first, so fill never runs on colder clocks.
        let overdraw = rate(&gpu, BenchGfxKind::Overdraw, warm, window);
        let fill = rate(&gpu, BenchGfxKind::Fill, warm, window);
        println!(
            "fill {:.1} Gpixel/s, overdraw {:.1} Gpixel/s",
            fill / 1e9,
            overdraw / 1e9
        );
        // FP16 blending runs at half the ROP rate (RTX 4080: 138.7 against 275.7 Gpixel/s).
        assert!(overdraw < fill, "overdraw {overdraw} >= fill {fill}");
    }
}
