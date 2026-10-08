//! The disk side of the benchmark and the stress test (M8c): test files, aligned buffers and
//! the pure I/O helpers. The IOCP engine (C4) builds on these.

#[cfg(windows)]
pub mod buffer;
pub mod file;
pub mod hist;
pub mod offsets;
