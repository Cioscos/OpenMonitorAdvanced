//! The disk side of the benchmark and the stress test (M8c): test files, aligned buffers, the
//! pure I/O helpers and the phase engine (`engine`, portable, on the `IoQueue` trait) with
//! its IOCP queue (`iocp`, Windows).

#[cfg(windows)]
pub mod buffer;
pub mod engine;
pub mod file;
pub mod hist;
#[cfg(windows)]
pub mod iocp;
pub mod offsets;
#[cfg(test)]
mod tests;
