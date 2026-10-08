//! The six benchmark workloads (DB2, DB3). Sizes are fixed (`DataSize::Fixed`), so every
//! CPU does the same work.

use oma_ipc::load::KernelId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchKernel {
    Ntt,
    Hash,
    Compress,
    Sort,
    Fft,
    Gemm,
    // GPU benchmark loads (`super::gpu::GPU_LOADS`).
    Fma,
    IntHash,
    Bandwidth,
    Fill,
    Texture,
    Overdraw,
    // Disk benchmark: the fill step and the tests (`super::disk`).
    DiskFill,
    Seq1mQ8t1,
    Seq1mQ1t1,
    Seq128kQ32t1,
    Rnd4kQ32t1,
    Rnd4kQ32t16,
    Rnd4kQ1t1,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Workload {
    pub id: BenchKernel,
    pub kernel: KernelId,
    /// `Mop/s`, `MB/s`, `Melem/s` or `GFLOP/s`.
    pub unit: &'static str,
    /// Raw work of one iteration (unit x 10^6 for the `M` units, x 10^9 for `G`).
    pub work_per_iteration: f64,
    /// Iterations per thread: about 1 s of one repetition on one thread (AVX-512, B3 timings).
    pub iterations: u64,
}

pub const WORKLOADS: [Workload; 6] = [
    Workload {
        id: BenchKernel::Ntt,
        kernel: KernelId::K5,
        unit: "Mop/s",
        // 2 NTTs x 16384 butterflies x 15 stages.
        work_per_iteration: 491_520.0,
        iterations: 900,
    },
    Workload {
        id: BenchKernel::Hash,
        kernel: KernelId::Hash,
        unit: "MB/s",
        // The 1 MiB buffer, counted once (SHA-256 and CRC32C both read it).
        work_per_iteration: 1_048_576.0,
        iterations: 1300,
    },
    Workload {
        id: BenchKernel::Compress,
        kernel: KernelId::Compress,
        unit: "MB/s",
        // 4 blocks x 256 KiB of input (compressed and decompressed).
        work_per_iteration: 1_048_576.0,
        iterations: 300,
    },
    Workload {
        id: BenchKernel::Sort,
        kernel: KernelId::Sort,
        unit: "Melem/s",
        // 262 144 u32 elements.
        work_per_iteration: 262_144.0,
        iterations: 90,
    },
    Workload {
        id: BenchKernel::Fft,
        kernel: KernelId::K2,
        unit: "GFLOP/s",
        // 2 transforms x 5 N log2(N) with N = 4096 (nominal convention): 2 x 5 x 4096 x 12.
        work_per_iteration: 491_520.0,
        iterations: 17_500,
    },
    Workload {
        id: BenchKernel::Gemm,
        kernel: KernelId::K7,
        unit: "GFLOP/s",
        // 2 n^3 with n = 256.
        work_per_iteration: 33_554_432.0,
        iterations: 640,
    },
];
