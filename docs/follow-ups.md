# Follow-ups

Items consciously left open, with where they live and when they are expected to be picked up.
Updated at the end of every milestone (last update: M4).

## Open: code

| Item | Where | Pick up |
|---|---|---|
| A USB HDD whose bridge rejects ATA pass-through gets no LHM SMART at all, because a disk that can't confirm its power state keeps SMART off for every disk (D6). Needs a per-module switch to disable storage SMART for just that disk instead. | `service/OpenMonitorAdvanced.Service/Sensors/` (D6 filter) | M5 (per-module LHM switches, disk rules) |
| On a machine with more than one interactive user, any of them can stop `oma-service` for the others: the service has no notion of "who asked". | `app/src-tauri/src/service.rs` | accepted |
| The service's private memory measured 63–84 MB in the S1 spike against the 80 MB budget (spec §1.2); Task 6's countermeasures (non-concurrent GC, `ConserveMemory`, LHM history off, post-`Open` compaction) narrow this but Task 15's real measurement is the gate. | `service/OpenMonitorAdvanced.Service/` | M6 (perf budget) — report to the user rather than widen the budget if still over 80 MB |
| `app/src-tauri/nsis/*.dll` helper (`nsExec.dll`) ships unsigned, so SmartScreen may still warn even after the main binaries are signed. | `app/src-tauri/nsis/` | M6 (SignPath phase) |
| A third-party folder inside `Program Files` that grants `Users` Modify rights passes the Advanced-sensors-component install-path check, which only verifies the path is under `Program Files` and free of reparse points, not its ACL. | `app/src-tauri/nsis/oma.nsh` (install-path check) | accepted; verify/document |
| A silent install refused for a reason other than the path check (for example a missing `/NOSENSORS` outside `Program Files`) may still have created an empty `$INSTDIR` and installed the WebView2 runtime before refusing. | `app/src-tauri/nsis/installer.nsi` | accepted; verify/document |
| Redistributing the official PawnIO setup is common practice (LibreHardwareMonitor and FanControl both do it), but the PawnIO author has not been asked to confirm it for this project. | `THIRD_PARTY_NOTICES.md`, `scripts/build-installer-payload.ps1` | before 1.0 |
| The log guard is never dropped: `App::run` ends the process with `process::exit`, so the last buffered log lines can be lost. Use `run_return`, or flush on `RunEvent::Exit`. | `app/src-tauri/src/main.rs` | M5 (tray and settings) |
| The single-instance callback ignores the second launch's arguments, so a second `--minimized` launch opens the window. `scripts/measure-footprint.ps1 -FillHistoryMinutes` relies on a second launch *without* arguments opening the window: keep that working. | `app/src-tauri/src/main.rs` | M5 (tray) |
| `used_pct` is duplicated in the memory and storage providers. | `crates/oma-win/src/memory.rs`, `crates/oma-win/src/storage.rs` | when touched |
| PDH: the item count returned by the API goes unchecked into `from_raw_parts`, and a null `szName` is not guarded. | `crates/oma-win/src/pdh.rs` | when touched |
| The label-key test keeps a hand-written list: only GPU keys are cross-checked against the code (`GpuField` self-test); CPU, memory, storage and network keys are not. | `crates/oma-win/tests/labels.rs` | when touched |
| The CSP has no `devCsp` with `ws://localhost:1420`, so Vite hot reload inside `pnpm tauri dev` may be blocked. | `app/src-tauri/tauri.conf.json` | when touched |
| NVML is not initialised again after the NVIDIA driver is updated or unloaded while the app runs; its fields fall back to D3DKMT until a restart (README, "Known limits"). | `crates/oma-win/src/gpu/nvml.rs` | M6 |
| Disk temperature probes retry every 30 s, including disks asleep at startup; new driver sensor indices request rediscovery without waking a sleeping disk. Verify real standby/wake behavior before using these readings in rules. | `crates/oma-win/src/storage.rs` | M5 (disk rules; retry and index identity already covered in M3) |
| A disk identified only by its PnP instance id (no serial, no unique GPT or MBR id) gets a new id when it is moved to another port: its history and statistics restart. | `crates/oma-win/src/storage_identity.rs` | accepted |
| Intel and other GPUs whose PnP maximum-link read fails show no maximum link at all: `pcieMaxGen`/`pcieMaxWidth` come only from the PnP base layer (device capability, identical in safe mode), because NVML's max-link calls report the device+slot-limited value and IGCL does not read `ctlPciGetProperties`. Not yet exercised on Intel or non-NVIDIA hardware. | `crates/oma-win/src/gpu/pnp.rs` | M6 (hardware matrix) |
| Network pages show KPIs and the sensor table in bit/s but the history chart's axis and legend stay in byte/s: a known limit until per-sensor unit settings arrive. | `app/src/components/advanced/HistoryChart.svelte` | M5 (settings/units) |
| Discovery reads every disk's temperature in one tick; with several NVMe drives waking from a low-power state this can exceed the 200 ms tick budget, so the value arrives one tick late. | `crates/oma-win/src/storage.rs` | M5 (disk rules) |
| `gpu/pnp.rs` `display_interfaces` has no retry on `CR_BUFFER_SMALL`: a GPU hot-plugged between the two calls gets no maximum link on that discovery. | `crates/oma-win/src/gpu/pnp.rs` | M6 (hardware matrix) |
| The service link thread wakes every 50 ms while connected (`Driver::next_event` reads the pipe in `POLL_SLICE` slices to notice commands): a permanent 20 Hz timer wake in the tray process. Merge commands and pipe events into one channel so the thread blocks until the deadline. Check it in the Task 15 idle measurement. | `crates/oma-win/src/svc/link.rs` | M5, or earlier if Task 15 shows it |
| After `Incompatible` or `PidMismatch` the link keeps reconnecting every 5 s, and each connection resets the service's 2-minute idle timer, so an incompatible service never idles out while the app runs. Re-probe only on "Avvia" after `Incompatible`. | `crates/oma-win/src/svc/link.rs` | M5 |
| Nothing in the UI says when the service runs without PawnIO (after a pending reboot, exit code 3010, or with PawnIO removed): the badge disappears and the CPU, board and DIMM sensors are simply missing. Needs a `Hello` field or a status detail. | `crates/oma-ipc`, `service/OpenMonitorAdvanced.Service/`, `app/src/` | M5 |
| `SvcProvider::poll` clones the whole snapshot and the drive list every tick only to compare generations; generation accessors would avoid the per-tick allocation. | `crates/oma-win/src/svc/provider.rs` | when touched |
| Pipe listener: after a failed connect, the replacement instance is created after the old one is disposed and without `FILE_FLAG_FIRST_PIPE_INSTANCE`, a brief zero-instance gap (the client-side PID check protects the app). Cheap hardening: create it with `first: Volatile.Read(ref _busy) == 0`, so a squatter makes it fail loudly (R20) instead of being joined. | `service/OpenMonitorAdvanced.Service/Pipe/PipeListener.cs` | when touched |
| The installer protects `%ProgramData%\OpenMonitorAdvanced` and its `logs` folder, and cleans `logs`, but leaves any other entry a user may have planted in `OpenMonitorAdvanced` before the first install (the service uses none). Any new path there (settings, rules) must get the same lock-down and service-side check. | `app/src-tauri/nsis/oma.nsh` (`OmaProtectLogDir`), `service/OpenMonitorAdvanced.Service/Logging/LogDirectoryGuard.cs` | M5 (settings) |
| `Mono.Posix.NETStandard` 1.0.0 (a LibreHardwareMonitor dependency, now referenced directly to drop its native assets) has its licence only behind a Microsoft fwlink: confirm the terms when the licences are reviewed. | `THIRD_PARTY_NOTICES.md` | M6 (licences) |

