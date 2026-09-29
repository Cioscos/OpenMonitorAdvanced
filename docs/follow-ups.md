# Follow-ups

Items consciously left open, with where they live and when they are expected to be picked up.
Updated at the end of every milestone (last update: M5a).

## Open: code

| Item | Where | Pick up |
|---|---|---|
| USB disks and the D6 gate. A USB stick (live check 2026-09-30: SanDisk Extreme, bus 0x07, seek penalty query error 1, ATA pass-through error 50) plugged in when the service starts keeps the D6 gate closed (`keeps storage disabled: power state unknown`), so SMART stays off for all disks until it is unplugged; plugged in at runtime it only skips its own SMART. The per-disk SMART switch (M5a) does not help, because the gate closes before it is read. The fix is the SAT `CHECK POWER MODE` fallback of F1.4 (`docs/superpowers/references/m5/f1-service-reconfiguration.md`), to be planned as a spike, not a switch. | `service/OpenMonitorAdvanced.Service/Sensors/` (D6 gate) | M5b/M5c spike |
| On a machine with more than one interactive user, any of them can stop `oma-service` for the others: the service has no notion of "who asked". | `app/src-tauri/src/service.rs` | accepted |
| DDR5 SPD page stays on whichever page it was left on (e.g. page 4) after the service stops, instead of resetting; stock LHM behaves the same way (found in Task 15, 2026-09-27). Optional bounded reset for parity, otherwise accepted. | `app/src/` (RAM/SPD page) | accepted; revisit if a user reports it |
| `app/src-tauri/nsis/*.dll` helper (`nsExec.dll`) ships unsigned, so SmartScreen may still warn even after the main binaries are signed. | `app/src-tauri/nsis/` | M6 (SignPath phase) |
| A third-party folder inside `Program Files` that grants `Users` Modify rights passes the Advanced-sensors-component install-path check, which only verifies the path is under `Program Files` and free of reparse points, not its ACL. | `app/src-tauri/nsis/oma.nsh` (install-path check) | accepted; verify/document |
| A silent install refused for a reason other than the path check (for example a missing `/NOSENSORS` outside `Program Files`) may still have created an empty `$INSTDIR` and installed the WebView2 runtime before refusing. | `app/src-tauri/nsis/installer.nsi` | accepted; verify/document |
| Redistributing the official PawnIO setup is common practice (LibreHardwareMonitor and FanControl both do it), but the PawnIO author has not been asked to confirm it for this project. | `THIRD_PARTY_NOTICES.md`, `scripts/build-installer-payload.ps1` | before 1.0 |
| `used_pct` is duplicated in the memory and storage providers. | `crates/oma-win/src/memory.rs`, `crates/oma-win/src/storage.rs` | when touched |
| PDH: the item count returned by the API goes unchecked into `from_raw_parts`, and a null `szName` is not guarded. | `crates/oma-win/src/pdh.rs` | when touched |
| The label-key test keeps a hand-written list: only GPU keys are cross-checked against the code (`GpuField` self-test); CPU, memory, storage and network keys are not. | `crates/oma-win/tests/labels.rs` | when touched |
| The CSP has no `devCsp` with `ws://localhost:1420`, so Vite hot reload inside `pnpm tauri dev` may be blocked. | `app/src-tauri/tauri.conf.json` | when touched |
| NVML is not initialised again after the NVIDIA driver is updated or unloaded while the app runs; its fields fall back to D3DKMT until a restart (README, "Known limits"). | `crates/oma-win/src/gpu/nvml.rs` | M6 |
| Disk temperature probes retry every 30 s, including disks asleep at startup; new driver sensor indices request rediscovery without waking a sleeping disk. Verify real standby/wake behavior before using these readings in rules. | `crates/oma-win/src/storage.rs` | M5 (disk rules; retry and index identity already covered in M3) |
| A disk identified only by its PnP instance id (no serial, no unique GPT or MBR id) gets a new id when it is moved to another port: its history and statistics restart. | `crates/oma-win/src/storage_identity.rs` | accepted |
| Intel and other GPUs whose PnP maximum-link read fails show no maximum link at all: `pcieMaxGen`/`pcieMaxWidth` come only from the PnP base layer (device capability, identical in safe mode), because NVML's max-link calls report the device+slot-limited value and IGCL does not read `ctlPciGetProperties`. Not yet exercised on Intel or non-NVIDIA hardware. | `crates/oma-win/src/gpu/pnp.rs` | M6 (hardware matrix) |
| Discovery reads every disk's temperature in one tick; with several NVMe drives waking from a low-power state this can exceed the 200 ms tick budget, so the value arrives one tick late. | `crates/oma-win/src/storage.rs` | M5 (disk rules) |
| `gpu/pnp.rs` `display_interfaces` has no retry on `CR_BUFFER_SMALL`: a GPU hot-plugged between the two calls gets no maximum link on that discovery. | `crates/oma-win/src/gpu/pnp.rs` | M6 (hardware matrix) |
| Re-identification at hot-plug. DiskInfoToolkit's hot-plug thread re-identifies the not-yet-identified disks on every `DBT_DEVNODES_CHANGED`, which can wake them (F1.1). | `service/OpenMonitorAdvanced.Service/Sensors/` (storage) | M5b spike |
| A rapid off/on of the memory module may delay the SMBus through the `~SPDAccessor` finalizers (F2.3). Measure when the module is switched back on quickly. | `service/OpenMonitorAdvanced.Service/Sensors/ModuleApplier.cs` | when touched |
| A D6 blocker without model or serial leaves `smartBlockedBy` empty (R16): the Sources view shows only the generic limit text and cannot name the disk. Add a flag like `smartGateClosed`. | `service/OpenMonitorAdvanced.Service/`, `app/src/` | M5b |
| `smartSelectable` (R20, a hidden storage device property) is UI plumbing: keep it out of the M5c CSV metadata. | `crates/oma-win/src/storage.rs` | M5c |
| A leftover `settings.json.tmp` is not recovered at load (a failed save can leave defaults on the next start), and no test ties the field names of `encode`, `decode_lenient` and the patch schema together (patch `encode(everything_changed())` minus the read-only keys). Both before M5b types `rules`. | `app/src-tauri/src/settings/` | M5b (first task) |
| `Settings` is cloned on every tick; add a narrow accessor before M5b's rules read it. | `app/src-tauri/src/settings/` | M5b |
| An unknown module name or message `type` is echoed unbounded in `bad_request` and in the log, and can exceed `MaxFrameBytes`. Truncate. | `service/OpenMonitorAdvanced.Service/Protocol/`, `Sensors/` | when touched |
| `shell_open` joins the shell thread without a timeout, uses `ShellExecuteW` without `SEE_MASK_NOASYNC`/`FLAG_NO_UI` (use `ShellExecuteExW`), and does not check that a folder exists before opening it. | `app/src-tauri/src/` (shell commands) | when touched |
| The installer is perMachine, so the uninstaller deletes the HKCU Run value in the elevating admin's hive: a standard user's own Run value survives. The Run value also keeps a stale exe path if the exe moves (compare with the current path at startup and repair). | `app/src-tauri/nsis/oma.nsh`, `app/src-tauri/src/autostart.rs` | M6 |
| Service link: the command queue is unbounded in production, and the status stays stale while `Held` (`NotInstalled`/`Stopped` are not shown after `Incompatible`/`PidMismatch` until Start). | `crates/oma-win/src/svc/link.rs` | when touched |
| A second launch opens the settings store before single-instance exits it, and the interval listener may take the engine lock on the main thread. | `app/src-tauri/src/main.rs` | when touched |
| Flaky timing assert: `reads_disk_temperatures_on_this_machine` (pre-existing hardware test) fails its 200 ms bound under parallel load (1-17 ms in isolation). Loosen or serialize. | `crates/oma-win/` (storage tests) | when touched |
| `SvcProvider::poll` clones the whole snapshot and the drive list every tick only to compare generations; generation accessors would avoid the per-tick allocation. | `crates/oma-win/src/svc/provider.rs` | when touched |
| Pipe listener: after a failed connect, the replacement instance is created after the old one is disposed and without `FILE_FLAG_FIRST_PIPE_INSTANCE`, a brief zero-instance gap (the client-side PID check protects the app). Cheap hardening: create it with `first: Volatile.Read(ref _busy) == 0`, so a squatter makes it fail loudly (R20) instead of being joined. | `service/OpenMonitorAdvanced.Service/Pipe/PipeListener.cs` | when touched |
| The service's logs live in `$INSTDIR\service\logs` (ruling R30). Any future service-owned path (settings, rules) must stay under a folder no user can create first, with the same service-side check (`LogDirectoryGuard`), never under `%ProgramData%`. | `service/OpenMonitorAdvanced.Service/Logging/LogDirectoryGuard.cs`, `app/src-tauri/nsis/oma.nsh` | M5 (settings) |
| `Mono.Posix.NETStandard` 1.0.0 (a LibreHardwareMonitor dependency, now referenced directly to drop its native assets) has its licence only behind a Microsoft fwlink: confirm the terms when the licences are reviewed. | `THIRD_PARTY_NOTICES.md` | M6 (licences) |

