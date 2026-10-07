/******************************************************************************
 * FIRESTARTER - A Processor Stress Test Utility
 * Copyright (C) 2020-2023 TU Dresden, Center for Information Services and High
 * Performance Computing
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/\>.
 *
 * Contact: daniel.hackenberg@tu-dresden.de
 *****************************************************************************/

// Adapted from FIRESTARTER (https://github.com/tud-zih-energy/FIRESTARTER), commit 927ae17e55f3f90f7575f6a68630a366fde9c94e, src/firestarter/X86/Payload/FMAPayload.cpp: the instruction groups translated line by line (registers, rotation of the destinations, displacements, L1 wrap), for AVX2+FMA and for SSE2.
// Adapted from FIRESTARTER (https://github.com/tud-zih-energy/FIRESTARTER), commit 927ae17e55f3f90f7575f6a68630a366fde9c94e, src/firestarter/X86/Payload/AVX512Payload.cpp: the same for AVX-512.
// Modified for OpenMonitor Advanced: Rust intrinsics generated at build time instead of asmjit at run time; accumulators reset every block; L3/RAM items dropped.
// The modifications are part of OpenMonitor Advanced, GPL-3.0-or-later.

//! Build script of `oma-load`:
//! - writes `$OUT_DIR/k1_payload.rs`, the unrolled passes of K1 (A9, DA8), so no code is
//!   generated at run time;
//! - embeds the version resource of `oma-load.exe` (plan DP8), so the file properties show
//!   the product and the `X.Y.Z` version like the app's;
//! - compiles the HLSL shaders of the GPU stress test with `fxc.exe` into `$OUT_DIR/*.cso`
//!   (M8b1, DG3), so every build carries the same bytecode and nothing needs
//!   `d3dcompiler_47.dll` at run time;
//! - exports `OMA_SHADER_DIGEST`, the FNV-1a 64 of the benchmark shaders' bytecode (M8b2,
//!   DH8), since the bytecode depends on the version of `fxc.exe`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

include!("src/kernels/k1/groups.rs");

fn main() {
    // The shaders' `rerun-if-changed` lines replace Cargo's default (any file of the
    // package), so the inputs of the K1 payload are listed too.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/kernels/k1/groups.rs");
    write_k1_payload();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    compile_shaders();
    let version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    let mut res = tauri_winres::WindowsResource::new();
    res.set("ProductName", "OpenMonitor Advanced")
        .set("FileDescription", "OpenMonitor Advanced load generator")
        .set("FileVersion", &version)
        .set("ProductVersion", &version)
        .set("OriginalFilename", "oma-load.exe")
        .set("InternalName", "oma-load");
    res.compile().expect("compile the version resource");
}

/// The GPU shaders: output name (`$OUT_DIR/<name>.cso`), file in `shaders/` (without
/// `.hlsl`), entry point and profile.
const SHADERS: [(&str, &str, &str, &str); 11] = [
    ("s1_fma", "s1_fma", "main", "cs_5_0"),
    ("s2_hash", "s2_hash", "main", "cs_5_0"),
    ("s3_stream", "s3_stream", "main", "cs_5_0"),
    ("compare", "compare", "main", "cs_5_0"),
    ("probe", "probe", "main", "cs_5_0"),
    ("s4_vram", "s4_vram", "main", "cs_5_0"),
    ("scene_vs", "scene", "vs", "vs_5_0"),
    ("scene_ps", "scene", "ps", "ps_5_0"),
    ("tile_hash", "tile_hash", "main", "cs_5_0"),
    ("bench_gfx_vs", "bench_gfx", "vs", "vs_5_0"),
    ("bench_gfx_ps", "bench_gfx", "ps", "ps_5_0"),
];

/// The shaders of the GPU benchmark, in the order of the digest (DH8): `bench_gfx` is its
/// vertex then its pixel shader.
const DIGEST_SHADERS: &[&str] = &[
    "s1_fma",
    "s2_hash",
    "compare",
    "s3_stream",
    "bench_gfx_vs",
    "bench_gfx_ps",
];

