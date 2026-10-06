# GPU stress test and GPU benchmark: how existing tools work, how they detect errors, and what we can build

Research date: 2026-10-06. Target: OpenMonitor Advanced (Rust + D3D11/DXGI hand-written bindings, no admin, vendor-neutral, GPL-3.0-or-later).

Conventions used below:
- **[V]** = verified in a primary source I fetched (URL given).
- **[S]** = from a search-result summary only, not opened in full (treat as probable).
- **[I]** = my inference / engineering judgement, not stated by a source. Check before relying on it.

---

## 1. Executive summary

1. **FurMark-style loads are power viruses.** A single, extremely ALU/ROP-dense, fully independent shader loop pins the GPU at its power limit instantly. Vendors react with power limiters (NVIDIA GTX 580 era: hardware current sensing plus a driver that recognised the executable by name, FurMark and OCCT only). Result: clocks drop and the test no longer exercises the boost V/F curve where overclock instability actually lives. OCCT's answer was 3D Adaptive (variable load, switching load). [V geeks3d.com/?p=7516, V ocbase news]
2. **Instability often appears at partial load, not at 100 %.** OCCT states its Adaptive test "hits instability zones between 40 % and 60 % intensity", because at 100 % the GPU throttles clocks/voltage down. So an OC stability test needs a load *sweep* and a *transient/switch* pattern, not just a flat maximum. [V ocbase news]
3. **Error detection is a separate concern from load generation.** The only tools that publish how they detect errors are the compute ones: gpu-burn (many redundant GEMMs compared to each other), memtest_vulkan (address-derived pattern, write once, re-read forever, rotated read order, bit-level error statistics), MemtestG80/CL and NVIDIA DCGM memtest (Memtest86-style pattern suite). OCCT and FurMark/Kombustor only say they "detect artifacts"; the algorithm is not disclosed. [V]
4. **D3D11 compute alone is enough for a credible first version** of: ALU burn, integer-hash verify, VRAM verify, bandwidth benchmark, FP32/INT32 throughput benchmark, and an offscreen graphics load with GPU-side checksum. D3D12 is needed for: native FP16/INT64/wave ops, async compute + copy queues running concurrently with 3D (a realistic "mixed" load), explicit residency control, and any matrix/tensor workload (SM 6.9 long vectors / cooperative vectors, DirectML).
5. **Windows facts that shape the design:** TDR default is 2 s (a dispatch longer than that kills the device); D3D11 guarantees a single resource only up to `min(max(128 MB, 25 % VRAM), 2048 MB)`; VRAM must be sized from `IDXGIAdapter3::QueryVideoMemoryInfo` budget minus a margin; `DXGI_ERROR_DEVICE_REMOVED/HUNG/RESET` plus `GetDeviceRemovedReason` is the crash signal; everything runs headless (no window) on D3D11/D3D12 compute.
6. **GDDR6/GDDR6X overclock errors are often silent**: the link CRC (EDC) retries transfers, so you see *performance loss*, not wrong data. A VRAM OC test must therefore also watch bandwidth regression, not only data mismatches. [V for the mechanism, S for the GDDR6X specifics: see section 4.4]
7. Licences: all the tools worth studying for *algorithms* are permissive or LGPL/GPL (memtest_vulkan Zlib, gpu-burn BSD-2, vkpeak MIT, clpeak GPL-3.0, MemtestCL LGPL). FurMark, OCCT, Kombustor, 3DMark, Unigine, Geekbench, PassMark and GFXBench are proprietary, so we only take publicly documented *ideas* from them.

---

## 2. Tool-by-tool survey

