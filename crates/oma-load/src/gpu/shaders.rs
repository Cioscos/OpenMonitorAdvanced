//! The shaders' bytecode, compiled by `build.rs` with `fxc.exe` (plan DG3).

macro_rules! cso {
    ($name:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/", $name, ".cso"))
    };
}

/// S1, FP32 FMA chains: constant buffer `reference::fma_params`, a `float4` per thread.
pub const S1_FMA: &[u8] = cso!("s1_fma");
/// S2, integer hash: constant buffer `reference::hash_params`, a `uint4` per thread.
pub const S2_HASH: &[u8] = cso!("s2_hash");
/// Output against golden: constant buffer `{ elements, submission, 0, 0 }`, SRVs `t0`
/// (output) and `t1` (golden), counters in `u0` (layout in `compare.hlsl`).
pub const COMPARE: &[u8] = cso!("compare");
/// S4, the VRAM check: 32-byte constant buffer, chunk in `u0` (raw), statistics in `u1`
/// (layout in `s4_vram.hlsl`).
pub const S4_VRAM: &[u8] = cso!("s4_vram");
/// S5 and S6, the scene's vertex and pixel shaders: constant buffer `{ frame, reads, 0, 0 }`,
/// texture `t0` and sampler `s0` (`scene.hlsl`).
pub const SCENE_VS: &[u8] = cso!("scene_vs");
pub const SCENE_PS: &[u8] = cso!("scene_ps");
/// S6, the tile hashes: constant buffer `{ mode, slot, width, height }`, render target
/// `t0`, reference hashes `u0`, per-submission results `u1` (modes in `tile_hash.hlsl`).
pub const TILE_HASH: &[u8] = cso!("tile_hash");
/// Writes [`PROBE_VALUE`] to word 0 of `u0`.
pub const PROBE: &[u8] = cso!("probe");

pub const PROBE_VALUE: u32 = 0x0A11_C0DE;
/// The counters of `compare.hlsl` as reset before a phase: no mismatch, lowest index `MAX`.
pub const COUNTERS_RESET: [u32; 8] = [0, u32::MAX, 0, 0, 0, 0, 0, 0];