## Open: deferred features (spec §5.2 point 5, §7.3; M3 decision D10)

- NVML `TotalEnergyConsumption` (p95 about 9 ms) and `PcieThroughput` (blocks 31 ms): only with sampling outside the tick, when the CSV log or the rules need them (M5).
- Battery page: appears when a battery provider exists.
- Advanced view state (section, chart window, series) lives in the WebView `localStorage` until `settings.json` (M5).
- Per-module LibreHardwareMonitor switches (M5), including turning storage SMART off for a specific disk (see the USB HDD row above).
- **Not covered yet, despite LibreHardwareMonitor exposing related sensors — do not assume the mapping surfaces them without re-checking `SchemaBuilder.cs`:**
  - **CPU throttling / distance to TjMax:** `MatchCpuSensor` (`service/OpenMonitorAdvanced.Service/Sensors/SchemaBuilder.cs`) maps load, temperature and power/voltage sensors only; no throttle-reason or "Distance to TjMax" sensor is matched. To check availability, dump `SensorNode` names/types for the `Cpu` hardware (the S1 spike's `LhmDump` tool, or a `SchemaBuilderTests` fixture) on Intel and AMD CPUs and look for a temperature/factor sensor named along those lines before adding a match.
  - **RAM SPD timings:** `SchemaBuilder.cs` (around the DIMM temperature match, see the comment there) explicitly discards the SPD timing and capacity sensors RAMSPDToolkit exposes on each DIMM. To check availability, enable PawnIO and dump a DIMM's `SensorNode`s: the timing values are present but currently thrown away, not absent from LHM.
  - **SMART critical warning:** `MatchStorageSensor` discards any sensor whose name starts with `"Warning"` or `"Critical"` (temperature limits and NVMe/SMART warning sensors alike). To check availability, dump a drive's `SensorNode`s and confirm which of those discarded sensors carry an actual critical/warning boolean or threshold worth mapping as a `flag` sensor before an M5 rule tries to consume it.

## Manual checks owed by a human

- Tray left click and the "Open" menu item re-create the window after it was closed (M1).
- USB disk hot-plug keeps or changes disk ids correctly (M1). Since M3 also: a disk without a serial number (for example a VHDX mounted by an administrator) appears with a `storage/gpt-…` id and keeps it across a restart.
- IGCL telemetry and PCIe link on Intel hardware, including `ctlPciGetState` layout and per-tick cost (unmeasured, no Intel hardware available); ADL on a dedicated Radeon (hardware matrix, spec §12).
- Set the power plan's "turn off hard disk after" to 1-2 minutes, put the app in the tray, and confirm the SATA HDD spins down and stays down. If it does not, gate the 30 s refresh on observed disk activity (PDH idle time) — M5 disk rules.
- An MBR disk or a VHD without a serial number gets a `storage/mbr-…` id: the MBR identity tier and the geometry IOCTL were never exercised on real hardware (every disk in the M3 hardware matrix is GPT).
- Anti-cheat compatible mode has not been verified against a real anti-cheat-protected game (FACEIT with PawnIO 2.2.0 loaded, spec §13 point 1).
- D6 (disk standby detection) checks on real hardware: first open, hot-plug, sampling cadence; empty card readers' error codes; the R17 bus-class exclusions (NVMe, virtual disks, Storage Spaces).
- The service's memory footprint (Task 15 measurement) against the 80 MB budget, on the hardware matrix.
- Task 15's VM fault-injection scenarios for the installer and service: STOP stuck, uninstall helper exit 1, PawnIO setup exit code other than 0/3010, upgrade over an existing install, deselecting the Advanced sensors component, `/NOSENSORS`, a refused custom install directory, and the 3010 (reboot required) path.

## Closed in M4

- "N more sensors available with the service" (M3 deferred item, spec §7.3): implemented as a single generic notice, without a count, on the CPU/RAM/disk pages when the service is not connected — the app cannot know the count without the service.
- Spec §13 point 1 (FACEIT and PawnIO): resolved by research — the earlier FACEIT block was tied to the signing certificate of PawnIO versions before 2.1.0, not to the driver's presence; PawnIO 2.2.0 (Microsoft-signed) is accepted with the driver loaded. A real-game check stays open (see "Manual checks" above).
- Spec §13 point 2 (LibreHardwareMonitorLib trimming and NativeAOT): decided by the S1 spike — trimmed self-contained publish ships (identical sensor set trimmed vs. untrimmed, `docs/superpowers/references/m4/trim-warnings.md`), NativeAOT is excluded because LibreHardwareMonitorLib's WMI paths are unsafe under it.

## Closed in M3

- Disks without a unique readable serial are no longer dropped (identity fallback chain, Task 4).
- The "monitoring for N min" banner counts from the core's first tick, also after reopening from the tray (`startedAtMs`, Tasks 1 and 10).
- A panic in `Engine::tick` no longer stops sampling; `History::push` no longer panics on a length mismatch; the UI shows *Data not updating* (Tasks 2 and 10).
- Implausible-value debug logs are rate-limited to one line per sensor per minute (Task 2).
- The GPU PCI address is kept per LUID across enumerations (Task 6).
- A second launch while the window is closed re-creates the window: exercised by the full-history measurement (Task 14, Step 7).
