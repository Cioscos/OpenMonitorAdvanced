//! The Performance view (M8a1, M8a2): stress tests and the CPU benchmark, run by `oma-load.exe`.

pub mod bench;
pub mod board;
pub mod commands;
pub mod host;
pub mod orphans;
pub mod runner;
pub mod store;

/// `%LOCALAPPDATA%\OpenMonitorAdvanced\performance`; the temporary folder without LOCALAPPDATA.
pub fn performance_dir() -> std::path::PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join("OpenMonitorAdvanced")
        .join("performance")
}
