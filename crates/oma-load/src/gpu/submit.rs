//! Submissions to the GPU (plan DG4): at most 2 in flight, waited with an event query and
//! `sleep(1)`, never longer than 1 s; GPU time from timestamp queries.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Query, D3D11_QUERY,
    D3D11_QUERY_DATA_TIMESTAMP_DISJOINT, D3D11_QUERY_DESC, D3D11_QUERY_EVENT,
    D3D11_QUERY_TIMESTAMP, D3D11_QUERY_TIMESTAMP_DISJOINT,
};

use super::device::{gpu_error, removed_reason, GpuDevice, GpuError};

/// A submission that has not finished after this is hung.
const HUNG_AFTER: Duration = Duration::from_secs(1);
const IN_FLIGHT: usize = 2;
/// Tries of a timing whose timestamps were disjoint (a clock change on the GPU).
const TIMING_TRIES: usize = 3;

/// The submission queue of a GPU phase. Object safe, so the engine's tests can fake it.
pub trait Submit {
    /// Records `work` and sends it to the GPU; first waits for the oldest submission when
    /// 2 are in flight.
    fn submit(&mut self, work: &mut dyn FnMut(&ID3D11DeviceContext)) -> Result<(), GpuError>;
    /// Waits for every submission in flight.
    fn finish(&mut self) -> Result<(), GpuError>;
    /// Runs `work` alone and returns its GPU time in milliseconds; `TimingDisjoint` when
    /// the GPU clock kept changing.
    fn gpu_ms(&mut self, work: &mut dyn FnMut(&ID3D11DeviceContext)) -> Result<f64, GpuError>;
    /// Starts a measured window of a GPU benchmark phase (DH4): the submissions until
    /// [`Submit::window_end`] are timed together, without stopping the queue.
    fn window_begin(&mut self) -> Result<(), GpuError>;
    /// Ends the window, waits for the GPU and gives its milliseconds; `None` when the
    /// timestamps were disjoint.
    fn window_end(&mut self) -> Result<Option<f64>, GpuError>;
}

pub struct Submitter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    events: [ID3D11Query; IN_FLIGHT],
    /// Event slot and submission time of each submission in flight, oldest first.
    in_flight: VecDeque<(usize, Instant)>,
    next: usize,
    disjoint: ID3D11Query,
    start: ID3D11Query,
    end: ID3D11Query,
    /// The queries of the measured windows, apart from those of `gpu_ms`.
    window_disjoint: ID3D11Query,
    window_start: ID3D11Query,
    window_end: ID3D11Query,
}

impl Submitter {
    pub fn new(gpu: &GpuDevice) -> Result<Submitter, GpuError> {
        let device = gpu.device().clone();
        let query = |kind: D3D11_QUERY| -> Result<ID3D11Query, GpuError> {
            let mut query = None;
            let desc = D3D11_QUERY_DESC {
                Query: kind,
                MiscFlags: 0,
            };
            // SAFETY: a complete description and a live out pointer.
            unsafe { device.CreateQuery(&desc, Some(&mut query)) }
                .map_err(|e| gpu_error(&device, &e))?;
            query.ok_or(GpuError::Create(0x8000_4005_u32 as i32))
        };
        Ok(Submitter {
            events: [query(D3D11_QUERY_EVENT)?, query(D3D11_QUERY_EVENT)?],
            disjoint: query(D3D11_QUERY_TIMESTAMP_DISJOINT)?,
            start: query(D3D11_QUERY_TIMESTAMP)?,
            end: query(D3D11_QUERY_TIMESTAMP)?,
            window_disjoint: query(D3D11_QUERY_TIMESTAMP_DISJOINT)?,
            window_start: query(D3D11_QUERY_TIMESTAMP)?,
            window_end: query(D3D11_QUERY_TIMESTAMP)?,
            in_flight: VecDeque::with_capacity(IN_FLIGHT),
            next: 0,
            context: gpu.context().clone(),
            device,
        })
    }

