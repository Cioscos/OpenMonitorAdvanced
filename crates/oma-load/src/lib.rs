//! `oma-load`: the load generator the app starts for a stress test (M8a1).
//! The library is portable; everything specific to Windows is under `sys`,
//! `link` and the D3D11 half of `gpu`.

pub mod args;
pub mod engine;
pub mod gpu;
pub mod kernel;
#[cfg(target_arch = "x86_64")]
pub mod kernels;
pub mod link;
pub mod log;
pub mod rng;
pub mod sys;
pub mod verify;
