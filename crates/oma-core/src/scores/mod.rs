//! CPU and GPU benchmarks: the workloads, the plans, the fixed-scale scores, the gauge
//! full scale and the score file (pure; no Windows code).

mod bench;
mod board;
mod disk;
mod disk_bench;
mod file;
mod gauge;
mod gpu;
mod gpu_bench;
mod lifecycle;
mod plan;
mod score;
mod share;
mod workloads;

pub use bench::{
    BenchAction, BenchContext, BenchController, BenchEnd, BenchState, BenchStatus, SegmentState,
};
pub use board::{
    author_rows, known_version, median, normalize_model, parse_table, plausible,
    validate_submission, Board, ErrorCode, Model, Source, Submission, Table, TableRow,
    MAX_SUBMIT_BYTES, MAX_TABLE_BYTES, MODEL_MAX, PLAUSIBLE_MAX, PLAUSIBLE_MIN, SUBMIT_URL,
    TABLE_URL, VALUE_CAP,
};
pub use disk::{
    disk_baseline, disk_bench_plan, disk_calibration_from, disk_points, disk_tests, DiskBaseline,
    DiskProfile, DiskTest, B1_TESTS, B2_TESTS, DISK_BENCH_FILE, DISK_POINTS, DISK_SCORE_VERSION,
};
pub use disk_bench::{DiskBenchContext, DiskBenchController};
pub use file::{
    parse_score, score_file_name, summary, Device, DiskRate, KernelRate, ScoreFile, ScoreSample,
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
pub use share::{export_bytes, share_block, submission_bytes, HostFacts};
pub use workloads::{BenchKernel, Workload, WORKLOADS};
