# Research: in-game FPS/frametime overlay with rendered vs displayed FPS under frame generation

Date: 2026-10-04. Scope: input for the OpenMonitor Advanced design spec. No code was written in the repository.
Notation: **[fact]** = verified in a primary source (source code, official docs, release notes);
**[report]** = press or third-party report; **[inference]** = my reasoning, to verify in a spike.

---

## 0. TL;DR

- **ETW is the only no-injection route, and it needs privilege.** Starting or controlling a session,
  enabling providers and consuming real-time events all need admin, membership in *Performance Log
  Users*, or a service running as LocalSystem, LocalService or NetworkService **[fact]**. Our app runs
  without admin, so **oma-service (LocalSystem) must own the ETW session** and stream results over the
  existing pipe.
- **PresentMon** (Intel, MIT) is the reference implementation. The latest release is **v2.6.0
  (2026-09-21)**. Its console app is a single **980 KB, Intel-signed, static-CRT exe** (imports only
  advapi32, tdh, user32, shell32, kernel32 and ntdll) that streams per-frame CSV to stdout **[fact]**.
- **Generated frames are detected per vendor, not universally:**
  - **Intel XeSS-FG and AMD AFMF** are tagged through the *Intel-PresentMon* ETW provider
    (`FrameType` = `Intel_XEFG` = 50, `AMD_AFMF` = 100), emitted by the driver or SDK **[fact]**.
  - **DLSS FG/MFG and FSR FG frames are NOT tagged.** PresentMon counts them as ordinary presents.
    Intel says so explicitly: "Currently, PresentMon can only detect generated frames for
    Intel-XeSS-FG and AMD-FMF" **[fact]**.
  - For **DLSS FG**, the app frame rate can be recovered from **NVIDIA PCL Stats (Reflex) ETW
    markers**, one FrameID per app frame. Reflex is mandatory with DLSS FG. PresentMon consumes
    these markers with `--track_pc_latency` (beta) **[fact for the markers; inference for the FPS
    method]**.
  - **FSR 3/4 FG**: no ETW marker is known, so it is indistinguishable via ETW. Steam distinguishes
    it, probably from inside the process **[inference]**.
  - **Lossless Scaling** runs as a separate process that presents its own output: rendered = game
    PID, displayed = LS PID **[inference, high confidence]**.
  - **NVIDIA Smooth Motion**: unknown ETW signature, to be checked with an ETL capture on the
    RTX 4080 of this PC.
