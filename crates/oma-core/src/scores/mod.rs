//! CPU and GPU benchmarks: the workloads, the plans, the fixed-scale scores, the gauge
//! full scale and the score file (pure; no Windows code).

mod bench;
mod file;
mod gauge;
mod gpu;
mod gpu_bench;
mod lifecycle;
mod plan;
mod score;
mod workloads;

pub use bench::{
    BenchAction, BenchContext, BenchController, BenchEnd, BenchState, BenchStatus, SegmentState,
};
pub use file::{
    parse_score, score_file_name, summary, Device, KernelRate, ScoreFile, ScoreSample,
    ScoreSummary, Scores, FORMAT,
};
pub use gauge::full_scale;
pub use gpu::{
    gpu_baseline, gpu_bench_plan, gpu_calibration_from, gpu_points, median_spread, GpuBaseline,
    GpuLoad, GPU_CAP_S, GPU_LOADS, GPU_SCORE_VERSION, GPU_WINDOWS,
};
pub use gpu_bench::{GpuBenchContext, GpuBenchController};
pub use plan::{bench_plan, BenchMode, BenchStep, CAP_S, WARMUP_PAUSE_MS};
pub use score::{
    calibration_from, cpu_baseline, median3, per_second_to_units, points, rate, scaling, Baseline,
    BaselineError, SCALE_POINTS, SCORE_VERSION,
};
pub use workloads::{BenchKernel, Workload, WORKLOADS};