/// FNV-1a, 64 bits.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn compile_shaders() {
    println!("cargo:rerun-if-env-changed=OMA_FXC");
    let fxc = find_fxc().unwrap_or_else(|| {
        panic!("fxc.exe not found: install the Windows 10/11 SDK or set OMA_FXC")
    });
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    for (name, file, entry, profile) in SHADERS {
        let src = format!("shaders/{file}.hlsl");
        println!("cargo:rerun-if-changed={src}");
        // /O3 and /Gis (IEEE strictness, so the mad chains are never reassociated) give the
        // same bytecode as D3DCompile with OPTIMIZATION_LEVEL3 | IEEE_STRICTNESS (spike).
        let result = Command::new(&fxc)
            .args(["/nologo", "/O3", "/Gis", "/T", profile, "/E", entry, "/Fo"])
            .arg(out.join(format!("{name}.cso")))
            .arg(&src)
            .output()
            .unwrap_or_else(|e| panic!("run {}: {e}", fxc.display()));
        if !result.status.success() {
            panic!(
                "fxc failed on {src}:\n{}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
    // The published test vectors of FNV-1a 64.
    assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    let mut bytecode = Vec::new();
    for name in DIGEST_SHADERS {
        let cso = out.join(format!("{name}.cso"));
        bytecode.extend(std::fs::read(&cso).unwrap_or_else(|e| panic!("read {cso:?}: {e}")));
    }
    println!(
        "cargo:rustc-env=OMA_SHADER_DIGEST={:016x}",
        fnv1a64(&bytecode)
    );
}

/// `OMA_FXC`, else the highest `Windows Kits\10\bin\10.*\x64\fxc.exe`.
fn find_fxc() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OMA_FXC") {
        return Some(PathBuf::from(path));
    }
    let bin = Path::new(&std::env::var_os("ProgramFiles(x86)")?).join(r"Windows Kits\10\bin");
    std::fs::read_dir(bin)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            let version: Vec<u32> = name
                .strip_prefix("10.")?
                .split('.')
                .map(|part| part.parse().ok())
                .collect::<Option<_>>()?;
            let fxc = entry.path().join(r"x64\fxc.exe");
            fxc.is_file().then_some((version, fxc))
        })
        .max()
        .map(|(_, fxc)| fxc)
}

/// One instruction set of the generated passes.
struct Target {
    name: &'static str,
    feature: &'static str,
    groups: &'static str,
    /// Intrinsic prefix of the vector width: `_mm512`, `_mm256` or `_mm`.
    p: &'static str,
    /// `vfmadd231pd`; without it (SSE2) `mulpd` then `addpd`.
    fma: bool,
    /// FIRESTARTER's displacements in f64 words, see [`Disp`].
    disp: Disp,
}

/// Displacements from the line pointer, in f64 words, as in the payloads.
struct Disp {
    l1_l: usize,
    l1_ls_store: usize,
    l1_ls_load: usize,
    l2_l: usize,
    l2_s: usize,
    l2_ls_store: usize,
    l2_ls_load: usize,
}

/// AVX512Payload.cpp: whole `zmm` loads and stores.
const AVX512_DISP: Disp = Disp {
    l1_l: 8,
    l1_ls_store: 8,
    l1_ls_load: 16,
    l2_l: 8,
    l2_s: 8,
    l2_ls_store: 8,
    l2_ls_load: 16,
};

/// FMAPayload.cpp: `ymm` loads, stores of the low `xmm` half. SSE2 keeps them.
const FMA_DISP: Disp = Disp {
    l1_l: 4,
    l1_ls_store: 8,
    l1_ls_load: 4,
    l2_l: 8,
    l2_s: 8,
    l2_ls_store: 12,
    l2_ls_load: 8,
};

/// The accumulators `x0`..`x6` take the lines in turn (FIRESTARTER's add registers),
/// `x7`..`x9` the second FMA of `REG` (its alternate destinations).
const ADD_END: usize = 6;
const TRANS_START: usize = 7;
const TRANS_END: usize = 9;

fn write_k1_payload() {
    let targets = [
        Target {
            name: "avx512",
            feature: "avx512f",
            groups: AVX512_GROUPS,
            p: "_mm512",
            fma: true,
            disp: AVX512_DISP,
        },
        Target {
            name: "avx2",
            feature: "avx2,fma",
            groups: AVX2_GROUPS,
            p: "_mm256",
            fma: true,
            disp: FMA_DISP,
        },
        Target {
            name: "sse2",
            feature: "sse2",
            groups: AVX2_GROUPS,
            p: "_mm",
            fma: false,
            disp: FMA_DISP,
        },
    ];
    let mut out = String::from("// Generated by crates/oma-load/build.rs from groups.rs.\n");
    for t in &targets {
        out += &pass(t);
    }
    let dir = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    std::fs::write(std::path::Path::new(&dir).join("k1_payload.rs"), out)
        .expect("write k1_payload.rs");
}