- **Anti-cheat:** consuming ETW is passive (no injection, no handle to the game needed). One real
  pitfall: **after EA AntiCheat starts, `StartTraceW` fails with ACCESS_DENIED, while `EnableTraceEx2`
  still works** (PresentMon issue #573) **[fact]**. So create the session early, at service start,
  with no providers enabled, and toggle providers on demand. This is what the PresentMon service does
  since 2.5 **[fact]**.
- **Recommendation (MVP):** oma-service hosts the PresentMon **console** exe as a LocalSystem child,
  then parses CSV, aggregates and pushes batched frame data at 10–20 Hz over the pipe; the app renders
  the overlay. Alternative or upgrade: a private instance of the PresentMon **service** behind
  PresentMonAPI2. Do not reimplement PresentData yet. Details in §8.

---

## 1. PresentMon (GameTechDev/PresentMon)

### 1.1 Version, license, components

- **Latest release:** v2.6.0, published 2026-09-21 (GitHub API). Previous releases: 2.5.1
  (2026-06-29; 2.5.0 was pulled because its *shared service component* caused "a compatibility
  conflict with one of our downstream customers"), 2.4.1 (2026-01-16), 2.4.0 (2025-11-11), 2.3.1
  (2025-06-10) and 2.3.0 (2024-12-17).
  https://github.com/GameTechDev/PresentMon/releases
- **License:** MIT, "Copyright 2017-2024 Intel". The NVIDIA-contributed files (`NV_DD.h`,
  `NvidiaTraceConsumer.cpp`) are also `SPDX-License-Identifier: MIT` (Copyright 2025 NVIDIA).
  `THIRD_PARTY.txt` lists ADL (MIT), NVAPI (MIT-style) and CEF (BSD) for the GUI and service builds.
  MIT is GPL-3.0-compatible: redistribution needs only the copyright notice and the license text
  (put it in `THIRD_PARTY_NOTICES.md`).
  https://github.com/GameTechDev/PresentMon/blob/main/LICENSE.txt
- **Components** (README):
  1. **PresentData**: static C++ library that "performs the lowest-level collection and analysis of
     ETW events". It contains `PMTraceConsumer`, `PresentMonTraceSession` and the hand-written
     provider headers in `PresentData/ETW/*.h`.
  2. **Console application** (`PresentMon-2.6.0-x64.exe`, 980,320 bytes): standalone, writes CSV.
     I verified the Authenticode signature: `CN=Intel Corporation`, valid, Sectigo timestamp.
  3. **PresentMon Service** (`IntelPresentMon/PresentMonService`): ETW frame analysis plus vendor
     telemetry (NVAPI/NVML, ADL, IGCL, WMI), exposed via **PresentMonAPI2.dll** (C API) over a
     named-pipe control channel and shared-memory rings.
  4. **Capture application**: CEF UI plus a D3D11 overlay window, a client of the service. The
     overlay is a separate window in a higher Z-band (`CreateWindowInBand` + `uiAccess=true`, signed,
     installed in Program Files), not injected.
  - MSI v2.6.0 is about 170 MB, mostly CEF.

  https://github.com/GameTechDev/PresentMon/blob/main/README.md ·
  https://github.com/GameTechDev/PresentMon/blob/main/README-Service.md ·
  https://github.com/GameTechDev/PresentMon/blob/main/README-CaptureApplication.md

### 1.2 ETW providers consumed

From `PresentData/PresentMonTraceSession.cpp`, `EnableProvidersListing`, on main as of 2026-10:

| Provider | GUID / header | Events enabled (filtered by event ID) | Notes |
|---|---|---|---|
| Microsoft-Windows-Kernel-Process | `ETW/Microsoft_Windows_Kernel_Process.h` | ProcessStart, ProcessStop, ProcessRundown | `ERROR_ACCESS_DENIED` is tolerated |
| Microsoft-Windows-DxgKrnl | `802EC45A-1E99-4B83-9920-87C98277BA9D` | Always: PresentHistory_Start. **Display**: Blit, BlitCancel, Flip, IndependentFlip, FlipMultiPlaneOverlay, H/VSyncDPCMultiPlane, MMIOFlip, MMIOFlipMultiPlaneOverlay, Present_Info, PresentHistory(_Info/Detailed), QueuePacket Start/Stop, VSyncDPC. **GPU**: Context/Device/HwQueue start/stop/DCStart, DmaPacket. **FrameType**: MMIOFlipMultiPlaneOverlay3_Info | The Performance keyword is patched out; Win7 variants are also enabled |
| Microsoft-Windows-Win32k | | TokenCompositionSurfaceObject, TokenStateChanged (display); InputDeviceRead, RetrieveInputMessage, OnInputXformUpdate (input) | |
| Microsoft-Windows-Dwm-Core | | GetPresentHistory, SCHEDULE_PRESENT, SCHEDULE_SURFACEUPDATE, FlipChain Pending/Complete/Dirty | composed (windowed) presents |
| Microsoft-Windows-DXGI | `CA11C036-0102-4A2D-A6AD-F03CFED5D3C9` | Present Start/Stop, PresentMultiplaneOverlay Start/Stop (+ SwapChain_Start, ResizeBuffers for hybrid) | |
| Microsoft-Windows-D3D9 | | Present Start/Stop | |
| Microsoft-Windows-Direct3D12 | | CreatePipelineStateObject Start/Stop | 2.6 PSO compile metrics |
| **Intel-PresentMon** | `ECAA4712-4644-442F-B94C-A32F6CF8A499` | PresentFrameType (id 1, v0/v1), FlipFrameType (id 2, v0/v1), MeasuredInput/ScreenChange, App* markers (SimulationStart/End, RenderSubmit, PresentStart/End, Sleep, InputSample) | No installed manifest; PresentMon casts raw `UserData` |
| **NVIDIA Display Driver** | `AE4F8626-8265-40D1-A70B-11B64240E8E9` (`NV_DD.h`) | FlipRequest (id 1): fields `alloc`, `vidPnSourceId`, `ts`, `token` | Yields a **FlipDelay** added to the screen time (flip metering) |
| **NVIDIA PCL Stats** | `0D216F06-82A6-4D49-BC4F-8F38AE56EFAB` (`Nvidia_PCL.h`) | All events (no filter, VERBOSE) | Only with `--track_pc_latency` |

https://github.com/GameTechDev/PresentMon/blob/main/PresentData/PresentMonTraceSession.cpp

Session parameters: real-time mode, `BufferSize = 64` KB and `MinimumBuffers = 256` (about 16 MB),
`PROCESS_TRACE_MODE_EVENT_RECORD | RAW_TIMESTAMP`. Event-ID filters use
`EVENT_FILTER_TYPE_EVENT_ID` (max 64 IDs per provider).

### 1.3 Output metrics (exact definitions from `IntelPresentMon/metrics.csv`)

| Metric | Definition (verbatim) |
|---|---|
| MsBetweenPresents | "The time between this Present() call and the previous one, in milliseconds." |
| MsBetweenDisplayChange | "How long the previous frame was displayed before this Present() was displayed, in milliseconds." |
| MsUntilDisplayed | "The time between the Present() call and when the frame was displayed, in milliseconds." |
| MsInPresentAPI | "The time spent inside the Present() call, in milliseconds." |
| MsRenderPresentLatency | "The time between the Present() call and when GPU work for this frame completed" |
| MsBetweenSimulationStart | "The time between the start of simulation processing of the previous frame and this one" (needs app/PCL instrumentation) |
| MsPCLatency | "Time between PC receiving input and frame being sent to the display" (PCL; beta since 2.4.0) |
| FrameTime (v2, "FrameTime-App") | "How long it took from the start of this frame until the CPU started working on the next frame." |
| CPUBusy / CPUWait | CPU work before present / idle until the next frame |
| GPULatency / GPUTime / **GPUBusy** / GPUWait | GPUBusy: "time during which at least one GPU engine is executing work from the target process" |
| DisplayLatency / DisplayedTime | frame start → screen; time on screen ('NA' if not displayed) |
| MsAnimationError | "The difference between the previous frame's CPU delta and display delta." |
| MsClickToPhotonLatency / MsAllInputToPhotonLatency | earliest mouse click (or any input) contributing to the frame → displayed |
| InstrumentedLatency | instrumented frame start → display (Intel XeLL markers) |
| MsFlipDelay | "Delay added to when the Present() was displayed." (NVIDIA flip metering) |
| **FPS-Presents** (PM_METRIC_PRESENTED_FPS) | "The rate at which the application is calling Present()." |
| **FPS-Display** (PM_METRIC_DISPLAYED_FPS) | "The rate at which new frames are being displayed on the screen." Since 2.3.0, "tracks both application and generated frames". |
| **FPS-App** (PM_METRIC_APPLICATION_FPS) | "The rate at which the application is rendering and displaying new frames to the screen." According to the 2.3.1 notes, it "typically matches FPS-Presents except in frame generation scenarios". |
| FrameType | "Whether the frame was rendered by the application or generated by a driver/SDK." |

- **FrameType CSV strings** (`PresentMon/CsvOutput.cpp`): `Application` (also printed for NotSet and
  Repeated), `Intel XeSS-FG`, `AMD AFMF`. Enum values: NotSet 0, Unspecified 1, Application 2,
  Repeated 3, Intel_XEFG 50, AMD_AFMF 100.
- **API stats** (`PresentMonAPI.h`, `PM_STAT`): AVG, PERCENTILE_99/95/90/01/05/10, MAX, MIN,
  MID_POINT, MID_LERP, NEWEST_POINT, OLDEST_POINT, COUNT and NON_ZERO_AVG. 2.5.0 fixed "percentile
  calculation algorithm (e.g. 99% was effectively reporting MAX instead)".
- **Latency:** 2.2.0 cut ETW latency "from 1000ms to ~30ms" by flushing manually
  (`ControlTraceW(..., EVENT_TRACE_CONTROL_FLUSH)`; `pmSetEtwFlushPeriod` in the API).

### 1.4 How generated frames are detected, per vendor

Source: `PresentData/PresentMonTraceConsumer.cpp`, `ETW/Intel_PresentMon.h`,
`IntelPresentMon/CommonUtilities/mc/SwapChainState.cpp`, issues #388 and #178.

- **Mechanism:** the driver or SDK emits **Intel-PresentMon** events.
  - `PresentFrameType_Info` (`FrameId`, `FrameType` [, `AppFrameId`]) is emitted on the presenting
    thread and is matched to the next present on the same thread.
  - `FlipFrameType_Info` (`VidPnSourceId`, `LayerIndex`, `PresentId`, `FrameType` [, `TimeStamp`]) is
    emitted per flip and matched to the present through the DxgKrnl `MMIOFlipMultiPlaneOverlay3_Info`
    PresentId mapping. PresentMon turns these events on only after it sees an MPO3 event
    (`mEnableFlipFrameTypeEvents`).
  - A present can then carry several `Displayed` entries `(FrameType, screenTime)`, for example a
    generated entry followed by an app entry.
  - `IsAppFrameType_ = NotSet || Application`. Everything else counts as generated: it is included in
    FPS-Display and excluded from FPS-App and FrameTime-App.
- **Intel XeSS-FG:** tagged `Intel_XEFG` (2.3.0+). On generated frames PresentMon sets CPU
  Busy/Wait/FrameTime to 0 and aligns CPUStartQPC with the native frame (#388).
- **AMD AFMF / AFMF 2** (driver-level): tagged `AMD_AFMF` (2.3.0+). In early 2025 an Intel engineer
  wrote "AMD-FMF currently has a bug and is not reporting the frame type" (#388), so the driver side
  must be verified on current Adrenalin.
- **NVIDIA DLSS FG / MFG:**
  - **No FrameType.** Generated frames are real `Present` calls on the game's real swapchain, made
    by Streamline or the driver from a present thread in the game process.
  - Intel (#388): without `--track_frame_type`, "Intel-XeSS-FG frames will be reported similarly to
    DLSS and FSR-generated frames", that is as normal presents. #178: "PresentMon isn't able to
    distinguish between app-rendered frames and driver-generated frames" (FSR3 context).
  - What NVIDIA contributed instead: the `NV_DD` **FlipRequest** event, used for flip-metering delay
    so that **displayed** timing is right with DLSS 4 MFG on Blackwell. v2.6.0 also deploys an
    "NVIDIA custom flip ETW manifest" with the service.
  - App-frame identity is available only from **PCL markers** (see §1.5).
- **FSR 3 / 3.1 / 4 FG:**
  - The FidelityFX `FrameInterpolationSwapChain` is a proxy `IDXGISwapChain4`. Its present thread
    "presents the generated frame. It repeats this for the real frame", generated first, then real.
    https://gpuopen.com/manuals/fidelityfx_sdk2/techniques/frame-interpolation-swap-chain/
  - Both presents come from the same PID and thread on the same swapchain, and no marker is known.
    The typical present-to-present signature is a zig-zag of very short and long intervals (Unwinder
    in #178).
- **Lossless Scaling:** it "doesn't inject any code … works like an overlay" and captures the game
  window **[report]** (https://steamcommunity.com/app/993090/discussions/0/4039232337479089112).
  Generated frames are therefore presented by `LosslessScaling.exe` on its own swapchain, while the
  game PID keeps presenting at the base rate **[inference]**.
- **NVIDIA Smooth Motion** (driver FG, RTX 50 and since 2025 also RTX 40, DX11/DX12):
  - Nothing in PresentMon sources is specific to it.
  - FrameView 2.0 "accurately reports the frame generation multiplier when NVIDIA Smooth Motion is
    active".
  - RTSS 7.3.7 had to "ignore presentation calls for invisible windows" and use an alternate D3D12
    queue detection when the Smooth Motion hook module is loaded. This suggests an extra in-process
    module and possibly a hidden swapchain **[inference]**.
  - **The ETW signature is unknown and needs an ETL capture.**

  https://www.nvidia.com/en-us/geforce/technologies/frameview/release-notes/ ·
  https://www.guru3d.com/download/rtss-rivatuner-statistics-server-download/
- **Vendors with in-tree support:** Intel and AMD through FrameType; NVIDIA through FlipRequest
  (display timing) and PCL (app timing and latency).

  Files: https://github.com/GameTechDev/PresentMon/blob/main/PresentData/ETW/Intel_PresentMon.h ·
  https://github.com/GameTechDev/PresentMon/blob/main/PresentData/NvidiaTraceConsumer.cpp ·
  https://github.com/GameTechDev/PresentMon/issues/388 ·
  https://github.com/GameTechDev/PresentMon/issues/178

### 1.5 NVIDIA PCL Stats (key for DLSS FG app-FPS)

- **TraceLogging provider** `"PCLStatsTraceLoggingProvider"`, GUID
  `0d216f06-82a6-4d49-bc4f-8f38ae56efab`.
- **Events:**
  - `PCLStatsEvent` (Marker u32, FrameID u64), plus `PCLStatsEventV2` (+Flags) and
    `PCLStatsEventV3` (+Value i32);
  - `PCLStatsInput`, `PCLStatsInit`, `PCLStatsShutdown` and `PCLStatsFlags`.
- **Markers:**
  - SIMULATION_START 0, SIMULATION_END 1, RENDERSUBMIT_START 2, RENDERSUBMIT_END 3,
    PRESENT_START 4, PRESENT_END 5;
  - PC_LATENCY_PING 8, OUT_OF_BAND_PRESENT_START 11, CONTROLLER_INPUT_SAMPLE 13;
  - LATE_WARP_* 15–19, **VENDOR_INTERNAL_ASYNC_PRESENT_START/END 20/21**,
    **NUM_PRESENTS_IN_BATCH 22**.

  The last ones look exactly like the hooks for counting driver-generated presents per app frame
  **[inference]**: undocumented, verify with ETL.
- **Side effect of enabling the provider:**
  - The game's enable callback sets `g_PCLStatsEnable = true`.
  - A ping thread then fires every 100–300 ms. When the game is foreground, it posts the registered
    window message `"PC_Latency_Stats_Ping"` to the game window, or **synthesizes VK_F13–F15
    keypresses (optionally via `SendInput`)** if the game configured it.

  So enabling PCL is not purely passive. The effect is benign and by design (FrameView and PresentMon
  rely on it), but it should be documented and kept opt-in.
- **Licensing:** that header (`source/plugins/sl.pcl/pclstats.h`) is under a **proprietary NVIDIA
  license**, so do not copy it. PresentMon's MIT `Nvidia_PCL.h` restates the GUID and marker values.
  The GUID and enum values are facts we can use with attribution, following our rule of no
  proprietary headers.

  https://github.com/NVIDIA-RTX/Streamline/blob/main/source/plugins/sl.pcl/pclstats.h ·
  https://github.com/GameTechDev/PresentMon/blob/main/PresentData/ETW/Nvidia_PCL.h

### 1.6 Embedding options

| Option | How | Pros | Cons |
|---|---|---|---|
| **A. Console exe as child process** | oma-service spawns `PresentMon-2.6.0-x64.exe --output_stdout --no_console_stats --session_name <ours> --stop_existing_session --track_frame_type --track_pc_latency [--no_track_gpu --no_track_input] [--process_id N]` and parses CSV from stdout | Documented, stable CLI and CSV contract; Intel-signed binary (anti-cheat-friendly); 1 MB, no extra deps; MIT; no C++ build | One `StartTrace` per launch (EA AC pitfall, §2.4) unless kept running before the game starts; CSV text parsing (trivial at ≤ 1k rows/s); no provider toggling; idle cost if left running for all processes |
| **B. Private PresentMon service instance + PresentMonAPI2** | Run `PresentMonService.exe` with `--etw-session-name`, `--control-pipe \\.\pipe\…` and `--shm-name-prefix Global\…` (`CliOptions.h`; defaults `PMService`, `Global\pm_svc_shm`). Consume via P/Invoke: `pmOpenSessionWithPipe`, `pmStartTrackingProcess(pid)`, `pmRegisterFrameQuery` + `pmConsumeFrames`, or `pmRegisterDynamicQuery` + `pmPollDynamicQuery` with PM_STAT | Session created at startup with provider toggling (anti-cheat mitigation, 0.000 % idle per 2.6 notes); unified metrics calculator with percentiles; frame and dynamic queries | Large surface (vendor telemetry libs, shared memory, IPC rewrite in 2.5). README warns that a client's own copy of PresentMonAPI2.dll is not guaranteed binary-compatible with the service (fine only if we ship both together). Running outside SCM in a release build is **unverified**. 2.5.0 "shared service" conflict precedent (avoid by a private, non-registered instance). Must build from source or extract from the MSI |
| **C. Port PresentData to .NET (or Rust)** | Reimplement `PMTraceConsumer` (about 168 KB of C++ state machine) and the metrics calculator | Single process, full control, our IPC | Highest effort and correctness risk (display tracking, MPO, DWM, Win32k tokens, lost presents), ongoing chase of upstream fixes |
| A′. Static-link PresentData via a C shim | MSVC C++ lib + `extern "C"` wrapper + `cc`/cmake from Rust, or C++/CLI | In-proc | Our ETW must run in the LocalSystem service (.NET), so a native DLL shim P/Invoked from .NET; adds a C++ toolchain to the build |

---

## 2. ETW permissions, sessions and lifecycle

### 2.1 Who may do what **[fact, Microsoft Learn]**

- `StartTrace`: "Only users with administrative privileges, users in the Performance Log Users group,
  and services running as LocalSystem, LocalService, NetworkService can control event tracing
  sessions … Only users with administrative privileges and services running as LocalSystem can
  control an NT Kernel Logger session."
  https://learn.microsoft.com/windows/win32/api/evntrace/nf-evntrace-starttracew
- `EnableTraceEx2`: `ERROR_ACCESS_DENIED` unless admin, Performance Log Users, or
  LocalSystem/LocalService/NetworkService.
  https://learn.microsoft.com/windows/win32/api/evntrace/nf-evntrace-enabletraceex2
- Real-time consume (`EVENT_TRACE_LOGFILE`): the same set of principals, or a user granted
  `TRACELOG_ACCESS_REALTIME` via `EventAccessControl`.
  https://learn.microsoft.com/windows/win32/api/evntrace/ns-evntrace-event_trace_logfilew ·
  https://learn.microsoft.com/windows/win32/api/evntcons/nf-evntcons-eventaccesscontrol
- **Consequence:** the Tauri app (standard user) cannot run the session. Adding the user to
  Performance Log Users is a persistent privilege grant, needs a re-logon and is not appropriate.
  `EventAccessControl` could grant the app's user `TRACELOG_ACCESS_REALTIME` on a session the service
  owns, but that still exposes system-wide present data of all processes to the user. **Do it in
  oma-service (LocalSystem) and stream only aggregated or filtered results.** The PresentMon team
  made the same choice: 2.3.1 "Re-engineered multi-process architecture to enable PresentMon to be
  run without Administrator privileges" (privileged service + unprivileged client).

### 2.2 Limits and coexistence **[fact]**

- **Sessions:** a maximum of **64 logging sessions** on most systems (`EtwMaxLoggers`, 32–256, reboot
  required; "must not be automatically modified by a program"). Also 8 system loggers.
- **Per provider:** "Up to eight trace sessions can enable and receive events from the same modern
  (manifest-based or TraceLogging) provider". Only 1 for legacy MOF/WPP providers.
  - All providers above are modern, so PresentMon (`PresentMon`/`PMService`), CapFrameX (which
    drives PresentMon), FrameView (embedded PresentMon core), RTSS's PresentMonDataProvider, the
    Steam overlay (if it uses ETW) and our session can all coexist, as long as **each uses a
    distinct, case-insensitive session name**. The console's `--session_name` help says so
    explicitly.
  - Risk: more than 8 concurrent sessions enabling DxgKrnl gives `ERROR_NO_SYSTEM_RESOURCES` on
    enable; handle it gracefully.
- **Session name:** use a fixed, product-specific name such as `OpenMonitorAdvanced-Frames`. Never
  reuse `PresentMon` or `PMService`: `--stop_existing_session` would kill another tool's session.

### 2.3 Cleanup after a crash **[fact]**

- "If the application that started the session ends without disabling the provider, the provider
  remains enabled" (EnableTraceEx2 remarks). Real-time sessions are not tied to the controller
  process, so a crashed service leaves the session alive. It keeps its buffers (nonpaged memory for
  kernel providers such as DxgKrnl) and occupies one of the 64 slots.
- **Mitigation**, same as PresentMon's `StopNamedTraceSession` and `--stop_existing_session`:
  - at service start, `ControlTraceW(0, L"OpenMonitorAdvanced-Frames", props,
    EVENT_TRACE_CONTROL_STOP)`;
  - stop the session on clean shutdown and on SCM stop;
  - the installer's uninstall hook should also stop it (`logman stop OpenMonitorAdvanced-Frames
    -ets`).
- If the service uses the console exe (option A), use a **Job Object with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`** so the child dies with the service. The ETW session itself
  still survives a hard kill of the child, so stop it by name before relaunching.

### 2.4 Anti-cheat interaction **[fact]**

- Issue #573 (closed): "After EA AntiCheat starts, calls to StartTraceW() consistently fail with
  ERROR_ACCESS_DENIED … (e.g., Battlefield 6)". The fix: "the service will create the ETW session at
  startup (with no providers enabled), and Start Capture / Stop Capture will enable and disable the
  required provider set via EnableTraceEx2()". It shipped in 2.5.0 as "Anticheat Mitigation".
  https://github.com/GameTechDev/PresentMon/issues/573 ·
  https://github.com/GameTechDev/PresentMon/releases/tag/v2.5.0
- **Design consequence:** our service creates the session at boot or service start (cheap: no
  providers enabled means no events) and only toggles providers. With option A, this means the
  console must already be running before such a game starts.

---

## 3. .NET and Rust ETW options

- **Microsoft.Diagnostics.Tracing.TraceEvent:** MIT (microsoft/perfview), latest **3.2.8** on NuGet.
  - Pros: `TraceEventSession` for real-time, `DynamicTraceEventParser` (TDH-based) for manifest
    providers and TraceLogging self-describing events.
  - It has no typed parser for DxgKrnl/DXGI, so these go through dynamic parsing (slower,
    allocation-heavy at thousands of events/s) or raw `UserData` offsets.
  - The package carries native helpers (KernelTraceControl, msdia) used for kernel/rundown and
    symbol features. They are not needed for our use, but they add weight.
  - **Trimming/AOT compatibility is unverified**. Our service checks trim warnings
    (`scripts/check-trim-warnings.ps1`), so this is a probable blocker to evaluate.
- **Hand-written P/Invoke** (advapi32 `StartTraceW`, `EnableTraceEx2`, `ControlTraceW`,
  `OpenTraceW`, `ProcessTrace`, `CloseTrace`; tdh `TdhGetEventInformation` only where layouts vary):
  matches our FFI conventions (hand-written bindings, size asserts), is trim-safe and has zero
  dependencies. PresentMon itself mixes raw struct casts (Intel provider, which has no manifest) with
  TDH property lookups by name (`mMetadata.GetEventData<T>(rec, L"field")`) for OS events whose
  layouts change across Windows builds.
- **Rust `ferrisetw`:** MIT OR Apache-2.0, **1.2.0 (2024-06-27)**; repo last pushed 2025-10-20,
  94 stars, about 34 open issues. It describes itself as "still WIP" (parsing limited to `TryParse`
  types); it is built on `windows-rs` and has a TDH schema cache. It is moderately maintained. It is
  irrelevant to the unprivileged app anyway: ETW must live in the LocalSystem service, which is .NET.
  https://github.com/n4r1b/ferrisetw · https://crates.io/crates/ferrisetw
- **`one_collect`** (Microsoft, Rust, MIT): "pre-release … breaking API and behavioral changes"
  expected. Not for production. https://github.com/microsoft/one-collect
- **Prior art in Rust:** `kewuamigo/glint-overlay` (MIT, 2026, 0 stars) uses ferrisetw.
  - It counts DXGI Present_Start (id 42), DxgKrnl flips (ids 116/166/168/184) and PresentHistory
    (171/172), and reads Intel-PresentMon FrameType raw bytes (offset 4 for PresentFrameType,
    offset 16 for FlipFrameType).
  - It detects FG **heuristically** from rate ratios with hysteresis. Useful as a cautionary
    reference: heuristics, not attribution.

  https://github.com/kewuamigo/glint-overlay/tree/HEAD/core/metrics/metrics-etw
- **Reimplement vs ship PresentMon:**
  - Rendered/presented FPS alone (count DXGI/D3D9 `Present_Start` per PID and swapchain) is easy,
    perhaps 300 lines.
  - **Displayed** FPS and frame-type attribution are the hard part: PresentHistory tokens, flip and
    MPO paths, DWM composition, lost presents, FlipFrameType deferral, NVIDIA flip delay. PresentMon
    keeps fixing bugs there in every release (2.4.0 "Fixed frame type bug when processing composed
    presents", "lost presents" handling, Streamer ring-buffer loss).
  - Since displayed FPS is the headline number with FG, **ship PresentMon** and do not port it until
    a concrete limitation forces us to.

---

## 4. How other tools do it

| Tool | Mechanism | Rendered vs displayed under FG |
|---|---|---|
| **Steam overlay "Performance Monitor"** (beta 2025-06-17/19; extended Aug 2025) | Steam's overlay is injected in-process (GameOverlayRenderer). Valve has not published how FG is detected **[inference: in-process, e.g. Streamline/FidelityFX awareness]** | Shows "FPS" (real game frames) and a separate counter labeled **"DLSS"** or **"FSR"** with output FPS including generated frames; 1-s intervals plus slowest/fastest frame. **XeSS FG not detected** (Valve: "XeSS support might come later"); not a universal detector. Sources: https://www.kitguru.net/gaming/joao-silva/steam-fps-overlay-can-now-detect-frame-generation/ · https://www.itechguides.com/real-or-fake-frames-steams-new-overlay-detects-dlss-and-fsr-frame-generation-with-limits/ · https://www.thefpsreview.com/2025/06/19/steam-introduces-frame-generation-monitoring-in-its-in-game-fps-overlay/ · https://gigazine.net/gsc_news/en/20250818-steam-performance-overlay |
| **RTSS / MSI Afterburner** (7.3.7, 2025-09-30) | Hooks Present in-process for the overlay and limiter. Optional **PresentMonDataProvider** (PresentMon console/service V2) for ETW metrics. Reflex marker functions `reflexlatency()`, `presentmonlatency()` | The native counter counts what its hook sees, which in practice includes generated frames **[report]**. 7.3.7 adds a layout with "frametimes at both rendering pipeline input (present-to-present) and … output (display-to-display)", and a custom `PresentMon-2.3.1-x64-DLSS4.exe` "to monitor framepacing performed by flip metering unit in DLSS4". Smooth Motion compatibility fixes. https://www.guru3d.com/download/rtss-rivatuner-statistics-server-download/ |
| **NVIDIA FrameView** (1.7 Jan 2026, 1.8, 2.0) | Embeds an (old) PresentMon core (GamersNexus: 1.10.0) plus NVIDIA driver data | Distinguishes Rendered FPS and Displayed FPS (MsBetweenDisplayChange recommended). 1.8 reports the **DLSS MFG multiplier** in overlay and logs ("currently only … NVIDIA's DLSS Multi-Frame Generation"). 2.0 adds the Smooth Motion multiplier and a **separate-window overlay** that needs MPO (else may force games out of iFlip, adding latency; BF6 anti-cheat incompatible). https://www.nvidia.com/en-us/geforce/technologies/frameview/release-notes/ · https://gamersnexus.net/gpus/fake-frames-tested-dlss-40-mfg-4x-nvidias-misleading-review-guide |
| **CapFrameX** (MIT) | Drives the PresentMon console (ETW) for capture; overlay via RTSS | Inherits PresentMon behaviour; #388 shows it consumes v2 metrics and FrameType. https://github.com/CXWorld/CapFrameX |
| **Intel PresentMon app** | Service (ETW) plus a separate D3D11 overlay window in a high Z-band (uiAccess) | FPS-Display / FPS-App / FPS-Presents; FrameType for XeFG and AFMF |
| **AMD Adrenalin overlay** | Driver-side | Reports of FPS metrics disappearing with AFMF (community thread); AMD drivers emit Intel-PresentMon FrameType for AFMF. https://community.amd.com/t5/pc-drivers-software/amd-software-performance-metrics-broken-with-afmf/m-p/671873/highlight/true |
| **NVIDIA app overlay** | Driver/NVAPI-side | Not researched in depth: no public mechanism docs found |

**Visibility of DLSS FG frames:**

- An in-process hook on the real DXGI `Present` sees generated frames, because Streamline or the
  driver calls the real swapchain once per output frame. A hook on the proxy swapchain the game calls
  sees app frames only **[inference]**.
- ETW sees all real presents, which is the same as the hook on the real swapchain; generated frames
  are therefore counted unless tagged or derived from PCL.

---

## 5. Standard derived metrics: exact definitions

Let `ft_i` be per-frame frametime in ms over a window of N frames, and `T = Σ ft_i`.

- **Average FPS** = `1000·N / T` = `1000 / mean(ft)`. It is time-weighted; never average the
  instantaneous `1000/ft_i`. CapFrameX and PresentMon both compute it this way.
- **1% low.** Three definitions are in use, so always label which one:
  1. **Percentile (P1 FPS)** = 1st percentile of instantaneous FPS = `1000 / P99(ft)`. Used by
     CapFrameX "P1" (`GetPQuantileSequence(fps, 0.01)`) and by the PresentMon API
     `PM_STAT_PERCENTILE_01`.
  2. **1% low average** = `1000 / mean(worst 1 % of frametimes by count)`. CapFrameX
     `OnePercentLowAverage`; it was CapFrameX's old "1% low".
  3. **1% low integral** (MSI Afterburner, current CapFrameX "1% low"):
     - sort frametimes in descending order;
     - accumulate until the sum reaches 1 % of total time T;
     - the frametime where the sum crosses that threshold gives `1000/ft`.

     It is time-based, so long frames weigh more.

  CapFrameX notes: "instead of calculating the value at the border … (P1), we're adding the frametime
  of all lowest frames until they reach a sum of 1% of the total benchmark time … comparable to MSI
  Afterburner". The **0.1% low** is the same with 0.001.
  https://www.capframex.com/news/detail/New%20version%201.5.3 ·
  https://github.com/CXWorld/CapFrameX/blob/master/source/CapFrameX.Statistics.NetStandard/FrametimeStatisticProvider.cs
- **Frametime graph:** plot per-frame `ft`. Under FG, show **displayed frametime**
  (MsBetweenDisplayChange, which is what the eye sees) and optionally **app frametime** (FrameTime-App
  or MsBetweenSimulationStart). Presented frametime (MsBetweenPresents) zig-zags with FSR FG and is
  misleading (Unwinder in #178).
- **Stutter:**
  - CapFrameX counts a frame as a stutter if `ft_i > 2.5 × average(ft)` (NVIDIA's driver team uses
    3×). It reports `StutteringCount%` and `StutteringTime% = Σ stutter ft / T`
    (`GetStutteringCountPercentage` / `GetStutteringTimePercentage`).
  - For a live overlay, use a **rolling median** over 1–2 s instead of the global average, plus a
    minimum absolute delta (for example > 8 ms) to avoid false positives at high FPS.
    **[recommendation]**
- **Latency** (PresentMon definitions in §1.3):
  - DisplayLatency and MsUntilDisplayed are available without instrumentation.
  - MsPCLatency (input → display) needs PCL/Reflex. MsClickToPhotonLatency is an estimate from
    Win32k input events.
  - Under FG, latency belongs to **app frames**: generated frames add display smoothness, not
    responsiveness.
- **FG multiplier** = displayed FPS ÷ app FPS over the same window (FrameView 1.8's "MFG ×N").

---

## 6. Which game to show

- **Foreground detection must run in the user session (the app)**, not in the service. oma-service
  runs in session 0 and cannot see the interactive desktop.
  - The app polls `GetForegroundWindow` → `GetWindowThreadProcessId` (or uses a
    `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` callback, which costs no CPU when idle).
  - It sends the target PID to the service; the service tracks only that PID's swapchains.
  - Exception: for UWP/Store games the foreground window belongs to `ApplicationFrameHost.exe`.
    Resolve the child CoreWindow's PID instead **[known Windows behaviour]**.
- **Multiple swapchains per PID** (menus, secondary windows, Smooth Motion's possible hidden
  swapchain): choose the swapchain with the most **displayed** frames in the last second. PresentMon
  reports per `SwapChainAddress`, and the API returns `numSwapChains`.
- **Launchers:** the launcher PID is never foreground once the game window is up, so
  foreground-driven selection handles launcher → game handoff. Keep a short grace period (2–3 s)
  before dropping a target, to survive alt-tab and loading screens.
- **Exclusions:**
  - our own process tree, because the overlay's WebView2 presents and must not measure itself;
  - `dwm.exe` and known overlays/capture tools.
- **Lossless Scaling:** when `LosslessScaling.exe` has an active swapchain and the foreground PID is
  a game, pair them:
  - rendered = game PID's app FPS;
  - displayed = LS PID's displayed FPS;
  - label "LSFG ×N".
- **Anti-cheat safety:**
  - ETW consumption is passive: no DLL injection, no game memory access, no handle to the game
    process required. Process names can come from the Kernel-Process events or
    `QueryFullProcessImageNameW` with `PROCESS_QUERY_LIMITED_INFORMATION`.
  - PresentMon's README positions this as the advantage of ETW.
  - Caveats: the EA Javelin `StartTrace` denial (§2.4); PCL enabling triggers the game's ping thread
    (§1.5). An overlay window, unlike injection, is generally safe, but FrameView 2.0 lists "Battlefield
    6 anticheat" as incompatible with its new window overlay.
  - We should never inject. The overlay must be a separate topmost, click-through window.
  - **Exclusive-fullscreen** games will cover a normal topmost window. PresentMon solves this with
    `uiAccess=true` + `CreateWindowInBand`, which requires a signed binary in Program Files; this
    touches our M6a signing work.

---

## 7. Data volume and CPU cost

- **Events per frame, display tracking without GPU tracking** **[estimate from the enabled event
  list]**:
  - DXGI Present Start/Stop (2), DxgKrnl PresentHistory Start/Info (2), Present_Info (1),
    QueuePacket Start/Stop (2–3), Flip/MMIOFlip/MPO (1–2);
  - windowed: + DWM (2–4) and Win32k tokens (2);
  - plus VSyncDPC about one per vblank per display.

  That is about 10–15 events per frame. At 240 FPS, about 3–4 k events/s plus about 150–250
  VSync/s. At about 100–200 B per event, under 1 MB/s through ETW buffers.
- **With GPU tracking** (DmaPacket per submission): one to two orders of magnitude more events in
  heavy DX12 titles. **Turn GPU tracking off by default** (`--no_track_gpu`). We do not need GPUBusy
  for an FPS overlay, and GPU utilisation already comes from our D3DKMT/PDH providers.
- **Measured references:**
  - PresentMon 2.6.0 cut service CPU "under load by 78 %" by moving ETW flush and related loops from
    high-precision timers to coarser Sleep, and idle CPU from 0.003 % to 0.000 % (no flushing when
    idle).
  - A classic, old figure (around 2000s hardware): ETW overhead "about 5 percent of CPU to log 20,000
    events per second" (ITPro Today). It is an upper bound only and not representative of modern
    CPUs.
  - Nothing official exists for per-frame overhead on the game threads. The ETW write path is a few
    hundred ns per event.

  https://github.com/GameTechDev/PresentMon/releases/tag/v2.6.0 ·
  https://www.itprotoday.com/windows-78/inside-event-tracing-windows
- **Memory:** PresentMon uses 64 KB × 256 buffers (about 16 MB). For our session, 64 KB × 16–32
  minimum buffers is likely enough at ≤ 5 k events/s with a 100 ms flush **[to tune]**. Watch the
  `EtwBuffersLost` column (CSV) or the `EventsLost` and `BuffersLost` counters on
  `EVENT_TRACE_PROPERTIES`.
- **Budget fit** (`docs/perf-budget.md`: core < 1 % CPU at idle):
  - **Idle (no game, overlay off):** the session exists, no providers are enabled, no consumer
    thread wakes. Zero cost.
  - **Overlay on but no game foreground:** providers off. Enable them only when the app reports a
    foreground PID that presents (cheap probe: enable DXGI `Present_Start` alone, filtered to that PID
    with `EVENT_FILTER_TYPE_PID`, which allows up to 8 PIDs).
  - **While gaming:** a realistic target is ≤ 0.5–1 % of one core for consumer plus aggregation;
    measure it with `scripts/measure-footprint.ps1`.
  - With option A, the console exe processes all processes unless `--process_id` is given, and the
    PID filter forces a restart per target. Measure both variants.

---

## 8. Recommended architecture

### 8.1 Who runs ETW

- **oma-service (LocalSystem)** owns everything ETW. The Tauri app stays unprivileged and only:
  1. reports the foreground PID;
  2. subscribes to frame stats;
  3. renders the overlay window.
- **MVP engine = option A (PresentMon console exe, Intel-signed, MIT), hosted by the service:**
  - Bundle `PresentMon-2.6.0-x64.exe` in the installer with a pinned SHA-256, like
    `pawnio.sha256`, plus the MIT notice.
  - Launch it **when the FPS overlay feature is enabled**, not per game, so it is already running
    before EA-AC-type games start (§2.4).
  - Flags: `--session_name OpenMonitorAdvanced-Frames --stop_existing_session --output_stdout
    --no_console_stats --track_frame_type --no_track_gpu --no_track_input --qpc_time`. Add
    `--track_pc_latency` only as an opt-in, because of the PCL side effect (§1.5).
  - Choose v1-style columns (`MsBetweenPresents`, `MsBetweenDisplayChange`, `MsUntilDisplayed`,
    `MsBetweenSimulationStart`, `MsPCLatency`) or `--v2_metrics` (CPUStart, FrameTime, DisplayedTime,
    AnimationError…). **The spike decides which column set we parse;** the header is self-describing,
    so parse by column name.
  - Put the child in a kill-on-close Job Object. Stop the named session on service start and stop.
- **Upgrade path = option B** (private `PresentMonService.exe` instance with our pipe, session and
  shm names, consumed via PresentMonAPI2 frame queries) if the spike shows:
  1. unacceptable idle cost of the always-on console;
  2. the need for provider toggling;
  3. that the console's per-launch `StartTrace` breaks with anti-cheat.
- **Option C (port)** only if A and B both fail.

### 8.2 How data reaches the overlay "at frame rate"

- The overlay does not need per-frame push. The service keeps a per-target ring of per-frame records:
  QPC, flags, frame type, app/generated, displayed, `ft_presented`, `ft_displayed`, `ft_app`, and
  latency where available.
- Every **50–100 ms (10–20 Hz)** the service sends one message:
  - all new frame records since the last push (≤ about 30 records at 240 FPS, a few hundred bytes in
    MessagePack);
  - rolling aggregates over 1 s for the counters (rendered FPS, displayed FPS, multiplier) and
    1–10 s for 1 %/0.1 % lows and stutter count.
- This fits our existing MessagePack pipe and the protocol conventions: new message types, no
  `skip_serializing_if`, fixtures. The overlay window then draws the frametime graph at its own
  cadence (our chart FPS setting 15/30/60) from the batched records. Text counters refresh at
  2–4 Hz, which is readable and what Steam (1 s) and Afterburner do.
- **ETW flush period:** call `ControlTrace` FLUSH every about 100 ms (option B: `pmSetEtwFlushPeriod`;
  option A: the console flushes internally) to keep end-to-end lag around 100–150 ms.

### 8.3 How rendered vs displayed FPS are computed (per target swapchain, rolling window W)

- **Displayed FPS** = `1000 · N_displayed / Σ MsBetweenDisplayChange` over displayed frames in W,
  counting both app and generated frames. Dropped frames (not displayed) are excluded. NVIDIA
  FlipDelay is already applied by PresentMon.
- **Presented FPS** = Present() calls per second (diagnostic only).
- **Rendered (app) FPS**, with a cascade by evidence quality and the chosen source shown as a badge:
  1. **FrameType-tagged** (XeSS-FG, AFMF): app frames are `Application`/`NotSet`, generated frames are
     `Intel XeSS-FG`/`AMD AFMF`. Rendered FPS = app frames / time. **Exact.**
  2. **PCL present** (DLSS FG/MFG, any Reflex title, opt-in): rendered FPS = number of distinct PCL
     FrameIDs with PRESENT_START (or SIMULATION_START) per second, read from
     `MsBetweenSimulationStart` on rows that carry it, or from a small own TraceLogging consumer of
     the PCL provider in a second session. **Near-exact; verify on the RTX 4080 with DLSS FG.**
  3. **Separate FG process** (Lossless Scaling): rendered = game PID presents, displayed = LS PID
     displayed. **Exact by construction.**
  4. **No evidence** (FSR FG, Smooth Motion until verified): show displayed FPS only, and mark
     "FG ?" if a heuristic fires, for example a strict alternating short/long present pattern with a
     ratio ≥ 1.8. Never present a heuristic number as rendered FPS without a badge.
- **FG multiplier** = displayed ÷ rendered, rounded to the nearest 0.5 only for the label.
- **Lows and stutter:** compute separately on displayed frametimes (smoothness) and app frametimes
  (responsiveness).

### 8.4 Spike checklist (before writing the plan)

1. On this PC (RTX 4080 + AMD iGPU), capture ETL with
   `PresentMon --track_frame_type --track_pc_latency` and xperf/WPR including the providers in §1.2
   for:
   - a DLSS FG title;
   - an FSR 3.1 FG title;
   - Smooth Motion on;
   - Lossless Scaling;
   - AFMF on the iGPU, if supported.

   Check: per-present FrameType, PCL FrameID cadence, NUM_PRESENTS_IN_BATCH/VENDOR_INTERNAL markers,
   swapchain count per PID.
2. Measure option A at idle (overlay on, no game) and in game with `measure-footprint.ps1`.
3. Verify whether `PresentMonService.exe` runs as a console child under LocalSystem with custom
   names, for option B.
4. Check that `--stop_existing_session` with our name never touches other tools' sessions, and that
   CapFrameX, FrameView and RTSS run concurrently.
5. Overlay window: MPO and iFlip behaviour (FrameView 2.0 caveat), exclusive fullscreen
   (uiAccess question), anti-cheat titles.

---

## 9. Source index

- PresentMon repo and README: https://github.com/GameTechDev/PresentMon
- Releases (2.3.0–2.6.0 notes quoted above): https://github.com/GameTechDev/PresentMon/releases
- Console README and CLI: https://github.com/GameTechDev/PresentMon/blob/main/README-ConsoleApplication.md ·
  https://github.com/GameTechDev/PresentMon/blob/main/PresentMon/CommandLine.cpp
- Service and SDK: https://github.com/GameTechDev/PresentMon/blob/main/README-Service.md ·
  https://github.com/GameTechDev/PresentMon/blob/main/IntelPresentMon/PresentMonService/CliOptions.h ·
  https://github.com/GameTechDev/PresentMon/blob/main/IntelPresentMon/PresentMonAPI2/PresentMonAPI.h
- Metric definitions: https://github.com/GameTechDev/PresentMon/blob/main/IntelPresentMon/metrics.csv
- Trace session and consumer: https://github.com/GameTechDev/PresentMon/blob/main/PresentData/PresentMonTraceSession.cpp ·
  https://github.com/GameTechDev/PresentMon/blob/main/PresentData/PresentMonTraceConsumer.cpp ·
  https://github.com/GameTechDev/PresentMon/blob/main/PresentData/PresentEventEnums.hpp
- Provider headers: https://github.com/GameTechDev/PresentMon/tree/main/PresentData/ETW ·
  Intel-PresentMon provider: https://github.com/GameTechDev/PresentMon/blob/main/Provider/README.md
- Issues: #388 (frame-gen detection scope) https://github.com/GameTechDev/PresentMon/issues/388 ·
  #178 (FSR3) https://github.com/GameTechDev/PresentMon/issues/178 ·
  #573 (EA AntiCheat StartTrace) https://github.com/GameTechDev/PresentMon/issues/573
- PCL Stats header (proprietary, reference only): https://github.com/NVIDIA-RTX/Streamline/blob/main/source/plugins/sl.pcl/pclstats.h
- FidelityFX FI swapchain: https://gpuopen.com/manuals/fidelityfx_sdk2/techniques/frame-interpolation-swap-chain/
- ETW docs: https://learn.microsoft.com/windows/win32/api/evntrace/nf-evntrace-starttracew ·
  https://learn.microsoft.com/windows/win32/api/evntrace/nf-evntrace-enabletraceex2 ·
  https://learn.microsoft.com/windows/win32/api/evntcons/nf-evntcons-eventaccesscontrol ·
  https://learn.microsoft.com/windows/win32/etw/configuring-and-starting-an-event-tracing-session ·
  https://learn.microsoft.com/windows/win32/etw/controlling-event-tracing-sessions
- ferrisetw: https://github.com/n4r1b/ferrisetw · one-collect: https://github.com/microsoft/one-collect ·
  glint-overlay: https://github.com/kewuamigo/glint-overlay
- Steam: https://www.kitguru.net/gaming/joao-silva/steam-fps-overlay-can-now-detect-frame-generation/ ·
  https://www.itechguides.com/real-or-fake-frames-steams-new-overlay-detects-dlss-and-fsr-frame-generation-with-limits/ ·
  https://hothardware.com/news/steam-new-overlay-performance-monitor
- RTSS 7.3.7: https://www.guru3d.com/download/rtss-rivatuner-statistics-server-download/
- FrameView: https://www.nvidia.com/en-us/geforce/technologies/frameview/release-notes/ ·
  GamersNexus MFG article: https://gamersnexus.net/gpus/fake-frames-tested-dlss-40-mfg-4x-nvidias-misleading-review-guide
- CapFrameX: https://www.capframex.com/news/detail/New%20version%201.5.3 · https://github.com/CXWorld/CapFrameX
- Smooth Motion on RTX 40: https://www.club386.com/nvidia-brings-driver-level-frame-generation-to-rtx-40-gpus/
