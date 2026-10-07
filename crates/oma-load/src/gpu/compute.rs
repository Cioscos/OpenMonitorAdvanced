//! S1 and S2, the verified compute loads (plan DG5). Every submission runs the kernel with
//! the same steps and seed, then `compare.hlsl` against the golden output of the first
//! one; the counters are read and reset once a second.

use std::sync::atomic::{AtomicBool, Ordering};

use oma_ipc::load::KernelId;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Buffer, ID3D11ComputeShader, ID3D11DeviceContext, ID3D11ShaderResourceView,
    ID3D11UnorderedAccessView,
};

use super::device::{GpuDevice, GpuError};
use super::engine::{GpuCheck, GpuMismatch, GpuWorkload, PhaseCtx};
use super::reference::{fma_params, fma_thread, hash_params, hash_thread};
use super::shaders::{COMPARE, COUNTERS_RESET, S1_FMA, S2_HASH};
use super::submit::{calibrate, Submit};

/// 16 MB of output, a `uint4` per thread.
const THREADS: u32 = 1 << 20;
const GROUPS: u32 = THREADS / 256;
/// The CPU sample check: 1 thread in `SAMPLE_EVERY`, `SAMPLE_STEPS` steps.
const SAMPLE_EVERY: usize = 97;
const SAMPLE_STEPS: u32 = 1000;
/// The submission after which the injected fault flips the golden (DA18).
const INJECT_AT: u64 = 3;

type Params = fn(u32, u32) -> [u32; 4];
type Reference = fn(u32, u32, u32) -> [u32; 4];

pub struct ComputeLoad {
    gpu: GpuDevice,
    params: Params,
    reference: Reference,
    seed: u32,
    steps: u32,
    inject: bool,
    kernel: ID3D11ComputeShader,
    compare: ID3D11ComputeShader,
    constants: ID3D11Buffer,
    compare_constants: ID3D11Buffer,
    out: ID3D11Buffer,
    out_uav: ID3D11UnorderedAccessView,
    out_srv: ID3D11ShaderResourceView,
    golden: ID3D11Buffer,
    golden_srv: ID3D11ShaderResourceView,
    counters: ID3D11Buffer,
    counters_uav: ID3D11UnorderedAccessView,
    /// Submissions after the golden one.
    submitted: u64,
    checked: u64,
}

impl ComputeLoad {
    /// S1 or S2 on `gpu`; panics on another kernel.
    pub fn new(kernel: KernelId, gpu: &GpuDevice, ctx: &PhaseCtx) -> Result<Self, GpuError> {
        let (bytecode, params, reference): (&[u8], Params, Reference) = match kernel {
            KernelId::S1 => (S1_FMA, fma_params, fma_thread),
            KernelId::S2 => (S2_HASH, hash_params, hash_thread),
            other => panic!("{other:?} is not a compute load"),
        };
        let (out, out_uav, out_srv) = gpu.structured_buffer(THREADS * 16, 16)?;
        let (golden, _, golden_srv) = gpu.structured_buffer(THREADS * 16, 16)?;
        let (counters, counters_uav, _) = gpu.structured_buffer(32, 4)?;
        Ok(ComputeLoad {
            params,
            reference,
            seed: (ctx.seed ^ (ctx.seed >> 32)) as u32,
            steps: 0,
            inject: ctx.inject.is_some(),
            kernel: gpu.compute_shader(bytecode)?,
            compare: gpu.compute_shader(COMPARE)?,
            constants: gpu.constant_buffer()?,
            compare_constants: gpu.constant_buffer()?,
            out,
            out_uav,
            out_srv,
            golden,
            golden_srv,
            counters,
            counters_uav,
            submitted: 0,
            checked: 0,
            gpu: gpu.clone(),
        })
    }

    /// The kernel with `steps`, writing the output. `ctx` must be `self.gpu.context()`:
    /// the constants are written through the device's context and must land in order with
    /// the dispatches recorded on `ctx`.
    fn run_kernel(&self, ctx: &ID3D11DeviceContext, steps: u32) {
        debug_assert_eq!(
            ctx.as_raw(),
            self.gpu.context().as_raw(),
            "the submission queue must wrap the load's own device context"
        );
        self.gpu
            .set_constants(&self.constants, (self.params)(steps, self.seed));
        // SAFETY: `ctx` is this device's immediate context (asserted above), used only on
        // this thread; resources of this device; GROUPS groups of 256 threads fill the
        // THREADS elements; the UAV is unbound afterwards so the output can be read as an SRV.
        unsafe {
            ctx.CSSetShader(&self.kernel, None);
            ctx.CSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            ctx.CSSetUnorderedAccessViews(0, 1, Some(&Some(self.out_uav.clone())), None);
            ctx.Dispatch(GROUPS, 1, 1);
            ctx.CSSetUnorderedAccessViews(0, 1, Some(&None), None);
        }
    }

    fn words(bytes: &[u8]) -> Vec<u32> {
        bytes
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect()
    }
}