## Open: deferred features (spec §5.2 point 5, §7.3; M3 decision D10)

- NVML `TotalEnergyConsumption` (p95 about 9 ms) and `PcieThroughput` (blocks 31 ms): only with sampling outside the tick, when the CSV log or the rules need them (M5).
- Battery page: appears when a battery provider exists.
- Per-disk SMART switch and per-module switches are done (M5a); what is left for USB disks is the SAT fallback above.
- **Not covered yet, despite LibreHardwareMonitor exposing related sensors — do not assume the mapping surfaces them without re-checking `SchemaBuilder.cs`:**
  - **CPU throttling / distance to TjMax:** `MatchCpuSensor` (`service/OpenMonitorAdvanced.Service/Sensors/SchemaBuilder.cs`) maps load, temperature and power/voltage sensors only; no throttle-reason or "Distance to TjMax" sensor is matched. To check availability, dump `SensorNode` names/types for the `Cpu` hardware (the S1 spike's `LhmDump` tool, or a `SchemaBuilderTests` fixture) on Intel and AMD CPUs and look for a temperature/factor sensor named along those lines before adding a match.
  - **RAM SPD timings:** `SchemaBuilder.cs` (around the DIMM temperature match, see the comment there) explicitly discards the SPD timing and capacity sensors RAMSPDToolkit exposes on each DIMM. To check availability, enable PawnIO and dump a DIMM's `SensorNode`s: the timing values are present but currently thrown away, not absent from LHM.
  - **SMART critical warning:** `MatchStorageSensor` discards any sensor whose name starts with `"Warning"` or `"Critical"` (temperature limits and NVMe/SMART warning sensors alike). To check availability, dump a drive's `SensorNode`s and confirm which of those discarded sensors carry an actual critical/warning boolean or threshold worth mapping as a `flag` sensor before an M5 rule tries to consume it.

## Manual checks owed by a human

- Tray left click and the "Open" menu item re-create the window after it was closed (M1).
- USB disk hot-plug keeps or changes disk ids correctly (M1). Since M3 also: a disk without a serial number (for example a VHDX mounted by an administrator) appears with a `storage/gpt-…` id and keeps it across a restart.
- IGCL telemetry and PCIe link on Intel hardware, including `ctlPciGetState` layout and per-tick cost (unmeasured, no Intel hardware available); ADL on a dedicated Radeon (hardware matrix, spec §12).
- HDD standby (M5a live check 2026-09-30, inconclusive): with a 60 s disk timeout the SATA HDD (ST2000DM008) never spun down, even with the app closed and the service stopped, and 0 file transfers/s on the disk, so OMA is not the cause. Retest over a longer window (5-10 min) and check the drive's own power management. Once a baseline spin-down is observed, check whether the core's 30 s disk temperature poll (`TEMPERATURE_PERIOD`, not gated by the SMART switch) resets the idle timer; if it does, gate it on observed disk activity (M5 disk rules).
- An MBR disk or a VHD without a serial number gets a `storage/mbr-…` id: the MBR identity tier and the geometry IOCTL were never exercised on real hardware (every disk in the M3 hardware matrix is GPT).
- Anti-cheat compatible mode's toggle, service stop/restart and badge persistence were verified live in Task 15 (2026-09-27, dev machine); still owed: a real anti-cheat-protected game with the mode on and off (FACEIT or Vanguard with PawnIO 2.2.0 loaded, spec §13 point 1, Task 15 Phase H, optional).
- D6 (disk standby detection) checks on real hardware: first open, hot-plug, sampling cadence; empty card readers' error codes; the R17 bus-class exclusions (NVMe, virtual disks, Storage Spaces). Not exercised in Task 15 (2026-09-27); until it is, do not treat the absence of disk wake as guaranteed.
- The service's memory footprint against the 80 MB budget on the rest of the hardware matrix (the dev machine is done, see "Closed in M4").
- Task 15's VM fault-injection scenarios for the installer and service, still owed as of 2026-09-27 (not attempted on this PC): `/S /NOSENSORS`, the Components page in EN and IT, deselecting the Advanced sensors component, an upgrade from the interface, an uninstall that leaves PawnIO, the reboot PawnIO requests (exit code 3010), STOP stuck, the uninstall helper exiting 1, PawnIO setup exiting neither 0 nor 3010, a refused custom install directory outside `Program Files`, an upgrade with a leftover `service\logs` holding a junction (recursive `icacls /reset /T` must not follow it), and a third-party writable folder inside `Program Files`.

## Manual checks owed after M5a

- PawnIO scenarios in a VM: driver stopped, uninstalled, and the 3010 reboot state (`rebootPending`, then `ok` after the reboot); the Win32 codes 2/3/5 of the probe live. `ok` was checked on the dev machine.
- Autostart across a real logout/login (the Run value and the Task Manager enable/disable states were verified; the login itself was deferred).
- Per-module switches with two clients (two user sessions): the module stays on while one of them wants it.

## Closed in M5a

- Log guard: the log is flushed at exit.
- Single-instance arguments: a second `--minimized` launch no longer opens the window.
- Network history chart in byte/s: it follows the bit/s or byte/s setting.
- The 20 Hz link wake (`POLL_SLICE`): the link thread now blocks until its deadline.
- `Incompatible` reconnects: the link is re-probed only on "Start".
- PawnIO status: `Hello` carries it and the UI shows it in Data sources and in the badge popup.
- Advanced view state moved from `localStorage` to `settings.json` (imported once at first start).
- Live checks (2026-09-30, dev machine, release build): first start with migration, tray icon and tooltip, Simple/Advanced tray items, language switch, 2 s interval, close-to-tray off, autostart and the Task Manager states. Budget in `docs/perf-budget.md` (M5a).

## Closed in M4

- "N more sensors available with the service" (M3 deferred item, spec §7.3): implemented as a single generic notice, without a count, on the CPU/RAM/disk pages when the service is not connected — the app cannot know the count without the service.
- Spec §13 point 1 (FACEIT and PawnIO): resolved by research — the earlier FACEIT block was tied to the signing certificate of PawnIO versions before 2.1.0, not to the driver's presence; PawnIO 2.2.0 (Microsoft-signed) is accepted with the driver loaded. A real-game check stays open (see "Manual checks" above).
- Spec §13 point 2 (LibreHardwareMonitorLib trimming and NativeAOT): decided by the S1 spike — trimmed self-contained publish ships (identical sensor set trimmed vs. untrimmed, `docs/superpowers/references/m4/trim-warnings.md`), NativeAOT is excluded because LibreHardwareMonitorLib's WMI paths are unsafe under it.
- Task 15 (live verification, 2026-09-27, dev machine): Phases A-G done (build/install, service ACL/DACL/failure-action checks, on-screen sensor pages, anti-cheat toggle with service stop/restart and badge persistence across an app restart, idle shutdown after 2 minutes, and the app+service footprint measurement). Phase H (a real anti-cheat game) stays open, see "Manual checks" above.
- Shutdown crash found and fixed during Task 15 (9c9eea8): on STOP the service crashed in a RAMSPDToolkit `SPDAccessor` finalizer, because the old close sequence ran a forced GC after `DriverManager.UnloadDriver()` had already nulled the module the finalizer needed, causing exit code 1 and an SCM restart with anti-cheat still on. Fixed by having the service call only `LhmTree.Dispose()` (`computer.Close()`) and exit, without the forced GC or `UnloadDriver()`. Verified after the fix: clean STOP, exit code 0, no SCM events.
- The service's private memory (spec §1.2 budget, 80 MB) measured in the S1 spike at 63-84 MB; Task 15's real measurement (2026-09-27, dev machine) is 51.6 MB (window) / 52.7 MB (tray), within budget (`docs/perf-budget.md` M4 rows). Still owed: the same measurement across the rest of the hardware matrix (see "Manual checks" above).

## Closed in M3

- Disks without a unique readable serial are no longer dropped (identity fallback chain, Task 4).
- The "monitoring for N min" banner counts from the core's first tick, also after reopening from the tray (`startedAtMs`, Tasks 1 and 10).
- A panic in `Engine::tick` no longer stops sampling; `History::push` no longer panics on a length mismatch; the UI shows *Data not updating* (Tasks 2 and 10).
- Implausible-value debug logs are rate-limited to one line per sensor per minute (Task 2).
- The GPU PCI address is kept per LUID across enumerations (Task 6).
- A second launch while the window is closed re-creates the window: exercised by the full-history measurement (Task 14, Step 7).