/// The function of one pass and its count of L2 lines.
fn pass(t: &Target) -> String {
    let lines = unroll(t.groups).unwrap_or_else(|e| panic!("{}: {e}", t.name));
    let p = t.p;
    let fma = |acc: &str, x: &str, y: &str| {
        if t.fma {
            format!("{acc} = {p}_fmadd_pd({x}, {y}, {acc});")
        } else {
            format!("{acc} = _mm_add_pd({acc}, _mm_mul_pd({x}, {y}));")
        }
    };
    let load = |ptr: &str| format!("{p}_loadu_pd({ptr})");
    let store = |ptr: &str, v: &str| {
        if p == "_mm256" {
            // FMAPayload stores the low 128 bits (`vmovapd xmmword_ptr`).
            format!("_mm_storeu_pd({ptr}, _mm256_castpd256_pd128({v}));")
        } else {
            format!("{p}_storeu_pd({ptr}, {v});")
        }
    };
    let last_l1 = lines.iter().rposition(|l| l.starts_with("L1"));
    let d = &t.disp;

    let mut body = String::new();
    let (mut add, mut mov, mut l2) = (1, TRANS_START, 0);
    let mut prev: Vec<String> = Vec::new();
    for (i, &item) in lines.iter().enumerate() {
        let x = format!("x{add}");
        let l1_at = |disp: usize| format!("l1.add(o1 + {disp})");
        let l2_at = |disp: usize| format!("l2.add({})", l2 * 8 + disp);
        // Without FMA, LLVM computes the loop-invariant `a * c` and `c * b` once, and its
        // scheduler mixes the lines until the 16 xmm registers spill. An asm with no
        // instruction (the operands appear only in an assembler comment) takes `c`, the
        // accumulators of the line before and those of this line, and may touch memory, so
        // each line keeps its own `mulpd` and the lines, loads and stores stay in
        // FIRESTARTER's order.
        let mut regs = vec![x.clone()];
        if item == "REG" {
            regs.push(format!("x{mov}"));
        }
        let barrier = if t.fma {
            String::new()
        } else {
            let c_dir = if i + 1 == lines.len() { "in" } else { "inout" };
            let mut operands = vec![format!("{c_dir}(xmm_reg) c")];
            for r in prev.iter().chain(&regs) {
                let op = format!("inout(xmm_reg) {r}");
                if !operands.contains(&op) {
                    operands.push(op);
                }
            }
            let comment: Vec<String> = (0..operands.len()).map(|n| format!("{{{n}}}")).collect();
            format!(
                "core::arch::asm!(\"/* {} */\", {}, options(nostack, preserves_flags)); ",
                comment.join(" "),
                operands.join(", ")
            )
        };
        prev = regs;
        let code = match item {
            "REG" => {
                let code = format!(
                    "{} {}",
                    fma(&x, "a", "c"),
                    fma(&format!("x{mov}"), "c", "b")
                );
                mov += 1;
                code
            }
            "L1_L" => format!(
                "{} {}",
                fma(&x, "a", "c"),
                fma(&x, "b", &load(&l1_at(d.l1_l)))
            ),
            "L1_LS" => format!(
                "{} {}",
                store(&l1_at(d.l1_ls_store), &x),
                fma(&x, "a", &load(&l1_at(d.l1_ls_load)))
            ),
            "L2_L" => format!(
                "{} {}",
                fma(&x, "a", "c"),
                fma(&x, "b", &load(&l2_at(d.l2_l)))
            ),
            "L2_S" => format!("{} {}", store(&l2_at(d.l2_s), &x), fma(&x, "a", "c")),
            "L2_LS" => format!(
                "{} {}",
                store(&l2_at(d.l2_ls_store), &x),
                fma(&x, "a", &load(&l2_at(d.l2_ls_load)))
            ),
            other => unreachable!("parse_groups refuses {other}"),
        };
        let code = format!("{barrier}{code}");
        let _ = writeln!(body, "        // {i}: {item}\n        {code}");
        if item.starts_with("L1") && Some(i) != last_l1 {
            // FIRESTARTER advances one cache line and wraps at the end of the zone.
            body += "        o1 = (o1 + 8) & m1;\n";
        }
        if item.starts_with("L2") {
            l2 += 1;
        }
        add = if add == ADD_END { 0 } else { add + 1 };
        if mov > TRANS_END {
            mov = TRANS_START;
        }
    }

    let name = t.name;
    let upper = name.to_uppercase();
    let mut f = String::new();
    let _ = writeln!(
        f,
        "
/// L2 lines one {name} pass touches, one after the other.
const L2_PER_PASS_{upper}: usize = {l2};

/// One pass of the {name} sequence, {n} lines.
///
/// # Safety
/// The CPU has `{feature}`. `l1` points to `st.l1_mask + 1` words plus `K1_PAD`, `l2` to
/// `L2_PER_PASS_{upper} * 8` words plus `K1_PAD`.
#[target_feature(enable = \"{feature}\")]
#[inline(never)]
unsafe fn k1_block_{name}(st: &mut K1State, l1: *mut f64, l2: *mut f64) {{
    // SAFETY: the caller guarantees the feature and the sizes; the L1 offsets are masked
    // to the zone and the L2 offsets stay below the bound, both plus at most 24 words. An
    // empty asm (SSE2) has no instruction: it touches no memory or stack and leaves every
    // register it names as it was.
    unsafe {{
        let a = {load_a};
        let b = {load_b};
        let {mut_c}c = {load_c};",
        n = lines.len(),
        feature = t.feature,
        load_a = load("st.a.as_ptr()"),
        load_b = load("st.b.as_ptr()"),
        load_c = load("st.c.as_ptr()"),
        mut_c = if t.fma { "" } else { "mut " },
    );
    for i in 0..=TRANS_END {
        let _ = writeln!(
            f,
            "        let mut x{i} = {};",
            load(&format!("st.acc[{i}].as_ptr()"))
        );
    }
    if last_l1.is_some() {
        f += "        let m1 = st.l1_mask;\n        let mut o1: usize = 0;\n";
    }
    f += &body;
    for i in 0..=TRANS_END {
        // The whole register goes back to the state.
        let _ = writeln!(f, "        {p}_storeu_pd(st.acc[{i}].as_mut_ptr(), x{i});");
    }
    f += "    }\n}\n";
    f
}
