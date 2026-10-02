# Follow-ups

Items consciously left open, with where they live and when they are expected to be picked up.
Updated at the end of every milestone (last update: M6a).

## Open: code

| Item | Where | Pick up |
|---|---|---|
| USB disks and the D6 gate. A USB stick (live check 2026-09-30: SanDisk Extreme, bus 0x07, seek penalty query error 1, ATA pass-through error 50) plugged in when the service starts keeps the D6 gate closed (`keeps storage disabled: power state unknown`), so SMART stays off for all disks until it is unplugged; plugged in at runtime it only skips its own SMART. The per-disk SMART switch (M5a) does not help, because the gate closes before it is read. The fix is the SAT `CHECK POWER MODE` fallback of F1.4 (`docs/superpowers/references/m5/f1-service-reconfiguration.md`), to be planned as a spike, not a switch. | `service/OpenMonitorAdvanced.Service/Sensors/` (D6 gate) | M6 spike |
| Upgrade over an installed version (user check, 2026-10-02, 0.3.0 setup downloaded from GitHub): the installer uninstalls the old version first, and during that step a message says the product is still running. To investigate before fixing: whether the app really was running in the tray (then the upgrade should close it itself, or ask once and clearly), whether the message comes from the old uninstaller or from the new installer (`CheckIfAppIsRunning` is called in both), and whether the product name is shown correctly or as a raw placeholder. SmartScreen also appeared on that download, as expected for an unsigned setup. | `app/src-tauri/nsis/installer.nsi` (lines marked `CheckIfAppIsRunning`), `app/src-tauri/nsis/oma.nsh` | next installer work |
| On a machine with more than one interactive user, any of them can stop `oma-service` for the others: the service has no notion of "who asked". | `app/src-tauri/src/service.rs` | accepted |
| DDR5 SPD page stays on whichever page it was left on (e.g. page 4) after the service stops, instead of resetting; stock LHM behaves the same way (found in Task 15, 2026-09-27). Optional bounded reset for parity, otherwise accepted. | `app/src/` (RAM/SPD page) | accepted; revisit if a user reports it |
| Privacy statement for the update check. The update check (M6c) will be the first network request of the app, to GitHub: review `CODE_SIGNING.md` (Privacy), the behavior and the opt-out that the SignPath Foundation terms require before it ships. | `CODE_SIGNING.md`, `docs/superpowers/specs/2026-09-30-m6a-release-firma-design.md` §7 | M6c |
| Confirm on the first real SignPath request: whether the `file-version` constraint of the artifact configurations matches the string `X.Y.Z` or the fixed `0.2.0.0` resource of the NSIS files; that the Integration Pester tests actually run (not skip) in the CI `scripts` job; the 8.3 short `TEMP` path of the runner against the uninstaller directory check of `sign-shim.ps1`; the English line `The signature is timestamped:` of signtool with SignPath's RFC 3161 timestamps. | `.signpath/`, `scripts/verify-signatures.ps1`, `scripts/sign-shim.ps1`, `.github/workflows/ci.yml` | first signed release |
| A third-party folder inside `Program Files` that grants `Users` Modify rights passes the Advanced-sensors-component install-path check, which only verifies the path is under `Program Files` and free of reparse points, not its ACL. | `app/src-tauri/nsis/oma.nsh` (install-path check) | accepted; verify/document |
| A silent install refused for a reason other than the path check (for example a missing `/NOSENSORS` outside `Program Files`) may still have created an empty `$INSTDIR` and installed the WebView2 runtime before refusing. | `app/src-tauri/nsis/installer.nsi` | accepted; verify/document |
| Redistributing the official PawnIO setup is common practice (LibreHardwareMonitor and FanControl both do it), but the PawnIO author has not been asked to confirm it for this project. | `THIRD_PARTY_NOTICES.md`, `scripts/build-installer-payload.ps1` | before 1.0 |
| `used_pct` is duplicated in the memory and storage providers. | `crates/oma-win/src/memory.rs`, `crates/oma-win/src/storage.rs` | when touched |
| PDH: the item count returned by the API goes unchecked into `from_raw_parts`, and a null `szName` is not guarded. | `crates/oma-win/src/pdh.rs` | when touched |
| The label-key test keeps a hand-written list: only GPU keys are cross-checked against the code (`GpuField` self-test); CPU, memory, storage and network keys are not. | `crates/oma-win/tests/labels.rs` | when touched |
| The CSP has no `devCsp` with `ws://localhost:1420`, so Vite hot reload inside `pnpm tauri dev` may be blocked. | `app/src-tauri/tauri.conf.json` | when touched |
| NVML is not initialised again after the NVIDIA driver is updated or unloaded while the app runs; its fields fall back to D3DKMT until a restart (README, "Known limits"). | `crates/oma-win/src/gpu/nvml.rs` | M6 |
| Disk temperature probes retry every 30 s, including disks asleep at startup; new driver sensor indices request rediscovery without waking a sleeping disk. Verify real standby/wake behavior before using these readings in rules. | `crates/oma-win/src/storage.rs` | HDD standby live check (see "Manual checks owed after M5b") |
| A disk identified only by its PnP instance id (no serial, no unique GPT or MBR id) gets a new id when it is moved to another port: its history and statistics restart. | `crates/oma-win/src/storage_identity.rs` | accepted |
| Intel and other GPUs whose PnP maximum-link read fails show no maximum link at all: `pcieMaxGen`/`pcieMaxWidth` come only from the PnP base layer (device capability, identical in safe mode), because NVML's max-link calls report the device+slot-limited value and IGCL does not read `ctlPciGetProperties`. Not yet exercised on Intel or non-NVIDIA hardware. | `crates/oma-win/src/gpu/pnp.rs` | M6 (hardware matrix) |
| Discovery reads every disk's temperature in one tick; with several NVMe drives waking from a low-power state this can exceed the 200 ms tick budget, so the value arrives one tick late. | `crates/oma-win/src/storage.rs` | when touched |
| `gpu/pnp.rs` `display_interfaces` has no retry on `CR_BUFFER_SMALL`: a GPU hot-plugged between the two calls gets no maximum link on that discovery. | `crates/oma-win/src/gpu/pnp.rs` | M6 (hardware matrix) |
| Re-identification at hot-plug. DiskInfoToolkit's hot-plug thread re-identifies the not-yet-identified disks on every `DBT_DEVNODES_CHANGED`, which can wake them (F1.1). | `service/OpenMonitorAdvanced.Service/Sensors/` (storage) | M6 spike |
| A rapid off/on of the memory module may delay the SMBus through the `~SPDAccessor` finalizers (F2.3). Measure when the module is switched back on quickly. | `service/OpenMonitorAdvanced.Service/Sensors/ModuleApplier.cs` | when touched |
| A D6 blocker without model or serial leaves `smartBlockedBy` empty (R16): the Sources view shows only the generic limit text and cannot name the disk. Add a flag like `smartGateClosed`: it needs a new field in the `service` block of the `Schema`, so protocol v3 (M5b ruling R10). The same flag would let the rules tell a sleeping disk from a missing value (see the standby limit below). | `service/OpenMonitorAdvanced.Service/`, `crates/oma-ipc/`, `app/src/` | M6 (protocol v3; M5c did not change the protocol, plan decision L15) |
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
| Rules: a disk in standby has no value, so its rules have no data and the coverage is partial: the banner says "incomplete data" instead of "all fine" while the HDD sleeps (ruling R-D). The `smartGateClosed` flag above would let the engine count a sleeping disk as covered. | `crates/oma-core/src/rules/health.rs` | with `smartGateClosed` (protocol v3) |
| CPU thermal throttling has no sensor in LibreHardwareMonitor 0.9.6 (S1 §1), so `cpu-throttle` has no instances. On Intel the service could read bit 0 of `IA32_PACKAGE_THERM_STATUS` itself through the public `LibreHardwareMonitor.PawnIo.IntelMsr`: new code, Intel only, not testable on this PC. | `service/OpenMonitorAdvanced.Service/Sensors/` | when Intel hardware is available |
| TjMax: the AMD table (`AmdTjMaxTable.cs`, 164 entries) covers desktop Ryzen only. Mobile and Ryzen AI (in the same AMD table, e.g. 7840HS at 100 °C), Threadripper, Zen/Zen+ with the offset Tctl, engineering samples and the Ryzen 3 3100/3300X (no published value) use the 85/95 °C fallback. The entries were checked against AMD's specifications table only (31 also against the product page). | `service/OpenMonitorAdvanced.Service/Sensors/AmdTjMaxTable.cs` | M6 (hardware matrix) |
| Intel `tjMaxC` is the TCC activation target without the TCC offset (bits 29:24 of the same MSR, not read by LHM), so on machines with an offset (typical on laptops) throttling starts below `tjMaxC`. | `service/OpenMonitorAdvanced.Service/Sensors/CpuIdentity.cs` | M6 (hardware matrix) |
| The disk critical warning exists only for NVMe (the DIT `CriticalWarning` attribute and the core's NVMe health log). A SATA rule from `SmartInfo.DiskStatus == Bad` (a CrystalDiskInfo-style heuristic, S1 §2) would be a separate rule, to be discussed. | `service/OpenMonitorAdvanced.Service/Sensors/` | to discuss |
| The machine schema fixture `crates/oma-core/tests/fixtures/this-machine-schema.json` holds this PC's Windows volume GUIDs and network adapter GUID (no serials or MACs). Regenerate it with hashed ids before the repository is pushed. | `crates/oma-core/tests/fixtures/` | before any push |

## Open: minor items from the M5b reviews

Small findings the task reviews accepted and deferred; none of them changes what the user sees today.

| Item | Where | Pick up |
|---|---|---|
| Settings store: one test swallows the shutdown error in all three cases; two negative assertions rely on a 100 ms sleep; the adopted `.tmp` test does not assert its removal after the save; repeated crashes before the first save accumulate `settings.json.tmp.bad-*` copies; `encode` of `rules` falls back to `{}` instead of failing loudly; one diagnostic per surplus custom rule (unbounded); non-rule load diagnostics stay after the user fixes the field. | `app/src-tauri/src/settings/` | when touched |
| Rule model: `Rule`, `RuleOverride` and `RulesSettings` lack `deny_unknown_fields` (a mistyped key in a hand-edited file or a patch is silently ignored); the trailing-`*` prefix meaning of `Selector.names` is undocumented; `validate.rs` checks the sensor kind through a `serde_json::Value`; untagged `Target`/`Threshold` give opaque errors; `resolve()` accepts `+8.9e1`. | `crates/oma-core/src/rules/` | when touched |
| Engine: the `bits_per_second` display key includes the byte component (invisible revision bumps); `health.rs` is long and its display-key formatter could be its own module; shrinking the interval can trigger one spurious suspend reset; `health_clock` before the first tick evaluates at monotonic 0; the zero-allocation guard does not cover a retained alarm in the Held/None steady state. Missing tests: a coverage-only change, below-equality with hysteresis 0, Held→Fresh slot recovery. | `crates/oma-core/src/rules/` | when touched |
| NVMe health log: `ERROR_INVALID_DEVICE_REQUEST` and similar are classified transient (retry every 30 s forever); the parser does not check the echoed protocol and data type; the `health_pick` wiring is untested; a duplicate `StorageDeviceProperty` query at discovery. | `crates/oma-win/src/storage_health.rs` | when touched |
| Service: `CpuCorePattern` also matches P-/E-core loads (harmless); a single-core Intel "CPU Core" is not mapped; no regression test for a stable device id when the NVMe flags appear; `HasCriticalWarning` is fixed at rebuild. | `service/OpenMonitorAdvanced.Service/Sensors/` | when touched |
| Shell: the catch-up snapshot read before the `applied` mutex can overwrite a newer change (`rules.rs`, same pattern in `interval.rs`); the Rust formatter drops the sign of -0; an evicted toast (more than 32 live) does nothing when clicked; a lost `Finished` on a full queue keeps the toast until it expires; the notifier's title fallback relies on `t()` returning the key. | `app/src-tauri/src/` | when touched |
| UI: "0 min" can flash until the clock of a new revision arrives; the default throughput hysteresis (3 B/s) shows as "0"; a failing `getRuleStatus` logs every second; `RuleRow` has its own switch instead of `controls/Toggle`; a stale doc comment in `rules.ts`; a few weak tests (the keyboard test clicks instead of pressing Enter, the Italian switch-label test checks the catalog only, no JSON test of a non-null `ThresholdSource`). | `app/src/` | when touched |

## Open: minor items from the M5c reviews

Small findings the task reviews accepted and deferred. The final fix wave already fixed the HRESULT mapping, `event.repeat`, the unit-change and overflow tests, the phase drift and drop count, the refused start clearing, the `format_number` allocation, the `cfg(test)` helpers, the `SensorTree` roles and the USB error codes. None of the items below changes what the user sees today.

| Item | Where | Pick up |
|---|---|---|
| Final review: a duplicate-hotkey error lands under the field the user did not edit (`log.hotkeyPause` is read first, so the `log.hotkeyDuplicate` fallback in `errorOf` is dead); the formula guard for cell values ignores a leading TAB or CR. | `app/src/` (`LogSection.svelte`), `crates/oma-core/src/csv.rs` | when touched |
| A writer failure does not log the raw OS error code (live check 9 saw only `failure=Unavailable`); log `raw_os_error()` so the next unmapped code is found without a repro. | `app/src-tauri/src/log/` | when touched |
| Spec M5 §2.3 wants the hotkey effects in `applyStatus`; they travel in `LogStatus.hotkeys` (requested, effective, state, reason), as the plan says (ruling R7). Same information for the UI; add an `applyStatus` entry only if something needs it. | `app/src/`, `app/src-tauri/src/hotkeys.rs` | accepted |
| Formatter and layout: `same_output` ignores the conversion (Flag and Count share the empty symbol); no rounding-boundary test (0.0005, 1.0005); the `retained_bytes` test bound is loose; no tests for the log-once, the -330 offset and the `checked_*` overflow of the local time; `Win32_System_Time` is out of alphabetical order in the `oma-win` `Cargo.toml`. | `crates/oma-core/src/csv.rs`, `crates/oma-win/` | when touched |
| Queue and writer: `retained_bytes` is computed twice per push under the lock; an oversized Start answers `Err` by reply with no `Failed` event, and `push_control` returns `Ok` for an unqueued control (document it); Pause/Resume of a session that is not open answers an English `Other` detail; the `pop(Some(timeout))` branch is untested; the paused-flush test name overpromises; a temporary directory leaks on failure in `fs.rs`. | `app/src-tauri/src/log/` (`queue.rs`, `writer.rs`, `fs.rs`) | when touched |
| Session: a tick meeting the barrier lock counts as dropped (a spurious `dropped` +1 and warning on pause or stop); the blocking `Mutex<OffsetCache>` on the sampler path and the blocking `push_control` of an in-session overflow; a barrier push failure leaves the admission closed (only reachable at exit); `miss_ticks` is off by one if `everyTicks` changes during a contended interval; the layout key uses the configured language, not the resolved one (see the `Language::System` check below); two tests start against a 100 ms real timeout and may flake under load; no tests for a mid-session overflow above 4096 columns, `open_log_folder` `folderMissing` and a shutdown while the serial is held. | `app/src-tauri/src/log/` (`session.rs`, `commands.rs`, `session/tests.rs`) | when touched |
| Tray and hotkeys: a Pause or Stop click racing an asynchronous writer error can toast twice; the `recording_dot` test loop starts at y=12 (could be 11); `set_menu` sets text on shared items right before the rebuild; a tautological assert after the swap in `hotkeys.rs`; a failed unregister is only logged, so a later re-register could read as `inUse`; `run_press` reads the state and then commands, so a UI or tray race can yield a started or stopped toast the hotkey did not cause (never a success on failure); `LogStatus.hotkeys` shows `unset` before the first apply; no test that an unreadable change keeps the old combination; the capture box shows the status line from before the suspension while it has focus; a double resume on removal (idempotent). | `app/src-tauri/src/tray.rs`, `tray_icon.rs`, `hotkeys.rs` | when touched |
| UI and mock: the `Settings` doc comment in `types.ts` now sits above `LogEveryTicks`/`LogSettings`, plus a double blank line near line 721; `mockSettings.ts` silences the destructured hotkeys with `void` and reports 10.5 as a range without a comment; the `maxFileMb` `as_f64().unwrap_or` in `decode.rs` is a dead fallback; the mock `logStart` while recording silently opens a new session and pause/resume/stop bump the revision on a no-op; no test for a command in flight during a reconnect. | `app/src/`, `app/src-tauri/src/settings/decode.rs` | when touched |
| Tape counter (`app/src/lib/log/tapeClock.svelte.ts`, fix `a47c7cc`): after REC or a resume it holds for about one sampling interval, until the first tick that adds recorded time (L3 counts nothing before it); on stop the deck settles back to the final value by less than one interval, because L3 does not count the time after the last tick. Both are coarser at a 5 s interval. | `app/src/lib/log/` | accepted |
| `cargo test -p oma-win -- --include-ignored` failed once (exit 101) in the final verification run and then passed 4 times in a row; the failing test was not captured. Probably a hardware test meeting a transient PDH status; rerun with the output kept when it happens again. | `crates/oma-win/` (hardware tests) | when it recurs |
| Recorder and pickers: Esc is caught only inside `.recorder` (with focus on the body it closes Settings and leaves the deck open); the keyboard-activation test only fires a click and nothing covers `reducedMotion` or its media change event; tabbing out of the deck leaves it open (approved behaviour); AltGr is reported as Ctrl+Alt; `SensorTree` has no arrow-key navigation (its tree roles were dropped, so it is a plain list of checkboxes); tests are missing for Esc `stopPropagation`, key repeat, the search filter and a rejected `pickLogFolder`; `setAllSensors` sends `[]` when the schema is null. | `app/src/` | when touched |

## Open: minor items from the M6a reviews

Small findings the task reviews accepted and deferred. The fix rounds already covered the `\z` anchors of the shim patterns, the independent GitHub context, the duplicate paths hidden by `7z x -y`, the silently skipped Integration run, the split release push and the stale `Cargo.lock` check. The items tied to the first signed run are under "Manual checks owed after M6a". None of the items below changes a release today; every one fails closed.

| Item | Where | Pick up |
|---|---|---|
| Shim: the plugin path pattern hard-codes `target\release\nsis\x64`, so a build with `CARGO_TARGET_DIR` or `--target` is refused; the "collect closed" message names only the import cause; a dangling junction or symlink skips the reparse-point check (`Test-Path` is false); a cleanup problem is lost when the inspection throws and the removal fails too; no test for the `IOException` path of `Invoke-OmaIoRetry`; `X509Certificate2Collection.Import(string)` is obsolete from .NET 9 (SYSLIB0057). | `scripts/lib/OmaSigning.psm1`, `scripts/lib/OmaCommon.psm1` | when touched |
| Version: the tag and version patterns end in `$`, which accepts a trailing newline (use `\z`); the tag comparison is case-insensitive, so `V0.3.0` passes (use `-cnotmatch`/`-cne`); the JSON version lookup needs a 2-space or tab indentation (it fails loudly otherwise); the rollback stops at the first failed restore and masks the original error; no test that the rewritten files keep their LF or CRLF endings. | `scripts/lib/OmaVersion.psm1` | when touched |
| Release notes and draft: `[IO.File]` gets relative paths, resolved against the process directory rather than `$PWD`; a CRLF body from GitHub receives an LF generated block (mixed endings, harmless) and nothing tests content after `generated:end` with CRLF; the CRLF to LF normalisation of the draft body has no comment and no test; cosmetic messages for a null status or conclusion and for a non-numeric size. | `scripts/render-release-notes.ps1`, `scripts/lib/OmaReleaseNotes.psm1`, `scripts/lib/OmaRelease.psm1` | when touched |
| CI gate: after *Re-run failed jobs* on `main` the gate can report jobs as missing. `docs/release.md` explains it; the message itself gives no hint. | `scripts/lib/OmaRelease.psm1` | when touched |
| Workflow: the run summary says "verified" without the attestation outcome; .NET is now installed in the `checks` job too, through the composite action (harmless). | `.github/workflows/release.yml`, `.github/actions/setup-toolchain/action.yml` | when touched |
| Docs: the setup name has dots in the README and in the asset (`OpenMonitor.Advanced_…`) and spaces in `CODE_SIGNING.md` (the name Tauri builds), with no line explaining it; the isolation rule for the Integration tests is repeated in four places; one overlong line in `docs/release.md`, and spec §6.3 step 1 is less precise than §6.2. | `README.md`, `CODE_SIGNING.md`, `docs/release.md`, `CLAUDE.md` | when touched |

## Open: deferred features (spec §5.2 point 5, §7.3; M3 decision D10)

- NVML `TotalEnergyConsumption` (p95 about 9 ms) and `PcieThroughput` (blocks 31 ms): only with sampling outside the tick. The M5c CSV log does not need them (it logs what the tick already has), so they wait for a request.
- Battery page: appears when a battery provider exists.
- Per-disk SMART switch and per-module switches are done (M5a); what is left for USB disks is the SAT fallback above.
- **Not covered yet, despite LibreHardwareMonitor exposing related sensors — do not assume the mapping surfaces them without re-checking `SchemaBuilder.cs`:**
  - **RAM SPD timings:** `SchemaBuilder.cs` (around the DIMM temperature match, see the comment there) explicitly discards the SPD timing and capacity sensors RAMSPDToolkit exposes on each DIMM. To check availability, enable PawnIO and dump a DIMM's `SensorNode`s: the timing values are present but currently thrown away, not absent from LHM.
- CPU throttling and the disk critical warning were settled in M5b: see the limits in "Open: code" (throttling has no LHM sensor; the critical warning is NVMe only).

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

## Manual checks owed after M6a

Owed after the SignPath Foundation approves the project (spec M6a §8.2). The application is deferred (2026-10-01): the form requires proof that the project is widely used, and the repository had been public for two days. None of this runs on the dev PC: use Windows Sandbox or a VM, started by the user.

- `workflow_dispatch` rehearsal with `test-signing`, including `verify-signatures.ps1 -Policy test` on the hosted runner.
- One release with `release-signing`, with `REQUIRE_SIGNING=true` set before the first signed publication (`docs/release.md`).
- Install the signed setup and check the signatures of `oma-app.exe`, `oma-service.exe` and `uninstall.exe` in `$INSTDIR`, plus the publisher in the UAC prompts of the install and of the uninstall.
- SmartScreen behavior on a downloaded signed setup; the README line stays as is until then.
- SmartScreen and UAC on a downloaded **unsigned** 0.3.0 setup on a normal Windows (the Sandbox check could not show them: no Mark of the Web and no UAC prompt there).

To settle on the first signed run (deferred from the M6a reviews):

- bind the binaries returned by SignPath to the submitted ones through the Authenticode PE digest (today SignPath is the only trust anchor for that step);
- the `timeout-minutes: 100` of the two SignPath steps against the action's own worst case (about 105 minutes) and the 240-minute job budget;
- the semantics of `file-version` in the SignPath artifact configurations;
- the uninstaller directory match when the runner's `TEMP` is a short 8.3 path (`RUNNER~1`);
- the signtool line "The signature is timestamped:" that the timestamp count relies on, on a SignPath-signed file with an RFC 3161 timestamp;
- `retention-days` of the unsigned `binaries-*`/`setup-*` run artifacts (90 days by default).

## Manual checks owed after M5c

- PC suspend and resume, and a daylight-saving change, during a recording (Review Focus 4): the timestamps, the offset and the pause gap. Not live-tested.
- `Language::System` with the OS language changed in the middle of a recording: the column labels keep the old language until the layout key changes (see the minor items).
- A real game with anti-cheat and the global hotkey (Ctrl+Alt+Shift+R): it is registered through `RegisterHotKey`, so it should not hook the game, but it was not tried with one.

## Manual checks owed after M5b

- PC suspend while a rule is maturing: no spurious alarm on resume (deferred by the user on 2026-09-30).
- HDD standby over 5–10 minutes: the banner and the coverage while the disk sleeps (known limit R-D above), together with the M5a spin-down check.
- A USB volume removed while its `volume-used` rule is active: the coverage and the retained alert.
- Toast click while the window is loading and from the notification centre, and the fallback for a removed device (the window-open and window-closed cases passed on 2026-09-30, with the right AUMID).
- TjMax on an Intel CPU and on AMD models other than this 7800X3D; the NVMe critical warning with a real warning bit set.

## Manual checks owed after M5a

- PawnIO scenarios in a VM: driver stopped, uninstalled, and the 3010 reboot state (`rebootPending`, then `ok` after the reboot); the Win32 codes 2/3/5 of the probe live. `ok` was checked on the dev machine.
- Autostart across a real logout/login (the Run value and the Task Manager enable/disable states were verified; the login itself was deferred).
- Per-module switches with two clients (two user sessions): the module stays on while one of them wants it.

## Closed in M6a

- Installed uninstaller (2026-10-01, Windows Sandbox, by the user): the `uninstall.exe` installed by `target/spike/B/setup-B.exe` has SHA-256 `2AE8939B5D09B3C39FAD95633E7D751485C97529621D3815D48F4844402B2E68`, the marked copy, so makensis embeds the replaced uninstaller.
- First hosted run of the `scripts` job with the Integration tests and of `actionlint` (2026-10-01): green on `eb18e07`, which fixed the test PFX.
- `workflow_dispatch` rehearsal without secrets (run 36842561268, `eb18e07`): setup and `SHA256SUMS.txt` as a run artifact, checksum checked by hand, no release created.
- First unsigned release with the new flow: 0.3.0, tag `v0.3.0` on `70bda0b` (run 36844320817), draft checked (checksum, build provenance attestation) and published by the user on 2026-10-02.
- `nsExec.dll` and the other NSIS plugins ship unsigned: closed as third-party code that we must not sign (SignPath terms; design decision D5). `PawnIO_setup.exe` keeps its author's signature. Only the installer, `oma-app.exe`, `oma-service.exe` and `uninstall.exe` are to be signed.
- Release pipeline, signing shim and verification scripts are implemented and covered by Pester; the signed path stays conditional on the Foundation's approval (see the owed checks above).

## Closed in M5c

- CSV sensor log (spec M5 §2): the tape recorder in the top bar, the tray items and the recording dot, global hotkeys (toggle and optional pause), Settings › Log CSV with the folder, sensors, interval and size limit, and the writer thread with parts, error states and a clean stop at exit.
- `smartSelectable` stays out of the CSV: the log metadata never reads it.
- CPU chart gaps from PDH (live check, 2026-09-30): `PDH_CALC_NEGATIVE_DENOMINATOR` no longer leaves a hole. The effective clock (`% Processor Performance`, whose `_Total` raw base now and then goes backwards) and the loads (`% Processor Utility`, whose 32-bit base wraps every 2^32 / ~625 kHz, about 1 h 54 min 32 s, which made the whole CPU provider degrade, rediscover and log a WARN) repeat their last valid values for at most 3 consecutive polls (so 3 × the sampling interval: 15 s at 5 s), marked `Quality::Held` through `Provider::repeated`, then read as absent; other PDH errors keep the old behaviour. Each case logs at INFO at most once an hour, with the status and a `suppressed` count (`crates/oma-win/src/cpu.rs`).
- Tape counter: it advances once a second while recording, run on locally between the core's paced `oma:log` statuses (fix `a47c7cc`).
- Live checks (2026-09-30, dev machine, installed build): recording, pause, resume and stop; the file in Excel; the tray items; the hotkey with the window closed; the capture box in Settings; a combination held by another process; the folder dialog; a USB stick pulled while recording (toast, red triangle, reason); a restart after the error; new parts after a unit and a language change; exit from the tray with a complete last row. The HotkeyInput fix `92ad46f` came from the user's check. Budget in `docs/perf-budget.md` (M5c, the final M5 measurement).

## Closed in M5b

- A leftover `settings.json.tmp` is recovered at load, and a test ties `encode`, `decode_lenient` and the patch schema together (Task 1).
- `Settings` is no longer deep-cloned on every tick: `snapshot()` shares it as an `Arc` (Task 1).
- Rules engine (spec M5 §3): default and custom rules and overrides with validation and a backup of an invalid file, the health report and the banner, the coloured tray icon with the verdict in its tooltip, and Windows toasts with a 5-minute cooldown whose click opens the device page.
- CPU TjMax (Intel, AMD desktop table), Tdie and Intel core names in the service; NVMe wear, spare and critical warning read by the core without privileges, and the critical warning also from the service.
- Live checks (2026-09-30, dev machine, installed build): `cpu-temp` at 79/89 °C "from TjMax", also with the service stopped; the "all fine" banner; a custom rule taking banner, tray and tooltip to critical with a toast titled "<device> · <sensor>" and "less than a minute" in the banner; toast click with the window open and closed; one toast in 5 minutes; service stopped during a CPU temperature alarm (alert retained as "data unavailable", no second toast when it returns); °F thresholds and hysteresis round trip; "Create rule…" from the Advanced view; NVMe wear and critical warning with the service stopped. Budget in `docs/perf-budget.md` (M5b).

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
