//! CPU benchmark: the six workloads, the fixed-work plan, the fixed-scale score, the
//! gauge full scale and the score file (pure; no Windows code).

mod file;
mod gauge;
mod plan;
mod score;
mod workloads;

pub use file::{
    parse_score, score_file_name, summary, Device, KernelRate, ScoreFile, ScoreSample,
    ScoreSummary, Scores, FORMAT,
};
pub use gauge::full_scale;
pub use plan::{bench_plan, BenchMode, BenchStep, CAP_S, WARMUP_PAUSE_MS};
pub use score::{
    cpu_baseline, median3, points, rate, scaling, Baseline, BaselineError, SCALE_POINTS,
    SCORE_VERSION,
};
pub use workloads::{BenchKernel, Workload, WORKLOADS};