impl GpuWorkload for ComputeLoad {
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
        // The GPU against the CPU reference, on a short run (DG5).
        sub.submit(&mut |ctx| self.run_kernel(ctx, SAMPLE_STEPS))?;
        sub.finish()?;
        let out = Self::words(&self.gpu.read_buffer(&self.out, THREADS * 16)?);
        stopped()?;
        for (i, id) in (0..THREADS as usize).step_by(SAMPLE_EVERY).enumerate() {
            // ~0.2 s of CPU for the whole sample (S1): look at the stop every ~20 ms.
            if i % 1024 == 0 {
                stopped()?;
            }
            if out[id * 4..id * 4 + 4] != (self.reference)(id as u32, self.seed, SAMPLE_STEPS) {
                tracing::error!(thread = id, "GPU output differs from the CPU reference");
                return Err(GpuError::ReferenceInvalid);
            }
        }
        stopped()?;
        self.steps = calibrate(sub, target_ms, &mut |ctx, steps| {
            self.run_kernel(ctx, steps)
        })?;
        tracing::info!(steps = self.steps, target_ms, "compute load calibrated");
        stopped()?;
        // The golden output: the first submission with the calibrated steps.
        let steps = self.steps;
        sub.submit(&mut |ctx| {
            self.run_kernel(ctx, steps);
            // SAFETY: `ctx` is this device's context (asserted in `run_kernel`); two buffers
            // of this device with the same description.
            unsafe { ctx.CopyResource(&self.golden, &self.out) };
        })?;
        self.gpu.write_words(&self.counters, &COUNTERS_RESET);
        sub.finish()
    }

    fn submit(&mut self, sub: &mut dyn Submit) -> Result<(), GpuError> {
        self.submitted += 1;
        let (n, steps) = (self.submitted, self.steps);
        sub.submit(&mut |ctx| {
            self.run_kernel(ctx, steps);
            self.gpu
                .set_constants(&self.compare_constants, [THREADS, n as u32, 0, 0]);
            // SAFETY: `ctx` is this device's context (asserted in `run_kernel`), so the
            // compare constants written just above land before this dispatch; resources of
            // this device; the SRVs are unbound afterwards so the kernel can write the
            // output again.
            unsafe {
                ctx.CSSetShader(&self.compare, None);
                ctx.CSSetConstantBuffers(0, Some(&[Some(self.compare_constants.clone())]));
                ctx.CSSetShaderResources(
                    0,
                    Some(&[Some(self.out_srv.clone()), Some(self.golden_srv.clone())]),
                );
                ctx.CSSetUnorderedAccessViews(0, 1, Some(&Some(self.counters_uav.clone())), None);
                ctx.Dispatch(GROUPS, 1, 1);
                ctx.CSSetShaderResources(0, Some(&[None, None]));
                ctx.CSSetUnorderedAccessViews(0, 1, Some(&None), None);
            }
        })?;
        if self.inject && n == INJECT_AT {
            // Bit 0 of the golden's first word: every later submission differs (DA18).
            sub.finish()?;
            // Whole elements: D3D11 drops a partial copy of a structured element.
            let mut first = [0; 4];
            first.copy_from_slice(&Self::words(&self.gpu.read_buffer(&self.golden, 16)?));
            first[0] ^= 1;
            self.gpu.write_element(&self.golden, 0, first);
            tracing::warn!("fault injected into the golden output");
        }
        Ok(())
    }

    fn check(&mut self, sub: &mut dyn Submit) -> Result<GpuCheck, GpuError> {
        sub.finish()?;
        let c = Self::words(&self.gpu.read_buffer(&self.counters, 32)?);
        // Counter [0] would wrap after a few dozen seconds of mismatches.
        self.gpu.write_words(&self.counters, &COUNTERS_RESET);
        let checks = self.submitted - self.checked;
        self.checked = self.submitted;
        // The first mismatch found, which may not be the lowest index (`compare.hlsl`).
        let mismatches = if c[0] > 0 {
            vec![GpuMismatch {
                iteration: c[5].into(),
                expected: c[3].into(),
                actual: c[4].into(),
            }]
        } else {
            Vec::new()
        };
        Ok(GpuCheck {
            checks,
            mismatches,
            notices: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Inject;
    use crate::gpu::submit::Submitter;

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn compute_loads_pass_and_injected_fault_is_a_mismatch() {
        let adapter = oma_win::gpu::stress_adapters()
            .into_iter()
            .next()
            .expect("no hardware GPU");
        let gpu = GpuDevice::open(adapter.luid).unwrap();
        let mut sub = Submitter::new(&gpu).unwrap();
        for kernel in [KernelId::S1, KernelId::S2] {
            for inject in [false, true] {
                let ctx = PhaseCtx {
                    seed: 0x0123_4567_89AB_CDEF,
                    integrated: adapter.integrated,
                    inject: inject.then_some(Inject { kernel, core: None }),
                    budget: Default::default(),
                };
                let mut load = ComputeLoad::new(kernel, &gpu, &ctx).unwrap();
                // Short submissions: the test stays far below its GPU budget.
                load.prepare(&mut sub, 5.0, &AtomicBool::new(false))
                    .unwrap();
                for _ in 0..6 {
                    load.submit(&mut sub).unwrap();
                }
                let check = load.check(&mut sub).unwrap();
                assert_eq!(check.checks, 6);
                if inject {
                    let m = check.mismatches[0];
                    assert!(m.iteration > INJECT_AT, "{m:?}");
                    assert_eq!(m.expected ^ m.actual, 1, "{m:?}");
                } else {
                    assert_eq!(check.mismatches, [], "{kernel:?}");
                }
                // The counters were reset by the check.
                load.submit(&mut sub).unwrap();
                let again = load.check(&mut sub).unwrap();
                assert_eq!(again.checks, 1);
                assert_eq!(again.mismatches.is_empty(), !inject);
            }
        }
    }
}
