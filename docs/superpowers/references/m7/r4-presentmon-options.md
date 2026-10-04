# PresentMon integration: service + PresentMonAPI2 (option 2) vs console exe + CSV (option 1)

Research date: 2026-10-04. Source: PresentMon tag **v2.6.0** (commit e13fce6, 2026-08-28), shallow clone in
`scratchpad/presentmon-src`. Release assets downloaded to `scratchpad/pm-dl`; the MSI was unpacked with an
administrative install (`msiexec /a ... /qn TARGETDIR=...`, exit 0, no admin rights needed) into
`scratchpad/pm-dl/msi-admin`. Paths below are relative to the clone unless stated.

---

## 1. Release assets (v2.6.0, published 2026-09-21)

| Asset | Size | Authenticode |
|---|---|---|
| `PresentMon-2.6.0-x64.exe` (console app) | 980,320 B | **Valid**, CN=Intel Corporation (Sectigo Public Code Signing CA R36, Sectigo timestamp) |
| `PresentMon-2.6.0.msi` (full Intel PresentMon: UI + CEF + shared service + SDK) | 169,672,704 B | **Valid**, Intel Corporation |
| `ReleaseSymbols.zip` | 54 MB | n/a |

There is **no standalone API DLL and no SDK zip**. The service and API DLL ship only inside the MSI. After `msiexec /a`
the shared-service payload is:

