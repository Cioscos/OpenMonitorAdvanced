//! The GPU side of the stress test (M8b1): sizing, pacing and CPU references are portable;
//! the shaders, the D3D11 device and the submissions exist only on Windows.

#[cfg(windows)]
pub mod compute;
#[cfg(windows)]
pub mod device;
#[cfg(windows)]
pub mod engine;
#[cfg(windows)]
pub mod graphics;
pub mod pace;
pub mod reference;
#[cfg(windows)]
pub mod shaders;
pub mod sizing;
#[cfg(windows)]
pub mod stream;
#[cfg(windows)]
pub mod submit;
#[cfg(all(test, windows))]
mod tests;
#[cfg(windows)]
pub mod vram;