### 2.1 FurMark 1 / FurMark 2 (Geeks3D)
- **What it does.** The "furry donut": a torus covered in fur, drawn with a multi-pass (reported as 100 passes/layers) GLSL vertex/pixel shader that makes the hairs finer layer by layer, with two dynamic lights doing self-shadowing. Very pixel-shader and ROP heavy, no vsync, fully GPU-bound. FurMark 2 is built on the GeeXLab engine, supports OpenGL and Vulkan, Windows + Linux (x86-64, arm64), has presets at 1080p/1440p/4K, a "Burn-in" stress mode, CLI (`/burnin /duration /log_gpu_data /nogui /vulkan`), and online scores. [V furmark.software/features.html; S for the "100 passes" detail via search summaries of furmark.software / majorgeeks / malwaretips]
- **Why it is a "power virus".** The workload has no stalls: high occupancy, almost no memory latency exposure, highly regular ALU+ROP work, so average switching activity (and power) is far above any game. It "drives VRMs and capacitors to their rated maximum" by its own marketing. [V furmark.software/features.html]
- **Why vendors throttle it.** NVIDIA's GTX 580 (2010) had per-12V-rail current/voltage sensing chips plus a driver that applied the limiter only when it recognised specific stress apps (FurMark 1.8+ and OCCT). With the limiter, FurMark drew ~153 W instead of ~304 W and frame rate fell about 50 %. NVIDIA's argument: the load is "unrealistic" and can harm hardware and PSUs. [V geeks3d.com/?p=7516]. AMD did the equivalent at driver level in the same era [S, not verified in this research].
- **Error reporting.** FurMark 2 includes an "artifact scanner" that looks for "visual anomalies like sparkles, texture corruption, or colour banding". Mechanism not documented; Kombustor (MSI's FurMark-based tool: OpenGL + Vulkan tests + artifact scanner) is the same family. [V furmark.software/features.html; V geeks3d.com/furmark/kombustor (landing page only)]
- **Licence.** Closed-source freeware. Nothing to copy; the concept (fur shells) is just heavy overdraw + procedural ALU and is easy to reimplement.
- **Lesson for us.** Offer a heavy graphics load but (a) do not make it the only test, (b) do not rely on flat 100 %, (c) be aware that drivers may power-limit "known" stress apps by process name, so our load process should have its own neutral name.

### 2.2 OCCT (OCBASE)
- **GPU tests.** 3D Standard (custom 3D engine, historically DirectX, "extreme levels of load, making most cards throttle"); 3D Adaptive (Unreal Engine based; runs at a fixed load, or *Variable* (load steps, e.g. 5 % increments every 20 s in the article's example; other guides say every 5 min), or *Switch* (rapid spikes between intensities to emulate alt-tab/menu/game transitions, "heavy transient loads, spiking endlessly")); VRAM test; Power test (CPU+GPU combined). OCCT 14.2 dropped DirectX for Vulkan by default (unifies adapter detection across Windows/Linux); its VRAM tests "rely on OpenCL". [V ocbase.com/news/occt-gpu-stress-testing-modern-adaptive-approach; V ocbase.com/news/occt-v14-vulkan; S ocbase.com/occt, rtech.support guide]
- **Why variable load.** At 100 % load the GPU downclocks, so "a stress test pushing your GPU to 100 % might display no error, but you could encounter an error at 50 %". The wider range of voltage/clock points exercised by a ramp is the point. Switch mode also stresses PSU/VRM transient response and doubles as a coil-whine detector. [V ocbase news]
- **Error detection.** 3D tests: "any artefact generated by the card will be picked up" automatically (algorithm undisclosed; [I] very likely a deterministic scene with frame/readback comparison). VRAM test: similar in principle to memtest_vulkan, lets you choose how much VRAM to test; recommended 95 % on dGPU, 90 % on iGPU/shared memory. OCCT itself says the VRAM test cannot detect transfer (link) errors that the hardware retries silently. [S rtech.support, profesionalreview.com, ocbase.com]
- **Licence.** Proprietary freeware (free Personal edition, paid Pro). Nothing to copy.
- **Lessons.** Variable and Switch modes are the two most valuable ideas for an OC stability test; implement both with a *verification workload running underneath*.

### 2.3 3DMark stress tests (UL)
- **What it does.** Loops an existing benchmark scene (Sky Diver, Fire Strike family, Time Spy Extreme, Steel Nomad, Speed Way (ray tracing), etc.) 20 times (~10 min) in Advanced Edition; Professional allows 2 to 5000 loops. No loading screens between loops. [V benchmarks.ul.com/news/check-your-pcs-stability-with-new-3dmark-stress-tests; S for newer test names]
- **Metric.** `Frame Rate Stability = fpsLow / fpsHigh * 100`, where fpsHigh is the average FPS of the best loop and fpsLow of the worst loop. **Pass = >= 97 % and all loops complete without interruption.** A result screen charts temps, clocks and FPS. Failure typically means thermal throttling or flaky hardware. [V support.benchmarks.ul.com/support/solutions/articles/44002134931-stress-test-result-screen]
- **Error detection.** None for wrong results. A crash, hang or device loss fails the run; otherwise only the performance consistency is judged.
- **Licence.** Proprietary.
- **Lesson.** The 97 % loop-stability rule is a cheap, well-understood verdict for the "normal" stress test: compare per-loop throughput of our fixed-size loops. It is *not* an error detector, so the OC test needs the verification layer on top.

### 2.4 Unigine Heaven / Valley / Superposition
- Superposition (2017, Unigine) has a stress-test mode that loops the benchmark for a chosen time, with selectable resolution (incl. 8K), texture quality, API and window mode; reports min/avg/max FPS, min/max GPU temperature, max GPU usage. Heavy tessellation and PBR scenes, i.e. a "game-like" sustained load. No documented artifact detector; stability is judged by crash/visual inspection. [S legitreviews, tomshardware, gamingonlinux]
- Licence: proprietary (free basic edition). Nothing to copy.

### 2.5 gpu-burn (wilicc, CUDA)
- **Licence: BSD-2-Clause** (confirmed by GitHub API). [V github.com/wilicc/gpu-burn]
- **Workload.** Allocates 90 % of free VRAM (`USEMEM 0.9`; `-m` for MB or %). Fills two 8192x8192 matrices A and B with random data; the rest of the memory holds `d_iters` result matrices C_i. Loops cuBLAS `Sgemm` (or `Dgemm` with `-d`; tensor cores with `-tc`) writing A*B into every C_i. [V gpu_burn-drv.cpp]
- **Error detection.** After each batch a small `compare` kernel checks that all `C_i` equal `C_0`: `fabsf(C[idx] - C[idx + i*step]) > EPSILON (0.001f)` increments a faulty-element counter (`atomicAdd`). The source comment says "no rounding errors due to results being accumulated in an arbitrary order, therefore EPSILON = 0 is OK" (same inputs, same kernel, same GPU). So the oracle is **self-consistency across redundant executions**, not a CPU golden result. Output: per-GPU passes/errors/temps. [V compare.cu, gpu_burn-drv.cpp]
- **Strengths/weaknesses.** Simple and effective at catching intermittent math-unit faults; blind to a *systematic* error that is identical in every copy; depends on vendor BLAS (we would write our own kernel).
- **Port to us.** The same idea works with an own FMA/GEMM compute shader; use integer-exact data so results are also vendor-independent (section 5.2).

### 2.6 memtest_vulkan (GpuZelenograd)
- **Licence: Zlib** (GitHub API: spdx `Zlib`; README: same licence as its `erupt` dependency). Rust + Vulkan compute (WGSL shaders compiled to SPIR-V at build time), cross-platform, **no admin needed on Windows**, inspired by MemtestCL. [V github.com/GpuZelenograd/memtest_vulkan, README, `src/main.rs` read in full for the shaders]
- **Algorithm (from the source).**
  - Buffer is `array<vec4<u32>>`. Expected value at index `i`: take four consecutive address-derived words `a = i*4 + calc_param + {1,2,3,4}`, rotate each left by `a % 31`. Every cell therefore holds a value that depends on its own address and on a per-iteration `calc_param` (`iter * 0x100107`).
  - `write` kernel: writes the expected values in a *reversed/mirrored* index order inside each window (so write order differs from read order).
  - `read` kernel: each invocation reads a cell at a **rotated, non-sequential address** (`new_mod = (11*id + 999*iter + calc_param + 7*(id/0x2000)) % 0x2000`), compares against the recomputed expected value, and on mismatch (slow path only) updates atomics: per-bit single-error index histogram, count of wrong-bit counts, count of 1-bits in the actual value, min/max actual value, min/max error index, `done_iter_or_err = 0xFFFFFFFF`. Workgroup size 64. 1 invocation = 1 vec4.
  - **Write once, reread every iteration.** Part of the memory is written only at the start and re-read forever: errors there are tagged `NEXT_RE_READ` (cell lost data while stored: refresh/retention/temperature) versus `INITIAL_READ` (error on the write/read path). That is why GB read > GB written in the log.
  - Memory is split into windows (each at most 4 GiB minus granularity, at least 2 windows so one can be re-read while the other is rewritten); `TEST_DATA_KEEP_FREE = 400 MiB` is left unused; allocation retries with less memory on failure (`err_retry_with_lower_memory`); sizes from `VK_EXT_memory_budget` on the DEVICE_LOCAL heap (non-discrete GPUs also consult free system RAM); needs at least 1 GB. Typical allocation 3.5 to 4 GB because many drivers refuse bigger contiguous allocations.
  - One dispatch per window per submit, then `wait_for_fences`; progress printed at 1 s, 10 s, 100 s intervals with GB/s.
- **Error taxonomy (README theory section).** Single-bit (stuck/marginal bit; EDC may fix on-wire), multi-bit/bus errors, data-inversion-bit errors, retention errors (NEXT_RE_READ), **address-bus errors** (random 12 to 20 flipped bits out of 32, wrong cell read), critical chip/controller errors (all 0 or all 1, vendor marker patterns), errors in the counters themselves (millions of errors = GPU basically broken), and compute/compare-logic errors.
- **Timing guidance in the README.** "Wait at least 6 minutes" so temperature rises; near-limit errors can need 2 to 3 hours; low-clock errors and clock-switch errors are hard to catch; v0.5 added a preliminary 15 s pause of load after warm-up to catch the "downclock then ramp" case. [V README]
- **What to take.** The whole design is directly portable to D3D11 compute (RW raw buffers, `[numthreads(64,1,1)]`). We would reimplement from the description (algorithms are not copyrightable); if any shader text were copied, Zlib needs the notice kept (put it in `THIRD_PARTY_NOTICES.md` per project rule).

### 2.7 MemtestG80 / MemtestCL (Imran Haque, Stanford / Folding@home)
- Memtest86-derived suite for GPU memory and logic. Open-source MemtestCL (OpenCL port of the CUDA MemtestG80) is **LGPL** (`COPYING.lgpl` in github.com/ihaque/memtestCL; GitHub reports spdx `NOASSERTION`, LGPL version not verified). Kernel layout: 1k work-groups x 512 work-items, N words per thread, linear coalesced addressing. [V github.com/ihaque/memtestCL README + kernels header; S simtk.org]
- Test list (as reproduced by NVIDIA DCGM's memtest plugin, which uses the same classic set): walking 1 bit, address check (each location holds its own address), moving inversions with ones/zeros, with an 8-bit pattern, with random pattern (60 patterns), block move (64 moves), moving inversions 32-bit shifting pattern, random-number sequence (1 MB), **modulo-20 with random pattern** (pattern on every 20th word, complement elsewhere, 20 shifts: defeats caches/write coalescing), bit fade (write pattern, wait ~1 min to hours, re-check), and a bandwidth-maximising "memory stress" kernel (1000 iterations). Errors are reported on any mismatch (`DCGM_FR_MEMORY_MISMATCH`). [V docs.nvidia.com/datacenter/dcgm/latest/reference/diagnostics/plugins/memtest.html; S for the Modulo-20/bit-fade descriptions from memtest86 docs]
- **Use for us.** The test catalogue is the classic reference for VRAM patterns; we can implement walking-ones, address, moving inversions (0x00/0xFF, 0x55/0xAA, seeded random) and modulo-20 as compute kernels. Study only; do not copy LGPL code into a GPL-3-or-later tree without checking the LGPL version.

### 2.8 Other relevant open-source tools (for studying)
| Tool | What | Licence |
|---|---|---|
| clpeak (krrishnarraj) | Peak bandwidth (float..float16 vectors), SP/DP/half/int throughput, transfer bandwidth, kernel launch latency; "small, tight kernels" | GPL-3.0 (GitHub API; older mirrors showed Unlicense/Apache, so check the commit you study) |
| vkpeak (nihui) | Vulkan equivalent: fp32/fp16/fp64/int8..int64, bf16/fp8, matrix kernels, bandwidth; prints GFLOPS / GIOPS / GBPS | MIT |
| Quake II RTX (NVIDIA) | Vulkan path-traced game, cross-vendor via `VK_KHR_ray_query`; usable as an external RT load | GPL-2.0 (GPL-3-compat. unclear, not needed) |
| gpu-burn | see 2.5 | BSD-2 |
| memtest_vulkan | see 2.6 | Zlib |

### 2.9 Ray tracing and "transient" workloads
- Ray tracing loads (Speed Way, Quake II RTX, Port Royal) light up RT cores/BVH traversal and have a different power/clock profile than raster/compute. Needs D3D12 DXR (or Vulkan RT). Not in scope for a first version; keep as a future "RT burn" mode. [S]
- Transient/spiky loads: GPU power excursions range from microseconds to milliseconds and a PSU must absorb them; ATX 3.0/3.1 requires tolerating up to 200 % of rated power for 100 us, 180 % for 1 ms, 160 % for 10 ms, 120 % for 100 ms. Standard monitoring (typically 10 to 100 ms) cannot see them. OCCT's Switch mode is the practical way to provoke them from software. [S seasonic.com/insights/gpu-power-spikes-explained, evezone, hwbusters]. [I] From a CPU-driven scheduler we can realistically switch at 10 to 500 ms periods; sub-ms transients would need GPU-side alternation inside a kernel.

### 2.10 Benchmarks and their scoring (for the benchmark half)
- **3DMark.** Fixed scenes; the score combines graphics and (for some tests) CPU sub-scores; results are relative (no GFLOPS). Stress tests use the frame-rate-stability metric above. [V UL pages]
- **Geekbench 6 Compute.** OpenCL 1.2, Metal 3.0, Vulkan 1.2. Workloads: image editing (horizon detection, edge detection, Gaussian blur), image synthesis (feature matching, stereo matching, particle-physics simulation), machine learning (background blur with DeepLabV3+ at 1080p, face detection). Score is calibrated to a baseline of 2500 (Dell Precision 3460, Core i7-12700); double the score = double the speed. [V via search summary of geekbench.com docs (PDF could not be parsed here); S]
- **PassMark G3D Mark.** Four graphics tests (DX9: 11 planes/500 trees/water at 1080p 8xAA; DX10: islands+meteors, geometry shader; DX11: up to 50 jellyfish with unordered transparency and tessellation, 1080p 4xAA; DX12: up to 100,000 asteroids and 71 ships at 4K with compute-shader bloom) plus GPU compute tests (DirectCompute and OpenCL); results averaged into the G3D score. [V videocardbenchmark.net/gpu_test_info.html]
- **GFXBench.** High-level scenes (Aztec Ruins: compute-shader tone mapping/bloom/motion blur; Car Chase: tessellation; Manhattan) and **low-level** tests: ALU 2, Tessellation, Texturing, Driver Overhead 2, Fill, Alpha Blending, Render Quality, Battery/Stability. Reports FPS and also fill-rate/ALU style numbers. [V via search summaries of gfxbench.com / apkmirror; S]
- **Synthetic peak tools (clpeak / vkpeak).** Report GFLOPS (FP), GIOPS (INT), GB/s (bandwidth). These are the units to use for our "Gop/sec", "Top/sec" style gauges. [V github READMEs]

---

## 3. Windows / DirectX facts that constrain the implementation

### 3.1 TDR (Timeout Detection and Recovery)
- Default timeout **2 s**: the GPU scheduler tries to preempt the running work; if the GPU cannot complete or preempt within `TdrDelay` (default 2 s) it declares the GPU hung. After 5 TDRs within 60 s (`TdrLimitCount`/`TdrLimitTime`) the OS bug-checks (0x117). `TdrLevel` default = 3 (recover). The recovery purges all VRAM allocations; the app must release and recreate the D3D device and all objects. Drivers are told to keep each DMA buffer under 2 s. [V learn.microsoft.com/windows-hardware/drivers/display/timeout-detection-and-recovery and .../tdr-registry-keys]
- **Never change the TDR registry keys** (Microsoft: apps shouldn't manipulate them; also needs admin). Design the workload to be TDR-proof instead.
- The preemption granularity of the adapter (graphics: DMA buffer/primitive/triangle/pixel/instruction; compute: dispatch/thread group/thread/instruction) is exposed in `DXGI_ADAPTER_DESC2/3` (`GraphicsPreemptionGranularity`, `ComputePreemptionGranularity`). With dispatch-level granularity a long dispatch blocks the desktop (DWM) for its duration. [V learn.microsoft.com ... IDXGIAdapter4::GetDesc3]
- **Practical rule [I]:** keep every submission to roughly 20 to 100 ms on the slowest target (iGPU) and self-calibrate: start with a small group count, time it with `D3D11_QUERY_TIMESTAMP(+DISJOINT)` or a CPU event query, then scale the group count to hit a target of about 30 to 50 ms per submit, always far below 2 s. Dispatch limits: 65535 groups per dimension, 1024 threads per group (cs_5_0). [V learn.microsoft.com ID3D11DeviceContext::Dispatch / compute shader overview]

### 3.2 Device removed as the crash signal
- `DXGI_ERROR_DEVICE_REMOVED` (0x887A0005), `_DEVICE_HUNG` (0x887A0006), `_DEVICE_RESET` (0x887A0007), `_DRIVER_INTERNAL_ERROR` (0x887A0020), `_INVALID_CALL`. Call `ID3D11Device::GetDeviceRemovedReason` (D3D12: `ID3D12Device::GetDeviceRemovedReason`, and `ID3D12Fence::SetEventOnCompletion(UINT64_MAX)` fires on removal) before releasing the device. HUNG = the app's commands hung the GPU; DRIVER_INTERNAL_ERROR = driver reset the device; REMOVED = driver update/physical removal (and sometimes a power/OC crash). In an OC test any of these is a **fail with a timestamp**, then the test process should recreate the device (or just exit with a result code). `dxcap -forcetdr` can force one for testing the handler. [V learn.microsoft.com ID3D11Device::GetDeviceRemovedReason; handling-device-lost-scenarios; dxgi-error]
- For D3D11 compute (no Present) removal surfaces as an `HRESULT` from `Map` on the staging resource, from `GetData` on queries, or from `Flush`. Always poll `GetDeviceRemovedReason` when a query/Map fails or a submit takes abnormally long.
- Recommendation [I]: run the load engine in a **child process** (like the existing `oma-overlay.exe` pattern) so a driver reset, a hang or an OOM can never take down the UI/tray app; communicate results over a pipe; put the child in a Job Object.

### 3.3 Sizing VRAM safely
- **Per-resource guarantee in D3D11:** `min(max(128 MB, 0.25 * dedicated VRAM), 2048 MB)`. The runtime *attempts* larger resources but may fail; so on a 16 GB card allocate many buffers of up to 2 GB; on an 8 GB card up to 2 GB; on a 4 GB card 1 GB; on 2 GB, 512 MB. Several 256 MB to 1 GB chunks are the portable choice; fill them with a loop until budget is reached. UAV views over raw buffers are limited by 32-bit byte offsets. [V learn.microsoft.com/windows/win32/direct3d11/overviews-direct3d-11-resources-limits]
- **Budget:** `IDXGIAdapter3::QueryVideoMemoryInfo(node 0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL)` returns `Budget`, `CurrentUsage`, `AvailableForReservation`. If usage exceeds budget the process is "intermittently frozen" and creation APIs may fail; the budget fluctuates as other apps run (and shrinks when the app is not foreground). On discrete GPUs LOCAL = VRAM; on UMA/iGPUs NON_LOCAL is always 0 and LOCAL is carved out of system RAM. [V learn.microsoft.com QueryVideoMemoryInfo, DXGI 1.4 improvements, D3D12 Memory management strategies]
- **Reference practice:** memtest_vulkan keeps 400 MB free and retries with less memory after a failure; OCCT suggests 95 % of VRAM on a dGPU and 90 % on an iGPU/shared memory. [V memtest_vulkan source; S OCCT guides]
- **Residency caveat [I]:** in D3D11 the OS can page resources between VRAM and system memory. A "VRAM test" is only valid if the data really lives in VRAM: after the first full pass check `CurrentUsage` (LOCAL) increased by about the allocated amount and that measured bandwidth is plausible; if the budget is exceeded, bandwidth collapses to PCIe speeds (also a diagnostic). D3D12 gives explicit `MakeResident/Evict` and heaps in the right pool, which is better for this job. Remember the budget is per-process and depends on other apps (browsers, games); show a warning if another process holds a lot of VRAM (our PDH provider already sees per-process GPU memory).
- iGPU: memory test = system RAM test through the GPU; use a smaller fraction (90 % of the *budget* at most, probably 50 % of free RAM) and warn the user.

### 3.4 Floating-point determinism across GPUs
- Direct3D float arithmetic is a *subset* of IEEE-754 with relaxed tolerances: add/sub/mul within 0.5 ULP but truncation allowed; denormals flushed to zero on 32-bit math; `mad` may be fused or not (hardware choice, but must be consistent within the same hardware); `sqrt`/`rcp` 1 ULP; `rcp`/`rsq` more relaxed; no FP exceptions. Doubles: IEEE 754R, denormals honoured, optional (`D3D11_FEATURE_DOUBLES`). 16-bit: round-to-nearest-even, 0.5 ULP unfused/0.6 fused, denormals preserved. [V learn.microsoft.com/windows/win32/direct3d11/floating-point-rules; mad function page]
- **Consequence:** do **not** compare float results between different GPUs or against a CPU reference with exact equality. Use one of:
  1. **Integer arithmetic** (imul, iadd, xor, shifts, rotates built from shifts): bit-exact everywhere; CPU golden values are valid on any GPU. Best for ALU error detection.
  2. **FP32 on exactly-representable integer-valued data** (e.g. matrix entries in [-8, 8], accumulations < 2^24): every product and partial sum is exact regardless of fusing or order, so results are bit-identical on all vendors and match a CPU integer reference. [I, but gpu-burn's comment about order-independence points the same way]
  3. **Self-consistency** (same GPU, same shader, same inputs; compare redundant copies or run-to-run), as gpu-burn does. Valid for any float math on a given device/driver.
- Avoid chaotic/contracting float recurrences that hide errors (a contracting map `x = a*x + b` with |a|<1 forgets low-bit errors) and anything producing NaN/Inf (NaN != NaN breaks comparison; compare bit patterns with `asuint`).

### 3.5 Adapter selection, headless, per-process visibility, HAGS
- **Adapter selection.** `IDXGIFactory6::EnumAdapterByGpuPreference` (Win10 1803+) orders by `HIGH_PERFORMANCE` (xGPU, dGPU, iGPU) or `MINIMUM_POWER` (iGPU first). Better for a test tool: enumerate **all** adapters (`EnumAdapters1`/`EnumAdapterByLuid`), skip `DXGI_ADAPTER_FLAG_SOFTWARE` (the "Microsoft Basic Render Driver", VendorId 0x1414/DeviceId 0x8C is always present, never returns DEVICE_REMOVED) and let the user choose, identifying each by `AdapterLuid` (stable within a boot; correlate with our existing D3DKMT/PDH LUIDs so the stress process and the sensors talk about the same GPU). Pass the chosen `IDXGIAdapter` to `D3D11CreateDevice` with `D3D_DRIVER_TYPE_UNKNOWN`. Use `DXGI_ADAPTER_DESC3` for VRAM sizes and the preemption granularity. [V learn.microsoft.com EnumAdapterByGpuPreference, DXGI overview, GetDesc3]
- **iGPU vs dGPU.** Both are ordinary adapters; the iGPU (AMD here) shares DRAM with the CPU, has a small `DedicatedVideoMemory`, but a large LOCAL budget carved from system memory. Run one test on one adapter at a time by default (running both together is a separate "all GPUs" mode; they contend for DRAM bandwidth and power).
- **Headless.** D3D11/D3D12 compute needs no window or swap chain. Graphics loads can render to an offscreen `Texture2D` render target and never `Present`; DXGI's occlusion (`DXGI_STATUS_OCCLUDED`) logic then does not apply. [I]; MS docs on swap chains only apply when presenting. memtest_vulkan runs as a plain console app with no admin and no window. [V]
- **Shader compilation without SDKs.** `d3dcompiler_47.dll` ships in System32 on Windows 10/11 [S, widely known], so runtime HLSL compile for D3D11 (cs_5_0) is possible, or ship precompiled DXBC blobs. For D3D12 SM 6.x you need DXIL **signed by `dxil.dll`'s validator** (unsigned DXIL is rejected on end-user machines), so compile offline with DXC + `dxil.dll` and embed the blobs. [V devblogs.microsoft.com/directx, DirectXShaderCompiler docs/DXIL.rst via search; S]
- **Vulkan** is *not* guaranteed on stock Windows: `vulkan-1.dll` arrives with the GPU driver package (memtest_vulkan: "requires system-provided vulkan loader ... installed with graphics drivers on most OS"). It is nearly universal on gaming PCs, but D3D11/D3D12 are guaranteed. Recommendation: D3D11 first, D3D12 second, Vulkan not needed. [V memtest_vulkan README]
- **Per-process GPU usage visibility.** The Windows performance counters `\GPU Engine(pid_<pid>_luid_0x........_0x........_phys_<n>_eng_<n>_engtype_<type>)\Utilization Percentage` (Win10 1709+) report per-process, per-engine utilisation; engine types include 3D, Compute, Copy, VideoDecode... Task Manager's headline "GPU %" is the busiest engine, not an average. D3D11 compute and graphics both run on the 3D engine; a D3D12 *compute queue* shows up as a separate Compute engine [I, widely observed]. Our existing PDH provider can therefore show the stress process's own load and detect *other* heavy GPU users. [S learn.microsoft.com Q&A, rainmeter docs, uberAgent docs]
- **HAGS (hardware-accelerated GPU scheduling, WDDM 2.7, Win10 2004+).** GPU-side scheduler processor takes over submission scheduling; can be on or off per user. Our workload should be robust to both; the state is readable via `D3DKMT_WDDM_2_7_CAPS.HwSchSupported/HwSchEnabled` (documented as "reserved for system use", so treat as best-effort) and worth reporting in the test summary because it can change dispatch latency behaviour. TDR semantic is unchanged as far as documentation goes [I]. [V learn.microsoft.com D3DKMT_WDDM_2_7_CAPS; S benchmarks]

---

## 4. Error-detection techniques usable through D3D11/D3D12

### 4.1 Compute ALU error detection
1. **Integer hash chain (exact).** Each thread seeds from its global id and a per-run seed, then iterates N rounds of e.g. `x = rotl(x * C1 + id, k) ^ (x >> s)` (imul/iadd/xor/shift). Writes only a 32-bit digest (or XOR-folds into a per-thread-group digest) at the end: tiny memory traffic, so it is ALU-bound. The CPU precomputes expected digests for a **sampled subset** of threads at start (cheap: a few thousand threads x N rounds) and the GPU digests for the whole grid are compared on-GPU against a second, independently executed copy (different thread-to-lane mapping), with an error counter (atomics) read back once per submit. Catches: single wrong bit anywhere in the chain (hash avalanche propagates it to the digest), ALU timing failures at low voltage, register file/SRAM corruption.
2. **Redundant execution cross-check (gpu-burn style).** Compute the same tile twice (or into `K` buffers) and compare. Cheap to add to any workload. Catches random faults; misses identical systematic faults, hence combine with item 1's CPU golden.
3. **Exact-integer FP32 GEMM / FMA chain.** Matrix entries small integers, so results are exact everywhere; verify against a CPU-computed checksum of a few output rows plus redundancy compare. Exercises FP32 FMA units and register/shared-memory paths (use `groupshared` tiles for a real GEMM-like load: ~16 to 32 KB per group is safe on cs_5_0).
4. **Divergent/data-dependent branches and `groupshared` stress** (barriers, bank patterns) to move beyond pure straight-line FMA. Optional.
5. **FP64** (if `D3D11_FEATURE_DOUBLES`): optional; most consumer GPUs run it at 1/32 to 1/64 rate, so it is a poor stress but a valid correctness probe.
6. **FP16 / INT8 / matrix:** D3D11 `min16float` is only a precision *hint* and may run at FP32 (`D3D12_SHADER_MIN_PRECISION_SUPPORT` explicitly "doesn't guarantee that the graphics hardware will actually run at lower precision"); real FP16 needs D3D12 SM 6.2 `Native16BitShaderOpsSupported`. Matrix/tensor paths need D3D12 SM 6.9 (long vectors, released 26 Feb 2026; Cooperative Vector was deprecated in favour of a unified matrix API planned for SM 6.10) or DirectML, driver-version dependent: out of scope for v1. [V learn.microsoft.com D3D12 options docs; S devblogs.microsoft.com/directx/shader-model-6-9-retail-and-more, guru3d]

### 4.2 VRAM pattern write/verify (memtest-style)
Implement in D3D11 with `RWByteAddressBuffer` (or `RWStructuredBuffer<uint4>`):
- **Address-derived rotated pattern, write once re-read many** (memtest_vulkan): `v(i) = rotl(addr+k, (addr+k) % 31)`; read order rotated each iteration; per-error statistics via atomics (count, bit-position histogram, min/max error index, flipped-bit-count histogram).
- **Classic suite as occasional passes** (every few minutes, because each full pass is slow): walking ones (address lines), address-in-address, moving inversions (0x00000000 / 0xFFFFFFFF, 0x55555555 / 0xAAAAAAAA, 32-bit shifting pattern, seeded random + complement), modulo-20 (defeats burst/coalescing optimisations), optional bit-fade (write, wait N minutes under load pause, verify).
- **Seeded random:** a counter-based RNG (`hash(seed, index)`) means no memory is needed to remember the data: the verify kernel recomputes the expected word from the index. Use a fresh seed per pass.
- **Report** per error: mode (initial read vs re-read), address range, bit-error statistics, and classify like memtest_vulkan (single-bit vs multi-bit vs address-bus-like (about 16 flipped bits) vs all-0/all-1 vs counter corruption).
- **Cache avoidance:** make the working set far larger than the L2/Infinity Cache (RTX 4080 L2 is 64 MB [S]); use windowed access with rotated order so reads come from DRAM.
- **Dispatch sizing:** one window (e.g. 256 MB to 1 GB) per submit is typically 1 to 10 ms on a dGPU (memtest_vulkan on a 3090 reads ~750 GB/s), but 50 to 100x slower on an iGPU: self-calibrate window sizes, never launch one dispatch over the whole VRAM.

### 4.3 Graphics artifact detection
- Render a **deterministic offscreen scene** (fixed camera, fixed seed, no temporal effects, no dithering, no random alpha, no overdraw order-dependent blending/UAV atomics, MSAA fixed). On a given GPU/driver identical inputs produce identical pixels [I]. Compute a hash of the render target **on the GPU** (a compute pass folding the RT into a few 32-bit words), copy only those words to a staging buffer, and compare with the hash of the first frame (reference captured at stock/cool state, or at the beginning of the run).
- On mismatch optionally read back the whole RT once and count differing pixels / locate them (sparkles, tile corruption, banding). Use a tolerance of exact equality; if some driver paths prove non-deterministic (e.g. certain texture filtering at LOD boundaries under power states) fall back to "N pixels differ by more than T" [I].
- This is the generic form of the "artifact scanner" in FurMark/Kombustor/OCCT; the algorithm is undisclosed there, so this is our own design. Cheap enough to run every N-th frame (readback of 16 bytes).

### 4.4 Silent errors and what the hardware hides
- **GDDR5/GDDR6/GDDR6X EDC.** The memory interface CRC detects corrupted bursts and the controller **retransmits until the burst succeeds**; therefore an overclocked VRAM bus often shows up as *lower bandwidth*, not wrong data. Only bus errors are covered: errors inside the DRAM cells or the memory controller are not detected by EDC. GDDR6X is reported to behave the same way (error detection and replay). [S itigic.com GDDR6X article; S anandtech GDDR5 EDC article (fetch failed, summarised in search result); OCCT itself notes transfer errors are invisible to its VRAM test]
- **Design consequence:** the VRAM OC mode must log bandwidth per window and flag a sustained drop versus the best/early window (suggest: flag if the median of the last N windows is >= 3 % below the best, echoing 3DMark's 97 % rule, and mark it as "EDC retries suspected" rather than a hard error; verify against thermal/power throttling flags from our own sensors first).
- **Other signals we can read without admin** (our providers already exist): NVML ECC error counters only on ECC-capable (pro) boards; PCIe replay counter (`nvmlDeviceGetPcieReplayCounter`) [I, verify in the NVML docs we already use]; clocks/throttle reasons to tell throttling from instability; Windows "Display" event log (Event 4101 "display driver stopped responding and has recovered") for a TDR that happened without us noticing [I].

### 4.5 Making "OC stable" meaningful
- Run the verification workload **while** the load varies (OCCT Variable/Switch idea), because the failing V/F points are often at partial load [V ocbase].
- Warm-up/soak: errors can be temperature dependent; memtest_vulkan's standard test is 5 to 6 min for that reason and 2 to 3 h for marginal cases. [V README]
- Include a **load pause then ramp** (about 15 s) to catch downclock/upclock transition errors (memtest_vulkan 0.5 approach). [V README]
- Verdict = no verification mismatch, no device removed/hung, no stalled dispatch (watchdog), and loop-to-loop throughput stability >= 97 % (3DMark rule) after discounting reported throttling.

---

## 5. Compute vs graphics vs memory-bound vs tensor/FP16 workloads (what each stresses)

| Class | Stresses | Power profile | Detects |
|---|---|---|---|
| FMA/ALU compute (FP32 chains, independent accumulators) | shader cores, register file, clocks/voltage | very high, steady | core instability, VRM load |
| Integer/hash compute | integer ALUs (separate datapath on some archs, e.g. NVIDIA INT32 lanes), schedulers | high | exact errors via digest |
| Memory-bound stream (read/write/copy float4) | DRAM, memory controller, bus, L2/Infinity Cache bypass | medium (memory power high on GDDR6X) | VRAM clock/timing errors, EDC retries via bandwidth |
| Random-access / pointer-chase / modulo-20 | DRAM latency, row buffer, address bus | medium | address-bus errors |
| Rasterization/overdraw (FurMark-like) | ROPs, pixel shaders, texture units, fixed-function | very high ("power virus") | artifacts, ROP/TMU faults |
| Mixed concurrent (3D + async compute + copy) | whole chip, scheduler, PCIe | highest realistic | cross-engine interactions (D3D12) |
| Matrix/tensor/FP16 (WMMA, DirectML) | tensor/matrix units | very high on supporting GPUs | tensor-core faults (D3D12 SM 6.9/DirectML; vendor/driver dependent) |
| Ray tracing (DXR) | RT cores/BVH traversal | high, different profile | RT unit faults (D3D12) |

Realistic GPU "FLOPS" sanity check: RTX 4080 = 9728 FP32 lanes x 2 FLOP x ~2.5 GHz boost, about 48.7 TFLOPS theoretical FP32; memory 256-bit GDDR6X @ 22.4 Gbit/s = about 717 GB/s; L2 64 MB. [S pugetsystems / notebookcheck / videocardbenchmark.net] A D3D11 FMA test will not reach theoretical peak on every architecture (dual-issue, compiler scheduling), so report **achieved** throughput, never "% of peak" unless the architecture is known.

---

## 6. Benchmark design notes (compute throughput, bandwidth, fill rate)

- **Units** (clpeak/vkpeak convention): GFLOPS/TFLOPS for FP, GIOPS/TIOPS for integer, GB/s for bandwidth, Gpixel/s and Gtexel/s for fill rate. "FMA = 2 FLOP". [V clpeak, vkpeak READMEs]
- **FMA throughput kernel.** Each thread holds 8 to 16 independent accumulators (hides FMA latency), inner loop of `mad` unrolled by the compiler, loop count large enough that launch overhead is < 1 %, final accumulators XORed into a single `RWStructuredBuffer` write that depends on every accumulator (prevents dead-code elimination; the compiler cannot constant-fold if the seeds come from a cbuffer constant). FLOPs = threads x iterations x accumulators x 2. Same shape for INT32 (`imad`/`umad` or mixed `iadd+xor`).
- **Bandwidth kernel.** `ByteAddressBuffer.Load4` -> `RWByteAddressBuffer.Store4` copy; separate read-only (reduce into a few words) and write-only variants; working set >= 512 MB to 1 GB (well beyond cache); coalesced consecutive addressing per wavefront; bandwidth = bytes moved / GPU time. Copy counts read+write bytes (state which convention).
- **Fill rate.** Full-screen quads (or many large triangles), no blending vs blending, RT format R8G8B8A8 and R16G16B16A16F, into an offscreen RT, depth off; Gpixel/s = pixels / GPU time. Texture rate: sample N textures per pixel. Optional graphics-scene FPS (fixed scene, fixed seconds) à la PassMark/GFXBench.
- **Timing.** `D3D11_QUERY_TIMESTAMP` + `TIMESTAMP_DISJOINT` (check `Disjoint == FALSE` and use the reported frequency) per measured batch; or CPU QPC around Flush + event query for batches >= 50 ms. Use multiple batches inside one measurement.
- **Warm-up and reproducibility.** Run 3 to 5 s of warm-up (clocks ramp, boost stabilises), then 5+ measured windows of >= 1 s; report the **median** and the spread (min/max or MAD); record average core/memory clock, power and temperature from our own sensors during the windows and the throttling state; flag runs where clock varies by more than about 5 % or throttling was active ("result not comparable"). Disclose boost: results with different boost behaviour are different results (this is also why Geekbench uses a fixed reference-machine baseline and reports relative scores).
- **Isolation.** Refuse or warn if another process shows significant GPU engine utilisation (we already collect per-process GPU data), if on battery (laptops), or if the adapter is a software adapter.
- **Score.** Keep raw units as the primary result (TFLOPS FP32, TIOPS INT32, GB/s). An optional relative "OMA score" can be a geometric mean of ratios to a fixed reference table, but needs a published reference device and is not necessary for v1.

---

## 7. Licensing summary and what we may reuse

| Source | Licence | Reuse stance for a GPL-3.0-or-later project |
|---|---|---|
| memtest_vulkan | Zlib | GPL-compatible; reimplement the algorithm from the description (preferred); if shader text/structure is copied keep the Zlib notice in `THIRD_PARTY_NOTICES.md` |
| gpu-burn | BSD-2-Clause | Same; our design differs anyway (own kernels, no cuBLAS) |
| vkpeak | MIT | Same; study kernel layout for peak FLOPS/IOPS |
| clpeak | GPL-3.0 (current) | Compatible; check licence of the exact revision studied (older mirrors say Unlicense/Apache) |
| MemtestCL / MemtestG80 | LGPL (version not verified) | Study the tests; do not paste into our tree before checking LGPL-2.1-only vs 3 vs or-later |
| NVIDIA DCGM memtest | NVIDIA (not verified) | Use only the public test list from the docs |
| Quake II RTX | GPL-2.0 | Not needed; GPL-2-only would be incompatible |
| FurMark, Kombustor, OCCT, 3DMark, Unigine, Geekbench, PassMark, GFXBench | Proprietary | Ideas only; never copy shaders/assets/scene names |

The project rule "no proprietary headers (NVML/ADL/IGCL), hand-written bindings" is unaffected: D3D11/D3D12/DXGI are Windows system APIs, already used by `oma-win`/`oma-overlay`.

---

## 8. Proposed modes for our app

### 8.0 Architecture assumptions [I]
- One GPU load engine in its **own process** (`oma-gpuload.exe`-style helper, spawned like the overlay, pipe protocol, Job Object, exit codes for "device lost/hung/OOM"), selected adapter by LUID, no window, D3D11 first. The UI app only configures, shows progress and the verdict, and keeps recording sensors with its existing providers (clocks, temps, power, throttle reasons) so results can be correlated.
- All modes share: self-calibrating submit size (target 30 to 50 ms per submit, hard ceiling well below 2 s), timestamp queries, `GetDeviceRemovedReason` polling, a CPU-side watchdog on each fence/query (e.g. 10 s without completion = "hung" result), VRAM sizing from `QueryVideoMemoryInfo` (95 % of budget on dGPU, 90 % on iGPU, minus 400 MB headroom, retry smaller on failure), verification counters read back once per second.
- Names of tests are working titles.

### 8.1 Stress mode catalogue

Legend: **D3D11** = feasible with D3D11 compute/graphics only. **D3D12** = needs D3D12.
Durations: **N** = "normal" stress test; **OC** = overclocking stability test.

| # | Mode | Workload | API | Targets | Error detection | Duration N / OC |
|---|---|---|---|---|---|---|
| S1 | **Compute Burn (FP32)** | Many groups of threads running long unrolled FMA chains, 8 to 16 independent accumulators, plus a small shared-memory tile exchange each outer loop | D3D11 | Shader cores, register file, clocks/voltage, VRM | N: none beyond device-lost, watchdog, throughput stability >= 97 %. OC: exact-integer FP32 variant verified against CPU golden + redundant copy | N 10 min (5 to 60 selectable) / OC 30 min |
| S2 | **Integer Hash Burn** | Per-thread integer hash chain (imul/iadd/xor/rotl), digest only | D3D11 | Integer ALUs, schedulers | CPU golden digests for a thread sample + dual-execution compare on GPU; mismatch counter per submit | N: optional 5 min / OC 20 min inside the OC suite |
| S3 | **Memory Stream** | Read/write/copy float4 over >= 1 GB working set | D3D11 | DRAM bandwidth, memory controller, memory clock | N: bandwidth stability. OC: bandwidth-regression detector (EDC retry proxy) + sampled data verify | N 5 min / OC 10 min (always combined with S4) |
| S4 | **VRAM Verify** | memtest_vulkan-style address-derived pattern, write-once/reread, rotated read order, periodic classic passes (walking ones, address, moving inversions, modulo-20, random seeded) over 90 to 95 % of VRAM budget in 256 MB to 1 GB chunks | D3D11 (D3D12 for explicit residency, optional) | VRAM cells, bus, address lines, retention at temperature | Every word verified on GPU; atomics collect count, bit histogram, min/max address, bit-count classification, INITIAL vs RE_READ | N: not offered or 6 min quick / OC: standard 6 min (minimum, warm-up), 30 min default as part of suite, extended 2 h |
| S5 | **Graphics Burn (fur/overdraw)** | Offscreen render: multi-layer shell fur, heavy procedural pixel shader, high overdraw, many instances; no vsync, no window | D3D11 | ROPs, TMUs, pixel shaders, fixed function, power virus profile | Device-lost + watchdog; OC variant adds GPU-side RT checksum compare every N frames (artifact scan) | N 10 min / OC 15 min |
| S6 | **Artifact Scan** | Deterministic scene (fixed seed, no temporal randomness) rendered at fixed resolution; GPU hash of the RT copied to staging; on mismatch readback and pixel diff | D3D11 | Raster pipeline, texture units, ROPs, caches under OC | Hash vs reference frame; diff count and location | OC 10 min inside the suite |
| S7 | **Adaptive Ramp (Variable)** | S1+S2 verify workload with load level 20 -> 100 % in 5 % steps every 20 to 60 s (partial occupancy by reducing group count, plus duty-cycled idle for lower levels); records the level at which the first error/hang occurs | D3D11 | Full V/F curve, boost behaviour, partial-load instability (OCCT claims 40 to 60 %) | Same as S1/S2 verification; report "first error at X % / clock Y MHz" | N: 15 to 20 min sweep / OC: 30 min, optionally two sweeps |
| S8 | **Transient Switch** | Alternate between 100 % (S1) and ~10 to 20 % (or idle) at periods 10 ms to 500 ms (selectable, with a random jitter option) | D3D11 | PSU/VRM transient response, power-state transitions, coil whine | Verification on every high phase; device-lost; warn that sub-ms transients cannot be generated from the CPU | N: 5 min / OC: 15 min |
| S9 | **Pause/Ramp** | After >= 5 min warm-up, stop load for 10 to 15 s, then restart at 100 % (memtest_vulkan 0.5 idea) | D3D11 | Low-clock and clock-switch errors | Verification after restart; first-iteration latency | OC only: repeat 3 times per hour |
| S10 | **Mixed Concurrent** | 3D raster queue + async compute queue + copy queue running together | D3D12 | Whole chip, scheduler, copy engines, PCIe | Per-queue verification (digest, VRAM pattern, checksum) | Later version; N 10 min / OC 30 min |
| S11 | **FP16 / Matrix Burn** | Native FP16 (SM 6.2) and, when available, SM 6.9 / DirectML matrix workloads | D3D12 | Packed math and matrix/tensor units | Exact small-integer data verified vs CPU golden | Later version |
| S12 | **RT Burn** | DXR ray traversal workload | D3D12 | RT cores | Deterministic image hash | Later version |

Pre-built profiles:
- **Normal stress test:** S5 + S1 combined (graphics + compute, one submit queue), then S7 (ramp) for the last part; verdict = no device-lost, no hang, throughput stability >= 97 % per 20 s window (3DMark rule), temperature/clock/power summary, throttle reasons.
- **Overclocking stability test:** default 30 min (quick 10 min, extended 2 h): S9-aware rotation of S4 + S2 + S1(exact) + S6 + S7 + S8, with the verification layer always on; verdict = zero mismatches, zero device-removed/hung events, no stalled dispatch, bandwidth/throughput stability >= 97 % after excluding reported throttling; failure report includes mode, time, clocks, temperature, power, and the error classification.

### 8.2 Benchmark gauges

| Gauge | Workload (what is timed) | Unit | API | Notes |
|---|---|---|---|---|
| **G1: Compute throughput** | FP32 FMA kernel (independent accumulator chains, grid >= 4x resident capacity, 1 s windows x 5 after 3 to 5 s warm-up, median) and INT32 `umad/xor` kernel; the dial shows FP32 **TFLOPS**, tooltip shows INT32 **TIOPS** (and FP16/FP64 when supported) | TFLOPS / TIOPS | **D3D11 compute**: FP32, INT32, FP64 (if `D3D11_FEATURE_DOUBLES`). **D3D12 needed**: real FP16 (SM 6.2 native 16-bit), INT64, matrix/tensor TFLOPS | Report achieved, not % of peak; record average clocks/power during the windows; flag throttling/clock drift > 5 %. RTX 4080 sanity expectation: tens of TFLOPS (theoretical 48.7) [S] |
| **G2: Memory bandwidth** | Float4 read-only reduce, write-only, and copy over a >= 1 GB working set (beyond L2/Infinity Cache), per-window median; dial shows **copy GB/s** (or read GB/s) with the others in tooltip | GB/s | **D3D11 compute** (raw buffers, 2 GB max per resource, several buffers) | State convention (copy counts read+write bytes). RTX 4080 sanity expectation: up to ~700 GB/s theoretical 717 [S]. iGPU: shows DDR bandwidth available to the GPU |
| G3 (optional): Graphics fill rate | Full-screen quads into an offscreen RGBA8 and RGBA16F RT, no blend and blend, plus textured variant; **Gpixel/s** and **Gtexel/s**; or a fixed-scene FPS à la PassMark/GFXBench | Gpixel/s, Gtexel/s, FPS | **D3D11** | Offscreen, no Present, so independent of vsync/monitor/DWM |

Benchmark methodology to follow everywhere (all D3D11): warm-up, >= 5 windows, median plus spread, timestamp queries, sensors recorded, power-limit/throttle flag, "other GPU process active" warning, refuse software adapters, optional 3 repeat runs with the spread shown.

### 8.3 What is feasible with D3D11 compute alone vs needs D3D12

**Feasible with D3D11 (v1):** S1, S2, S3, S4, S5, S6, S7, S8, S9 (all of the "normal" and "overclocking" profiles above), G1 (FP32, INT32, FP64 optional), G2, G3. Everything runs headless on the selected adapter; no new native dependencies beyond `d3d11.dll`, `dxgi.dll`, `d3dcompiler_47.dll` (or precompiled DXBC).

**Needs D3D12 (later):**
- concurrent multi-queue load (S10: graphics + async compute + copy) because a D3D11 immediate context is a single queue;
- native FP16/INT64/wave intrinsics, FP16/INT8/matrix benchmarks and burns (S11, G1 extensions): SM 6.2+/6.9 and DXIL signed offline;
- explicit VRAM residency (`MakeResident/Evict`, heaps in a chosen pool) for a stricter "this data is really in VRAM" VRAM test and for testing more than 2 GB per resource with confidence;
- ray tracing (S12);
- `ID3D12Fence::SetEventOnCompletion(UINT64_MAX)` device-removed callback (D3D11 polling suffices).

**Not needed:** Vulkan, CUDA, OpenCL, any vendor SDK.

### 8.4 Open questions / things to verify before coding
1. Verify on the dev machine (RTX 4080 + AMD iGPU) that D3D11 `Dispatch` with calibrated 30 to 50 ms batches keeps desktop responsiveness on both adapters (check `ComputePreemptionGranularity`) and that no TDR occurs on the iGPU.
2. Verify that the exact-integer FP32 GEMM design really gives identical results on the NVIDIA and AMD adapters (cheap experiment).
3. Verify GPU-hash artifact check determinism on both adapters over 30+ minutes at stock, to get a false-positive rate near zero before trusting it for OC.
4. Check `d3dcompiler_47.dll` presence and decide runtime compile vs embedded DXBC.
5. Check whether NVML `nvmlDeviceGetPcieReplayCounter` and throttle-reason queries are reachable through our existing hand-written NVML binding (no proprietary header copying).
6. Decide the helper process name/identity (neutral, not containing "furmark"/"occt"), given the historical driver heuristics keyed on executable names.
7. Measure our own overhead: the helper should keep CPU near 0 (event-query polling with `Sleep`, no spin) so the tray/UI budgets in `docs/perf-budget.md` hold while the helper runs.

---

## 9. Sources

Primary (opened and read):
- memtest_vulkan repo, README and `src/main.rs`: https://github.com/GpuZelenograd/memtest_vulkan (raw: https://raw.githubusercontent.com/GpuZelenograd/memtest_vulkan/main/src/main.rs, .../Readme.md)
- gpu-burn: https://github.com/wilicc/gpu-burn (raw `gpu_burn-drv.cpp`, `compare.cu`)
- MemtestCL: https://github.com/ihaque/memtestCL (README.md, memtestCL_kernels.cl)
- NVIDIA DCGM memtest plugin: https://docs.nvidia.com/datacenter/dcgm/latest/reference/diagnostics/plugins/memtest.html
- clpeak: https://github.com/krrishnarraj/clpeak ; vkpeak: https://github.com/nihui/vkpeak
- OCCT: https://ocbase.com/news/occt-gpu-stress-testing-modern-adaptive-approach ; https://www.ocbase.com/news/occt-v14-vulkan ; https://rtech.support/guides/how-to-use-occt/ (search summary)
- FurMark: https://furmark.software/features.html ; https://www.geeks3d.com/furmark/ ; https://geeks3d.com/?p=7516 (GTX 580 limiter) ; Kombustor https://www.geeks3d.com/furmark/kombustor
- 3DMark: https://benchmarks.ul.com/news/check-your-pcs-stability-with-new-3dmark-stress-tests ; https://support.benchmarks.ul.com/support/solutions/articles/44002134931-stress-test-result-screen
- PassMark GPU test info: https://www.videocardbenchmark.net/gpu_test_info.html
- Microsoft Learn: TDR https://learn.microsoft.com/windows-hardware/drivers/display/timeout-detection-and-recovery ; TDR keys https://learn.microsoft.com/windows-hardware/drivers/display/tdr-registry-keys ; D3D11 resource limits https://learn.microsoft.com/windows/win32/direct3d11/overviews-direct3d-11-resources-limits ; floating-point rules https://learn.microsoft.com/windows/win32/direct3d11/floating-point-rules ; DXGI overview (adapters, WARP) https://learn.microsoft.com/windows/win32/direct3ddxgi/d3d10-graphics-programming-guide-dxgi ; EnumAdapterByGpuPreference https://learn.microsoft.com/windows/win32/api/dxgi1_6/nf-dxgi1_6-idxgifactory6-enumadapterbygpupreference ; QueryVideoMemoryInfo https://learn.microsoft.com/windows/win32/api/dxgi1_4/nf-dxgi1_4-idxgiadapter3-queryvideomemoryinfo ; Residency https://learn.microsoft.com/windows/win32/direct3d12/residency ; DXGI_ERROR https://learn.microsoft.com/windows/win32/direct3ddxgi/dxgi-error ; GetDeviceRemovedReason (D3D11) https://learn.microsoft.com/windows/win32/api/d3d11/nf-d3d11-id3d11device-getdeviceremovedreason ; (D3D12) https://learn.microsoft.com/windows/win32/api/d3d12/nf-d3d12-id3d12device-getdeviceremovedreason ; device-removed handling https://learn.microsoft.com/windows/uwp/gaming/handling-device-lost-scenarios ; GetDesc3 / preemption https://learn.microsoft.com/windows/win32/api/dxgi1_6/nf-dxgi1_6-idxgiadapter4-getdesc3 ; D3D12 Options4 (native 16-bit) https://learn.microsoft.com/windows/win32/api/d3d12/ns-d3d12-d3d12_feature_data_d3d12_options4 ; D3D11 doubles https://learn.microsoft.com/windows/win32/api/d3d11/ns-d3d11-d3d11_feature_data_doubles ; D3DKMT_WDDM_2_7_CAPS https://learn.microsoft.com/windows-hardware/drivers/ddi/d3dkmdt/ns-d3dkmdt-d3dkmt_wddm_2_7_caps

Secondary (search summaries, not opened in full): Geekbench 6 GPU workloads https://www.geekbench.com/doc/geekbench6-gpu-compute-workloads.pdf ; GFXBench https://gfxbench.com/gfxbench.jsp ; Superposition https://www.legitreviews.com/?p=193529, https://tomshardware.com/news/unigine-superposition-graphics-benchmark-test,34121.html ; Shader Model 6.9 https://devblogs.microsoft.com/directx/shader-model-6-9-retail-and-more/ , https://www.guru3d.com/story/microsoft-ships-shader-model-69-and-expands-direct3d-12-capabilities/ ; GPU power spikes https://seasonic.com/insights/gpu-power-spikes-explained/ ; GDDR6X EDR https://itigic.com/gddr6x-memory-why-it-achieves-more-speed-and-overclock/ ; GDDR5 EDC https://at-web1.www.anandtech.com/show/2841/12 ; GPU Engine counters https://learn.microsoft.com/en-us/answers/questions/5641645/how-to-get-the-special-process-gpu-usage-with-the ; DXIL signing https://github.com/microsoft/DirectXShaderCompiler/blob/main/docs/DXIL.rst ; RTX 4080 specs https://www.notebookcheck.net/NVIDIA-GeForce-RTX-4080-GPU-Benchmarks-and-Specs.674575.0.html ; Quake II RTX https://github.com/NVIDIA/Q2RTX ; HAGS tests https://gamersnexus.net/guides/3599-windows-10-hardware-accelerated-gpu-scheduling-benchmarks