| File (in `Intel\PresentMonSharedService\`) | Size | Signature |
|---|---|---|
| `PresentMonService.exe` | 3,307 KB | Valid, Intel Corporation |
| `PresentMonAPI2.dll` | 1,962 KB | Valid, Intel Corporation |
| `ddETWExternal.xml` (NVIDIA DisplayDriver ETW manifest) | 11 KB | (xml, unsigned) |

The SDK folder adds `PresentMonAPI.h`, `PresentMonAPI2Loader.dll/.lib` (not needed). The console exe is also inside the
MSI (`PresentMonConsoleApplication\PresentMon-2.6.0-x64.exe`). No UCI files are in the MSI.

PE imports (parsed from the binaries): all three use only system DLLs, static CRT (no VC++ redistributable).
- console exe: KERNEL32 (+ delay-load ADVAPI32, SHELL32, tdh, USER32)
- `PresentMonService.exe`: tdh, KERNEL32, USER32, ADVAPI32, WS2_32, SETUPAPI, VERSION, pdh, ole32, OLEAUT32
- `PresentMonAPI2.dll`: KERNEL32, ADVAPI32, WS2_32

`PresentMonAPI2.dll` exports 32 unmangled C functions (`pmOpenSession`, `pmConsumeFrames`, ...) plus 14 mangled C++
internals (cereal statics, `pmLinkLogging_`, ...) that we would not touch.

---

## 2. Option 2: service + PresentMonAPI2

### a. Running `PresentMonService.exe` outside SCM

**Yes, it is a supported (developer) mode, also in release builds.**
- `IntelPresentMon/PresentMonService/ServiceMain.cpp:72-89`: `_tmain` calls `StartServiceCtrlDispatcher`; on
  `ERROR_FAILED_SERVICE_CONTROLLER_CONNECT` it calls `CommonEntry(argc, argv, true)` (app mode). No build-config gate.
- App mode runs `ConsoleDebugMockService` (`Service.cpp:205-259`), stop = `CTRL_C_EVENT`/`CTRL_CLOSE_EVENT` only
  (`Service.cpp:232-245`).
- `CliOptions.h:18-20`: `--etw-session-name` (default `PMService`), `--control-pipe` (default
  `\\.\pipe\sharedpresentmonsvcnamedpipe`, `GlobalIdentifiers.h:5`), `--shm-name-prefix` (default `Global\pm_svc_shm`).
  Also `--frame-ring-samples`, `--telemetry-ring-samples`, logging options, `--enable-test-control`.
- Intel itself uses this: `IntelPresentMon/KernelProcess/winmain.cpp:384-427` (`--svc-as-child`) launches
  `PresentMonService.exe --control-pipe ... --shm-name-prefix ... --etw-session-name ...` and waits for
  `<pipe>-in`. Defaults for the child: `\\.\pipe\pm-ctrl`, `pm-child-shm`, `pm-child-etw-session`
  (`Core/source/cli/CliOptions.h:41-43`). BUILDING.md calls it the "typical development" path.
- Verified live with the Intel-signed 2.6.0 binary (unelevated, custom names): it starts, prints "Running service as a
  console app...", creates its shm (`oma-research-shm_<salt>_int`) and the control pipe; idle 13 MB working set,
  0.016 s CPU in 70 s, 9 threads (ETW session could not start unelevated, as expected).

Quirks found (evidence):
- **`--timed-stop` does not work in app mode**: it uses a waitable-timer APC (`PMMainThread.cpp:220-236`) but the main
  thread waits non-alertably; the live test kept running past 4 s and had to be killed.
- **Graceful stop** from a service parent: no console to send Ctrl+C to. Options: `--enable-test-control` and write
  `%ping` then `%quit` to stdin (`testing/TestControl.cpp:20-34`, compiled into release), or TerminateProcess (job
  object) and let the next start clean the stale session (`RealtimePresentMonSession.cpp:184-191` stops a same-named
  session and retries) or stop it ourselves with `ControlTraceW(EVENT_TRACE_CONTROL_STOP)`.
- **Registry side effect**: even in app mode it opens or **creates** `HKLM\SOFTWARE\INTEL\PresentMon\Service`
  (`CommonUtilities/reg/Registry.h:154-183`, `Registry.h` in the service: `logLevel`, `logDir`, `frameRingSamples`...).
  Unelevated it logs "Failed to create registry key"; as LocalSystem it will leave an empty key. If the official
  PresentMon is installed, our private instance also reads Intel's `logLevel`/`logDir` values (CLI options win for ring
  sizes, `PMMainThread.cpp:200-207`).

Runtime needs besides the exe:
- `PresentMonAPI2.dll` (in our process, not the service's).
- `ddETWExternal.xml` next to the exe: loaded with `TdhLoadManifest` (process-local, **no wevtutil / registration**)
  in `PMMainThread.cpp:33-56`; failure is only logged. MSI ships it in the service folder
  (`PMInstallerLib/Library.wxs:53-66`).
- Vendor telemetry libraries are **not shipped**: ADL/IGCL/NVAPI/NVML are loaded at runtime from System32
  (`ControlLib/DllModule.h:19-25`, `LOAD_LIBRARY_SEARCH_SYSTEM32`; IGCL via `igcl/cApiWrapper.cpp:66-130`). UCI is an
  optional private SDK (`ControlLib/uci/UciSdk.h`, `Common.props:10-12`), absent from the MSI; missing = provider skipped.
- MSI custom actions: only `TryStartSharedService` (`sc start`) and the SCM registration (`Library.wxs:25-37,71-82`).
  The `Intel-PresentMon` ETW provider manifest (`Provider\Intel-PresentMon.man`) is registered by the full MSI, but
  `Provider/README.md` states registration is **not required** for PresentMon to consume the events (only for WPA/gpuview).
- Privileges: StartTrace needs admin/Performance Log Users (LocalSystem is fine); `Global\` shm needs
  SeCreateGlobalPrivilege (LocalSystem has it; with a custom non-Global prefix it does not matter).

### b. Turning telemetry off

**Not possible via CLI or API.** `TelemetryThreadEntry_` (`PMMainThread.cpp:113-181`) waits for the first client
session, then constructs `TelemetryCoordinator`, which always tries WMI (+PDH counters), UCI, ADL, IGCL, NVAPI, NVML
providers (`ControlLib/TelemetryCoordinator.cpp:293-340`). Only `--enable-mock-telemetry` exists. Consequences:
- one-off vendor-library initialisation in the child on every session open (duplicates what oma-win does, but no
  polling yet). `pmOpenSession` also waits for introspection readiness, i.e. for this enumeration (release notes 2.6.0).
- **Polling** happens only for metrics with a device id != 0 (`PollToIpc` skips device 0, `TelemetryCoordinator.cpp:232-235`),
  so a frame query of frame metrics (device 0) polls no hardware. But any registered query makes the usage map non-empty
  (`Middleware.cpp:395-432`, `ActionExecutionContext.cpp:66-77`), so the telemetry loop wakes every
  `gpu_telemetry_period_ms_` = **16 ms** by default (`PresentMonSession.h:62`) doing nothing. Mitigation:
  `pmSetTelemetryPollingPeriod(session, 0, 5000)` (max 5000, `PresentMonAPI.h:433-434`) -> 0.2 Hz wakeups.

### c. PresentMonAPI2 surface we would need

Header: `IntelPresentMon/PresentMonAPI2/PresentMonAPI.h` (468 lines), API version 3.4. Pure C ABI: `extern "C"`
(lines 17-19), plain enums, POD structs, opaque handles (`typedef struct PM_SESSION* PM_SESSION_HANDLE`). **No C++ types
in the public ABI.**

Functions to P/Invoke (10, of 32 exported):
`pmOpenSessionWithPipe(PM_SESSION_HANDLE*, const char* pipe)`, `pmCloseSession`, `pmStartTrackingProcess(h, pid)`,
`pmStopTrackingProcess`, `pmSetEtwFlushPeriod(h, ms)` (8..1000 ms; `0` = service default = **no manual flush**,
`RealtimePresentMonSession.cpp:138-141` + `PMMainThread.cpp:97-108`), `pmSetTelemetryPollingPeriod`,
`pmRegisterFrameQuery(h, &q, PM_QUERY_ELEMENT*, n, uint32_t* blobSize)`, `pmConsumeFrames(q, pid, uint8_t* blobs, uint32_t* n)`,
`pmFreeFrameQuery`, `pmGetApiVersion(PM_VERSION*)`. Optional: `pmFlushFrames`, `pmRegisterDynamicQuery`/`pmPollDynamicQuery`/`pmFreeDynamicQuery`.

Structs: `PM_QUERY_ELEMENT` {metric, stat, deviceId, arrayIndex, uint64 dataOffset, uint64 dataSize} = 32 bytes;
`PM_VERSION` (3x u16 + char[22] + char[8] + char[4]). Enums: `PM_STATUS`, subset of `PM_METRIC`, `PM_STAT`,
`PM_FRAME_TYPE` {NOT_SET, UNSPECIFIED, APPLICATION, REPEATED, INTEL_XEFG=50, AMD_AFMF=100}.

Frame metrics (all `PM_STAT_NONE`, deviceId 0): `PM_METRIC_CPU_START_QPC`, `PM_METRIC_FRAME_TYPE`,
`PM_METRIC_DISPLAYED_TIME`, `PM_METRIC_BETWEEN_DISPLAY_CHANGE`, `PM_METRIC_BETWEEN_PRESENTS`,
`PM_METRIC_CPU_FRAME_TIME` / `PM_METRIC_BETWEEN_APP_START`, `PM_METRIC_DROPPED_FRAMES`, `PM_METRIC_PC_LATENCY`,
`PM_METRIC_FLIP_DELAY` (`metrics.csv:12-106`). Blob decoding by `dataOffset` (see `SampleClient/FrameQuerySample.h`).

Dynamic query stats: AVG, P99, P95, P90, P01, P05, P10, MIN, MAX... (`PresentMonAPI.h:213-231`). **No 0.1 %
percentile**, so 0.1 % lows must be computed by us from frame data anyway (same code as option 1); the dynamic query
adds nothing essential.

Binary compatibility:
- **DLL and service must be from the same build**: the client sends its build id, the service returns its own; any
  mismatch of build hash or config throws `PM_STATUS_MIDDLEWARE_SERVICE_MISMATCH`
  (`PresentMonMiddleware/ActionClient.h:38-53`). README-Service.md: "If a client ships their own copy of
  PresentMonAPI2.dll, binary compatibility with the service will not be guaranteed." Shipping the pair from the same
  MSI satisfies this; pairing our DLL with an installed official service does not.
- Enum ordinals are not strictly append-only: 2.6.0 inserted `PM_METRIC_PSO_COMPILE_*` **before**
  `PM_METRIC_PROCESS_ID` (header diff 2.5.1 -> 2.6.0). The metrics we need sit below index 140 and did not move
  2.4.1 -> 2.6.0, but every bump needs a header diff.
- Thread safety: the DLL keeps a global, unguarded `std::unordered_map handleMap_` (`PresentMonAPI.cpp:24-25`):
  all calls must come from one thread. The client spawns its own asio/IPC thread inside our process
  (`Interprocess/source/act/SymmetricActionClient.h:36-38`).

### d. Provider toggling / idle cost

**Yes, exactly what issue #573 asks.** `RealtimePresentMonSession` constructor calls `StartEtwSession()`
(`RealtimePresentMonSession.cpp:17-22`) which runs `StartTraceW` **without providers**
(`trace_session_.Start(..., false)`, line 184; `PresentData/PresentMonTraceSession.cpp:475-517`). Providers are enabled
with `EnableTraceEx2` on the first tracked PID and disabled when the last target goes away
(`UpdateTracking`, lines 48-109; `PMTraceSession::StartProviders/StopProviders`, `PresentMonTraceSession.cpp:785-793`).
Issue #573 (closed): "After EA AntiCheat starts, calls to StartTraceW() consistently fail with ERROR_ACCESS_DENIED ...
EnableTraceEx2 still works"; Intel confirmed BF6 tracking works with this design.

Idle cost with the session open and no target: consumer thread blocked in `ProcessTrace`; output thread wakes every
100 ms (`RealtimePresentMonSession.cpp:539-585`); main loop every 250 ms (`PMMainThread.cpp:279-281`); flush thread
dormant (`PMMainThread.cpp:75-82`). ETW buffers: 64 KB x min 256 / max 1024 (`PresentMonTraceSession.cpp:498-500`),
i.e. up to 16-64 MB kernel buffer memory, identical in the console exe (same `PMTraceSession::Start`). 2.6.0 notes:
idle CPU 0.000 %. While tracking, the service sets `mFilteredProcessIds = false` (`RealtimePresentMonSession.cpp:151-152`)
and always enables GPU, input, display, frame type, app timing, PC latency and D3D12 PSO tracking (lines 164-171),
not configurable.

### e. Coexistence

- The official MSI registers `PresentMonSharedService` (`Library.wxs:25-37`) with default pipe/shm/session names. A
  private, non-SCM instance with its own `--etw-session-name`, `--control-pipe`, `--shm-name-prefix` shares no named
  object with it, with the console exe (session "PresentMon"), CapFrameX or FrameView (their own sessions). ETW allows a
  manifest provider in up to 8 sessions at once; PresentMon uses manifest providers only (DxgKrnl, Kernel-Process, DXGI,
  D3D9, DWM, Win32k, Intel-PresentMon, NV DisplayDriver, PCL). The 2.5.0 "shared service conflict" was an MSI
  distribution issue (asset withdrawn; "compatibility conflict with one of our downstream customers", fixed in 2.5.1),
  not a runtime naming conflict. Do not use the default names.
- Caveat: we must **not** use an installed official service with our DLL (build-id lockstep, see c).

### f. Build vs extraction

- Not in the release assets as loose files, but **extractable with Intel signatures intact**: `msiexec /a` worked
  unelevated here, all three files are Intel-signed (`Get-AuthenticodeSignature` = Valid). Admin installs do not run
  `InstallExecuteSequence` custom actions. In CI it means downloading the 170 MB MSI (cacheable) and pinning SHA-256 of
  the MSI and of the three files (same pattern as `nsis/pawnio.sha256`).
- Building from source: VS 2022 toolset v143, C++ `stdcpplatest`, vcpkg manifest mode with static triplet
  (`vcpkg.json`: gtest, cli11, boost-interprocess, boost-process, boost-circular-buffer, cereal, nlohmann-json,
  concurrentqueue, boost-asio, winreg, detours, csv-parser; `vcpkg.props`). Only service + API need building
  (`PresentMonService.vcxproj` refs PresentData, CommonUtilities, ControlLib, Interprocess, PresentMonAPIWrapperCommon,
  Versioning; `PresentMonAPI2.vcxproj` refs CommonUtilities, PresentMonMiddleware). CEF/Node/WiX/test certificate are
  only for UI/installer. Feasible on a GitHub `windows-*` runner with VS 2022, but: vcpkg Boost build is long uncached;
  the build id embeds git hash and timestamp (`Versioning/scripts/pre-build.ps1`), so not bit-reproducible; and the
  result is **not Intel-signed** (we would sign it ourselves). Extraction is clearly better.

### g. Licensing (redistribution, GPL-3.0-or-later)

- PresentMon: MIT (`LICENSE.txt`). `THIRD_PARTY.txt` lists ADL (MIT), NVAPI (MIT), CEF (BSD, UI only), DirectX error
  library (MIT), backward-cpp (MIT).
- Statically linked vcpkg libs not listed in `THIRD_PARTY.txt` but present in the binaries (strings: boost x7, cereal,
  moodycamel, winreg in the service; boost, cereal, moodycamel in the DLL): Boost (BSL-1.0), cereal (BSD-3), CLI11
  (BSD-3), concurrentqueue (BSD-2/BSL), winreg (MIT). All GPL-3 compatible; BSD/BSL notices must go into our
  `THIRD_PARTY_NOTICES.md` (we would have to collect them ourselves).
- `ControlLib/igcl/igcl_api.h` carries an **Intel proprietary "express license" banner** (lines 2-9), while the
  wrapper is MIT. It is compiled into `PresentMonService.exe` only (not the DLL). Upstream IGCL is published as MIT and
  Intel ships the binary under MIT, and the service is a separate program (GPL-3 "aggregate"), so risk is low, but it
  is a wrinkle for our "no proprietary vendor headers" policy (that policy is about our sources; we would not include
  the header). `nvml/nvml.h` is an Intel MIT rewrite; NVAPI and ADL headers are MIT.
- `PresentMonAPI2.dll` (MIT) loaded into our GPL process: compatible.

### Security notes specific to option 2 (important for a LocalSystem product)

- With a custom `--control-pipe` the server uses `SecurityMode::Child` = SDDL `D:(A;OICI;GA;;;WD)` (**Everyone, full
  access**, `ActionServer.cpp:26-28`, `CommonUtilities/pipe/Pipe.cpp:121-122`); the default service mode is
  `D:P(A;;GA;;;AU)S:(ML;;NW;;;LW)`. Any local user can connect to our LocalSystem child and issue StartTracking /
  SetEtwFlushPeriod / SetTelemetryPeriod / ReportMetricUse (ETL logging is disabled in this build:
  `acts/FinishEtlLogging.h` returns `PM_STATUS_FEATURE_DISABLED`), and send cereal-serialised input to a SYSTEM process.
- `GA` for Everyone includes `FILE_CREATE_PIPE_INSTANCE`: a local user can add their own server instance of the same
  pipe name, and the DLL's client connects with plain `CreateFileA` without `SECURITY_SQOS_PRESENT`
  (`Pipe.cpp:143-157`). Our **LocalSystem** service would then be the client of an attacker-controlled server: the
  attacker cannot get a usable SYSTEM token without SeImpersonatePrivilege (standard users get identification level),
  but can feed arbitrary cereal messages and a shm prefix to C++ code running inside oma-service. Intel's own clients
  run as the user, so this matters more for us than for them. Mitigations: random per-run names (pipes are
  enumerable, so this only narrows the race), or host the DLL in a separate low-privilege helper process (more code),
  or patch + self-build (loses the Intel signature).
- Any crash or access violation in the DLL takes down oma-service.

---

## 3. Option 1: console exe + CSV (`PresentMon/`)

- Binary: `PresentMon-2.6.0-x64.exe`, 980 KB, Intel-signed (Valid). Static CRT (`PresentMon.vcxproj:142`), only system
  DLLs. NVIDIA manifest is **embedded** as a resource and loaded with `TdhLoadManifestFromBinary`
  (`PresentMon/MainThread.cpp:24-40`, `ddETWExternalRcWrapper.rc`). No registry, no other files. MIT; uses PresentData
  and `CommonUtilities/mc` (the same `MetricsCalculator` the service middleware uses, so metrics match by construction).
- Flags (all exist, `CommandLine.cpp:263-303,436-477`, README-ConsoleApplication.md): `--session_name`,
  `--stop_existing_session`, `--terminate_existing_session`, `--output_stdout` (forces console stats off and flushes
  after header and **every row**, `CsvOutput.cpp:204-206, 890-893`), `--track_frame_type`, `--track_pc_latency`,
  `--process_id`, `--process_name` (repeatable), `--exclude`, `--no_track_gpu`, `--no_track_input`,
  `--terminate_on_proc_exit`, `--qpc_time`, `--write_frame_id`.
- CSV columns (default metrics, `CsvOutput.cpp:531-672`): `Application, ProcessID, SwapChainAddress, PresentRuntime,
  SyncInterval, PresentFlags, AllowsTearing, PresentMode, FrameType` (with `--track_frame_type`), `TimeInSeconds|TimeInQPC`,
  `MsBetweenSimulationStart, MsBetweenPresents, MsBetweenDisplayChange, MsInPresentAPI, MsRenderPresentLatency,
  MsUntilDisplayed, MsPCLatency` (with `--track_pc_latency`), `CPUStartTime|CPUStartQPC, MsBetweenAppStart, MsCPUBusy,
  MsCPUWait, [GPU columns], MsAnimationError, AnimationTime, MsFlipDelay, [input latency], [MsInstrumentedLatency]`.
  FrameType text: `Application`, `Intel XeSS-FG`, `AMD AFMF`; NotSet/Repeated are printed as `Application` in release
  builds (`CsvOutput.cpp:54-71`). Parse by header name, not position.
- PID filtering is user-mode in the consumer (`PresentMonTraceConsumer.cpp:3609-3611`, DWM always kept), so a
  `--process_id` run discards foreign events early; without a filter all presenting processes are analysed and printed.
- Start before the game: yes, with no `--process_id` (all processes) or with `--process_name` (needs the exe name in
  advance). But the console **always calls StartTraceW with providers enabled at start** (`MainThread.cpp:327-336`,
  `PMTraceSession::Start` default `enableProviders = true`, `PresentMonTraceSession.hpp:59`), and the PID cannot be
  changed at runtime (no control channel). So per-game restarts call StartTrace again -> **fails under EA Javelin**.
  Javelin-safe use of option 1 = one unfiltered instance started when the overlay feature is switched on (before the
  game), filtering by PID in our parser; cost = providers on (all processes) for as long as the overlay feature is on
  (`--no_track_gpu --no_track_input` remove the heaviest DxgKrnl DMA/input events). Needs measuring with
  `measure-footprint.ps1`.
- Target exit: with `--terminate_on_proc_exit` it exits when all targets have exited (`OutputThread.cpp:152-160`);
  without it, it keeps running idle.
- Stop: Ctrl events only (`MainThread.cpp:150-167`); from a service: kill (job object) then
  `PresentMon.exe --session_name X --terminate_existing_session`, or `ControlTraceW(STOP)` (1 P/Invoke). Already-running
  same-name session: `--stop_existing_session`.
- Latency: the console never flushes ETW buffers manually (no `EVENT_TRACE_CONTROL_FLUSH` in `PresentMon/`) and its
  output loop sleeps 100 ms (`OutputThread.cpp:634-635`), so data arrives in about 1 s chunks (default realtime flush).
  Mitigation: our service calls `ControlTraceW(0, sessionName, props, EVENT_TRACE_CONTROL_FLUSH)` every 100-250 ms (it
  is what the Intel service does in `RealtimePresentMonSession::FlushEvents`).
- Security: stdout is an anonymous pipe owned by oma-service; no named objects, no IPC surface. Clean.
- Encoding: to a pipe, stdout is not switched to UTF-16 (`Console.cpp:15-45`), so non-ASCII app names may be mangled;
  irrelevant since we filter by PID.

---

## 4. Verdict

| | Option 1: console exe + CSV | Option 2: service child + API2 |
|---|---|---|
| Payload | 1 Intel-signed file, 0.96 MB, direct release asset | 3 files, 5.3 MB, extracted from a 170 MB MSI (`msiexec /a`), Intel-signed |
| Our code (C#, est.) | ~350-450 LOC + ~150 LOC tests: launcher/job object, header-mapped CSV parser, flush timer + session stop (2 small P/Invokes) | ~600-800 LOC + ~200 LOC tests: launcher (random names, readiness wait, stdin `%quit`), 10 P/Invoke functions + 2 structs, blob decoder, single-thread owner, reconnect after child crash; +~300 LOC if the DLL is moved to a low-privilege helper |
| Testability | Pure parser, fixtures = recorded CSV; trivial TDD | P/Invoke layer only testable elevated/live; blob decoder unit-testable |
| Moving parts | 1 child process, anonymous pipe | child process + named pipe + shm + C++ DLL in our SYSTEM process + build-id lockstep |
| Anti-cheat (StartTrace denied after game start) | Only if started before the game and left unfiltered (all processes, providers always on while overlay is on) | Solved by design: session at child start, `EnableTraceEx2` per target (issue #573) |
| Idle cost while armed | Providers on, all presenting processes analysed (low with `--no_track_gpu`, to be measured) | Providers off until a PID is tracked; ~0 CPU; plus one vendor-telemetry init per session and a 0.2 Hz loop if period set to 5000 |
| Latency | About 1 s without help; 0.2-0.4 s with our flush timer | Configurable 8-1000 ms flush |
| Version bumps | Robust (parse by header name; columns have been stable in the default set) | Re-diff the header each bump (enum insertions in 2.6.0); always ship DLL+service pair |
| Security | No new surface | Everyone-GA control pipe on a SYSTEM child; untrusted-input parsing (cereal) in-process in oma-service |
| Telemetry duplication | None | ADL/IGCL/NVAPI/NVML/WMI/PDH init in the child, cannot be disabled |
| Effort | ~2-3 days | ~5-8 days (+ security hardening) |

**Option 2 is not "really complicated" in raw API terms**: the ABI is a clean C ABI (10 functions, 2 POD structs), the
service officially runs as a console child (`--svc-as-child` is Intel's own dev path), custom names are first-class
CLI options, and Intel-signed binaries can be extracted from the MSI without building. Its real costs are elsewhere:
double the code, a 5x payload pulled out of a 170 MB MSI, build-id lockstep, an undocumented-for-production console mode
with quirks (HKLM key creation, broken `--timed-stop`, stop via stdin test hooks), telemetry that cannot be turned off,
and above all a C++ client DLL running inside our LocalSystem service that talks over an Everyone-writable pipe.

**Recommendation: go with option 1 (console exe), designed behind a `IFrameSource`-style seam.** Run one instance with
`--output_stdout --track_frame_type --track_pc_latency --no_track_gpu --no_track_input --qpc_time --session_name
<ours> --stop_existing_session`, start it when the overlay feature is switched on (so it predates the game, which keeps
Javelin titles working without per-game StartTrace), filter by the PID the app gives us, add a 100-250 ms
`EVENT_TRACE_CONTROL_FLUSH` timer, and kill + `--terminate_existing_session` on stop. Measure the armed-idle cost with
`measure-footprint.ps1`; if it breaks the 1 % budget, or a future anti-cheat also needs provider toggling, swap the seam
to option 2 then, hosting PresentMonAPI2 in a low-privilege helper rather than in oma-service.

Not verified live (needs elevation or LocalSystem): an actual frame query end to end, the console's armed-idle CPU
cost, and the Javelin behaviour.