    /// Polls `query` into `out` until it leaves `pending` (GetData gives S_FALSE, an `Ok`,
    /// and leaves `out` alone while the GPU is busy), sleeping 1 ms between polls.
    fn wait_for<T: PartialEq + Copy>(
        &self,
        query: &ID3D11Query,
        pending: T,
        since: Instant,
    ) -> Result<T, GpuError> {
        loop {
            let mut out = pending;
            // SAFETY: `out` is a live `T` of the size the query writes (BOOL, u64 or the
            // disjoint struct, matched by the callers).
            unsafe {
                self.context.GetData(
                    query,
                    Some(&mut out as *mut T as *mut c_void),
                    size_of::<T>() as u32,
                    0,
                )
            }
            .map_err(|e| gpu_error(&self.device, &e))?;
            if out != pending {
                return Ok(out);
            }
            if since.elapsed() > HUNG_AFTER {
                // A device that is gone also stops answering: tell the two apart.
                return Err(match removed_reason(&self.device) {
                    0 => GpuError::Hung,
                    reason => GpuError::Lost(reason),
                });
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Waits for the disjoint query and the two timestamps of a timing, then gives its
    /// milliseconds (`None` when disjoint).
    fn read_timing(
        &self,
        disjoint: &ID3D11Query,
        start: &ID3D11Query,
        end: &ID3D11Query,
    ) -> Result<Option<f64>, GpuError> {
        let since = Instant::now();
        let pending = D3D11_QUERY_DATA_TIMESTAMP_DISJOINT::default();
        let clock = self.wait_for(disjoint, pending, since)?;
        let start = self.wait_for(start, u64::MAX, since)?;
        let end = self.wait_for(end, u64::MAX, since)?;
        Ok(timing_ms(start, end, clock))
    }

    fn wait_oldest(&mut self) -> Result<(), GpuError> {
        if let Some(&(slot, since)) = self.in_flight.front() {
            self.wait_for(&self.events[slot], 0i32, since)?;
            self.in_flight.pop_front();
        }
        Ok(())
    }
}

impl Submit for Submitter {
    fn submit(&mut self, work: &mut dyn FnMut(&ID3D11DeviceContext)) -> Result<(), GpuError> {
        if self.in_flight.len() == IN_FLIGHT {
            self.wait_oldest()?;
        }
        work(&self.context);
        let slot = self.next;
        // SAFETY: an event query of this device; Flush has no preconditions.
        unsafe {
            self.context.End(&self.events[slot]);
            self.context.Flush();
        }
        self.in_flight.push_back((slot, Instant::now()));
        self.next = (slot + 1) % IN_FLIGHT;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), GpuError> {
        while !self.in_flight.is_empty() {
            self.wait_oldest()?;
        }
        Ok(())
    }

    fn gpu_ms(&mut self, work: &mut dyn FnMut(&ID3D11DeviceContext)) -> Result<f64, GpuError> {
        self.finish()?;
        for _ in 0..TIMING_TRIES {
            // SAFETY: queries of this device, in the order D3D11 asks: the disjoint query
            // brackets the two timestamps.
            unsafe {
                self.context.Begin(&self.disjoint);
                self.context.End(&self.start);
            }
            work(&self.context);
            // SAFETY: as above.
            unsafe {
                self.context.End(&self.end);
                self.context.End(&self.disjoint);
                self.context.Flush();
            }
            if let Some(ms) = self.read_timing(&self.disjoint, &self.start, &self.end)? {
                return Ok(ms);
            }
        }
        Err(GpuError::TimingDisjoint)
    }

    fn window_begin(&mut self) -> Result<(), GpuError> {
        // SAFETY: queries of this device; the window's disjoint query brackets its two
        // timestamps, and `gpu_ms` (with its own queries) never runs inside a window.
        unsafe {
            self.context.Begin(&self.window_disjoint);
            self.context.End(&self.window_start);
        }
        Ok(())
    }

    fn window_end(&mut self) -> Result<Option<f64>, GpuError> {
        // SAFETY: as in `window_begin`.
        unsafe {
            self.context.End(&self.window_end);
            self.context.End(&self.window_disjoint);
            self.context.Flush();
        }
        self.read_timing(&self.window_disjoint, &self.window_start, &self.window_end)
    }
}

/// Milliseconds between two GPU timestamps; `None` when the clock was disjoint (or gave
/// no frequency), since the difference then means nothing.
fn timing_ms(start: u64, end: u64, clock: D3D11_QUERY_DATA_TIMESTAMP_DISJOINT) -> Option<f64> {
    (!clock.Disjoint.as_bool() && clock.Frequency != 0)
        .then(|| end.saturating_sub(start) as f64 * 1e3 / clock.Frequency as f64)
}

/// The parameter of `run` (iterations, instances) that makes one submission take about
/// `target_ms` of GPU time: doubles from 64 until a submission takes more than a quarter
/// of the target, then scales in proportion (at most 4 times, for a GPU that stays faster
/// than the cap).
pub fn calibrate(
    sub: &mut dyn Submit,
    target_ms: f64,
    run: &mut dyn FnMut(&ID3D11DeviceContext, u32),
) -> Result<u32, GpuError> {
    const CAP: u32 = 1 << 24;
    let mut param = 64u32;
    loop {
        let ms = sub.gpu_ms(&mut |ctx| run(ctx, param))?;
        if ms > target_ms / 4.0 || param >= CAP {
            let scale = if ms > 0.0 {
                (target_ms / ms).min(4.0)
            } else {
                4.0
            };
            return Ok((f64::from(param) * scale).round().max(1.0) as u32);
        }
        param *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::reference::{fma_params, fma_thread, hash_params, hash_thread};
    use crate::gpu::shaders::{COMPARE, COUNTERS_RESET, PROBE, PROBE_VALUE, S1_FMA, S2_HASH};

    /// GPU time grows linearly with the parameter, `per_unit_ms` each; the parameter is
    /// found from the call count, since a fake cannot hand `run` a device context.
    struct Linear {
        per_unit_ms: f64,
        calls: u32,
    }

    impl Submit for Linear {
        fn submit(&mut self, _: &mut dyn FnMut(&ID3D11DeviceContext)) -> Result<(), GpuError> {
            Ok(())
        }
        fn finish(&mut self) -> Result<(), GpuError> {
            Ok(())
        }
        fn gpu_ms(&mut self, _: &mut dyn FnMut(&ID3D11DeviceContext)) -> Result<f64, GpuError> {
            let param = 64u64 << self.calls;
            self.calls += 1;
            Ok(param as f64 * self.per_unit_ms)
        }
        fn window_begin(&mut self) -> Result<(), GpuError> {
            Ok(())
        }
        fn window_end(&mut self) -> Result<Option<f64>, GpuError> {
            Ok(None)
        }
    }

    #[test]
    fn disjoint_timings_are_rejected() {
        let clock = |frequency, disjoint: bool| D3D11_QUERY_DATA_TIMESTAMP_DISJOINT {
            Frequency: frequency,
            Disjoint: disjoint.into(),
        };
        assert_eq!(
            timing_ms(1_000, 41_000, clock(1_000_000, false)),
            Some(40.0)
        );
        assert_eq!(timing_ms(1_000, 41_000, clock(1_000_000, true)), None);
        assert_eq!(timing_ms(1_000, 41_000, clock(0, false)), None);
    }

    #[test]
    fn calibrate_doubles_then_scales() {
        let mut fake = Linear {
            per_unit_ms: 0.001,
            calls: 0,
        };
        // 64, 128, ... 16384 (16.4 ms > 10 ms), scaled to 40 ms.
        assert_eq!(calibrate(&mut fake, 40.0, &mut |_, _| {}), Ok(40_000));
        assert_eq!(fake.calls, 9);
        // A GPU too fast for the cap: at most 4 times the cap.
        let mut fast = Linear {
            per_unit_ms: 1e-9,
            calls: 0,
        };
        assert_eq!(calibrate(&mut fast, 40.0, &mut |_, _| {}), Ok(4 << 24));
    }

    fn gpus() -> Vec<oma_win::gpu::StressAdapter> {
        let gpus = oma_win::gpu::stress_adapters();
        assert!(!gpus.is_empty(), "no hardware GPU");
        gpus
    }

    fn words(bytes: &[u8]) -> Vec<u32> {
        bytes
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
            .collect()
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn probe_round_trips() {
        for adapter in gpus() {
            let gpu = GpuDevice::open(adapter.luid).unwrap();
            let shader = gpu.compute_shader(PROBE).unwrap();
            let (buffer, uav, _) = gpu.structured_buffer(4, 4).unwrap();
            let mut sub = Submitter::new(&gpu).unwrap();
            sub.submit(&mut |ctx| {
                // SAFETY: a shader and a UAV of this device; one group.
                unsafe {
                    ctx.CSSetShader(&shader, None);
                    ctx.CSSetUnorderedAccessViews(0, 1, Some(&Some(uav.clone())), None);
                    ctx.Dispatch(1, 1, 1);
                }
            })
            .unwrap();
            sub.finish().unwrap();
            let read = words(&gpu.read_buffer(&buffer, 4).unwrap());
            assert_eq!(read, [PROBE_VALUE], "{}", adapter.name);
        }
    }

    const THREADS: u32 = 1 << 20;
    const STEPS: u32 = 1000;

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn gpu_matches_the_cpu_reference() {
        type Reference = fn(u32, u32, u32) -> [u32; 4];
        let kernels: [(&str, &[u8], [u32; 4], Reference); 2] = [
            ("s1", S1_FMA, fma_params(STEPS, 0x5A5A), fma_thread),
            ("s2", S2_HASH, hash_params(STEPS, 0x1234_5678), hash_thread),
        ];
        for adapter in gpus() {
            let gpu = GpuDevice::open(adapter.luid).unwrap();
            let mut sub = Submitter::new(&gpu).unwrap();
            for (name, bytecode, params, reference) in kernels {
                let shader = gpu.compute_shader(bytecode).unwrap();
                let constants = gpu.constant_buffer().unwrap();
                gpu.set_constants(&constants, params);
                let (buffer, uav, _) = gpu.structured_buffer(THREADS * 16, 16).unwrap();
                let mut dispatch = |ctx: &ID3D11DeviceContext| {
                    // SAFETY: resources of this device; THREADS / 256 groups of 256 threads
                    // fill the THREADS elements.
                    unsafe {
                        ctx.CSSetShader(&shader, None);
                        ctx.CSSetConstantBuffers(0, Some(&[Some(constants.clone())]));
                        ctx.CSSetUnorderedAccessViews(0, 1, Some(&Some(uav.clone())), None);
                        ctx.Dispatch(THREADS / 256, 1, 1);
                    }
                };
                sub.submit(&mut dispatch).unwrap();
                sub.finish().unwrap();
                // The timing runs the same work again, so the output stays the same.
                let ms = sub.gpu_ms(&mut dispatch).unwrap();
                assert!(ms > 0.0 && ms < 1000.0, "{name} took {ms} ms");
                let out = words(&gpu.read_buffer(&buffer, THREADS * 16).unwrap());
                for id in (0..THREADS).step_by(97) {
                    let at = id as usize * 4;
                    assert_eq!(
                        out[at..at + 4],
                        reference(id, params[1], STEPS),
                        "{name} thread {id} on {}",
                        adapter.name
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn compare_counts_differences() {
        for adapter in gpus() {
            let gpu = GpuDevice::open(adapter.luid).unwrap();
            let mut sub = Submitter::new(&gpu).unwrap();
            let s1 = gpu.compute_shader(S1_FMA).unwrap();
            let compare = gpu.compute_shader(COMPARE).unwrap();
            let s1_constants = gpu.constant_buffer().unwrap();
            let compare_constants = gpu.constant_buffer().unwrap();
            gpu.set_constants(&compare_constants, [THREADS, 7, 0, 0]);
            let (out, out_uav, out_srv) = gpu.structured_buffer(THREADS * 16, 16).unwrap();
            let (golden, _, golden_srv) = gpu.structured_buffer(THREADS * 16, 16).unwrap();
            let (counters, counters_uav, _) = gpu.structured_buffer(32, 4).unwrap();
            let run_s1 = |sub: &mut Submitter, seed: u32| {
                gpu.set_constants(&s1_constants, fma_params(10, seed));
                sub.submit(&mut |ctx| {
                    // SAFETY: resources of this device; the UAV is unbound afterwards so
                    // the compare pass can read the buffer as an SRV.
                    unsafe {
                        ctx.CSSetShader(&s1, None);
                        ctx.CSSetConstantBuffers(0, Some(&[Some(s1_constants.clone())]));
                        ctx.CSSetUnorderedAccessViews(0, 1, Some(&Some(out_uav.clone())), None);
                        ctx.Dispatch(THREADS / 256, 1, 1);
                        ctx.CSSetUnorderedAccessViews(0, 1, Some(&None), None);
                    }
                })
                .unwrap();
            };
            let run_compare = |sub: &mut Submitter| {
                sub.submit(&mut |ctx| {
                    // SAFETY: resources of this device; the SRVs are unbound afterwards so
                    // S1 can write the output again.
                    unsafe {
                        ctx.CSSetShader(&compare, None);
                        ctx.CSSetConstantBuffers(0, Some(&[Some(compare_constants.clone())]));
                        ctx.CSSetShaderResources(
                            0,
                            Some(&[Some(out_srv.clone()), Some(golden_srv.clone())]),
                        );
                        ctx.CSSetUnorderedAccessViews(
                            0,
                            1,
                            Some(&Some(counters_uav.clone())),
                            None,
                        );
                        ctx.Dispatch(THREADS / 256, 1, 1);
                        ctx.CSSetShaderResources(0, Some(&[None, None]));
                    }
                })
                .unwrap();
            };
            gpu.write_words(&counters, &COUNTERS_RESET);
            run_s1(&mut sub, 0x5A5A);
            // SAFETY: two buffers of this device with the same description.
            unsafe { gpu.context().CopyResource(&golden, &out) };
            run_s1(&mut sub, 0x5A5A);
            run_compare(&mut sub);
            sub.finish().unwrap();
            assert_eq!(
                words(&gpu.read_buffer(&counters, 32).unwrap()),
                COUNTERS_RESET
            );

            // Another seed changes the low byte of the base, so every word differs.
            run_s1(&mut sub, 0x5A5B);
            run_compare(&mut sub);
            sub.finish().unwrap();
            let c = words(&gpu.read_buffer(&counters, 32).unwrap());
            assert_eq!(c[0], THREADS * 4, "{}", adapter.name);
            assert_eq!(c[1], 0);
            let golden_words = words(&gpu.read_buffer(&golden, THREADS * 16).unwrap());
            let out_words = words(&gpu.read_buffer(&out, THREADS * 16).unwrap());
            let first = c[2] as usize;
            assert_eq!(
                [c[3], c[4], c[5]],
                [golden_words[first], out_words[first], 7]
            );
        }
    }
}
