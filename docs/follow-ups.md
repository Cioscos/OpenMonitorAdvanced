# Follow-ups

Items consciously left open, with where they live and when they are expected to be picked up.
Updated at the end of every milestone (last update: M7d).

## Open: code

| Item | Where | Pick up |
|---|---|---|
| The service logs no SAT detail at Information level: the M6b live checks had to read the SCSI status and the sense bytes from the probe (`target/spike/m6b/sat-probe.ps1`), which showed SCSI status 0x00 with an ATA Status Return descriptor (`72 .. 09 0C`) in the sense buffer for both STANDBY IMMEDIATE and CHECK POWER MODE. Log once per drive and route (Debug or once at Information) the SCSI status, the returned bytes and `SenseInfoLength`, so the next bridge can be diagnosed from the log alone. | `service/OpenMonitorAdvanced.Service/Sensors/DiskPowerProbe.cs` (SAT path) | when touched |
| Report the malformed `{product_name}`-style placeholders of the Tauri NSIS template to Tauri upstream (the upgrade behaviour itself is closed, see "Closed in M7a"). SmartScreen also appears on an unsigned setup, as expected. | `app/src-tauri/nsis/` | upstream report: when convenient |
| On a machine with more than one interactive user, any of them can stop `oma-service` for the others: the service has no notion of "who asked". | `app/src-tauri/src/service.rs` | accepted |
| DDR5 SPD page stays on whichever page it was left on (e.g. page 4) after the service stops, instead of resetting; stock LHM behaves the same way (found in Task 15, 2026-09-27). Optional bounded reset for parity, otherwise accepted. | `app/src/` (RAM/SPD page) | accepted; revisit if a user reports it |
| Confirm on the first real SignPath request: whether the `file-version` constraint of the artifact configurations matches the string `X.Y.Z` or the fixed `0.2.0.0` resource of the NSIS files; that the Integration Pester tests actually run (not skip) in the CI `scripts` job; the 8.3 short `TEMP` path of the runner against the uninstaller directory check of `sign-shim.ps1`; the English line `The signature is timestamped:` of signtool with SignPath's RFC 3161 timestamps. | `.signpath/`, `scripts/verify-signatures.ps1`, `scripts/sign-shim.ps1`, `.github/workflows/ci.yml` | first signed release |
| A third-party folder inside `Program Files` that grants `Users` Modify rights passes the Advanced-sensors-component install-path check, which only verifies the path is under `Program Files` and free of reparse points, not its ACL. | `app/src-tauri/nsis/oma.nsh` (install-path check) | accepted; verify/document |
| A silent install refused for a reason other than the path check (for example a missing `/NOSENSORS` outside `Program Files`) may still have created an empty `$INSTDIR` and installed the WebView2 runtime before refusing. | `app/src-tauri/nsis/installer.nsi` | accepted; verify/document |
| Redistributing the official PawnIO setup is common practice (LibreHardwareMonitor and FanControl both do it), but the PawnIO author has not been asked to confirm it for this project. The request is drafted below ("Draft: request to the PawnIO author"), to be sent only on the user's request. | `THIRD_PARTY_NOTICES.md`, `scripts/build-installer-payload.ps1` | before 1.0 |
| The label-key test keeps a hand-written list: only GPU keys are cross-checked against the code (`GpuField` self-test); CPU, memory, storage and network keys are not. | `crates/oma-win/tests/labels.rs` | when touched |
| The CSP has no `devCsp` with `ws://localhost:1420`, so Vite hot reload inside `pnpm tauri dev` may be blocked. | `app/src-tauri/tauri.conf.json` | when touched |
| NVML is not initialised again after the NVIDIA driver is updated or unloaded while the app runs; its fields fall back to D3DKMT until a restart (README, "Known limits"). | `crates/oma-win/src/gpu/nvml.rs` | M6 |
| A disk identified only by its PnP instance id (no serial, no unique GPT or MBR id) gets a new id when it is moved to another port: its history and statistics restart. | `crates/oma-win/src/storage_identity.rs` | accepted |
| Intel and other GPUs whose PnP maximum-link read fails show no maximum link at all: `pcieMaxGen`/`pcieMaxWidth` come only from the PnP base layer (device capability, identical in safe mode), because NVML's max-link calls report the device+slot-limited value and IGCL does not read `ctlPciGetProperties`. Not yet exercised on Intel or non-NVIDIA hardware. | `crates/oma-win/src/gpu/pnp.rs` | M6 (hardware matrix) |
| Discovery reads every disk's temperature in one tick; with several NVMe drives waking from a low-power state this can exceed the 200 ms tick budget, so the value arrives one tick late. | `crates/oma-win/src/storage.rs` | when touched |
| `gpu/pnp.rs` `display_interfaces` has no retry on `CR_BUFFER_SMALL`: a GPU hot-plugged between the two calls gets no maximum link on that discovery. | `crates/oma-win/src/gpu/pnp.rs` | M6 (hardware matrix) |
| Re-identification at hot-plug. DiskInfoToolkit's hot-plug thread re-identifies the not-yet-identified disks on every `DBT_DEVNODES_CHANGED`, which can wake them (F1.1). Declared limit since M6b (spec M6b §8, decision D4): no code on our side; the upstream report is drafted below ("Draft: DiskInfoToolkit upstream report"). | `service/OpenMonitorAdvanced.Service/Sensors/` (storage) | upstream report, published only on the user's request |
| Over-the-shoulder elevation (setup started by a standard user, elevated with another account) gives no relaunch of the app in silent or passive mode: the elevated account is not the one that had the app open. | `app/src-tauri/nsis/oma.nsh` | when touched |
| GUI reinstall of the same version: the page that closes the app can look frozen for up to 10 s (the wait for `--quit`), and a Cancel after the app was closed leaves it closed. `OmaCloseApp` also waits 10 s when `OmaAppWasRunning` is empty (app only in another session) and leaves the error flag set when `DisplayVersion` is missing (add `ClearErrors`). | `app/src-tauri/nsis/oma.nsh` | when touched |
| `oma-app.exe --quit` with no instance running holds the single-instance mutex for a few milliseconds: a normal launch in that instant is lost. Its process behaviour (no window, no tray, no Run write) has no automated test: U7 checks it. | `app/src-tauri/src/main.rs` | accepted; U7 |
| A rapid off/on of the memory module may delay the SMBus through the `~SPDAccessor` finalizers (F2.3). Measure when the module is switched back on quickly. | `service/OpenMonitorAdvanced.Service/Sensors/ModuleApplier.cs` | when touched |
| The installer is perMachine, so the uninstaller deletes the HKCU Run value in the elevating admin's hive: a standard user's own Run value survives (documented in the README, "Known limits"). | `app/src-tauri/nsis/oma.nsh` | accepted |
| A second launch opens the settings store before single-instance exits it, and the interval listener may take the engine lock on the main thread. | `app/src-tauri/src/main.rs` | when touched |
| `SvcProvider::poll` clones the whole snapshot and the drive list every tick only to compare generations; generation accessors would avoid the per-tick allocation. | `crates/oma-win/src/svc/provider.rs` | when touched |
| Pipe listener: after a failed connect, the replacement instance is created after the old one is disposed and without `FILE_FLAG_FIRST_PIPE_INSTANCE`, a brief zero-instance gap (the client-side PID check protects the app). Cheap hardening: create it with `first: Volatile.Read(ref _busy) == 0`, so a squatter makes it fail loudly (R20) instead of being joined. | `service/OpenMonitorAdvanced.Service/Pipe/PipeListener.cs` | when touched |
| The service's logs live in `$INSTDIR\service\logs` (ruling R30). Any future service-owned path (settings, rules) must stay under a folder no user can create first, with the same service-side check (`LogDirectoryGuard`), never under `%ProgramData%`. | `service/OpenMonitorAdvanced.Service/Logging/LogDirectoryGuard.cs`, `app/src-tauri/nsis/oma.nsh` | M5 (settings) |
| Update check: WinHTTP does not check certificate revocation (M6c ruling). Revisit if the check ever downloads anything. | `crates/oma-win/src/http.rs` | accepted |
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

## Limits declared in M6b (spec M6b §8)

What the disk work of M6b knowingly does not cover. The user-visible ones are also in the README ("Known limits").

- **A USB hard disk in standby behind a bridge was not verified** (no such hardware). For this reason the SMART of USB disks (bus 0x07) is off by default; the user can switch it on per disk, with the warning that some adapters do not report standby.
- **DiskInfoToolkit re-identification at hot-plug** can wake a disk the library cannot identify, when it is plugged in after SMART was switched on (see "Open: code" and the draft report below).
- **An idle HDD is not read.** Without the service its temperature is not updated while the disk does no work; with the service, its temperature and SMART are not updated either until the disk works again. The last value stays visible, muted, as «Last reading».
- **A standby chosen by the disk itself** (firmware timer, APM), which Windows does not know about, shows as «Idle» and not as «In standby» until the disk works: no passive signal tells them apart, and asking the disk would keep Windows from turning it off.
- **An HDD asleep when the service starts keeps the D6 gate closed**: until it wakes, no disk's SMART is published, NVMe included (live check, prova 2, 2026-10-02: schema with 0 devices until the user touched `D:`). By design: LibreHardwareMonitor's first identification touches every disk.
- **A keyless HDD, or one whose read/write counters cannot be read**, is read in the first rounds of a storage episode only (Task 10): without counters the service sees no activity.
- **A default-off (USB) disk that another client switched on, and that this client cannot bind** (twin USB disks, a hint mismatch), is not filtered locally (Task 11).
- **Retries that keep a disk awake.** While `EnableStorage` keeps failing, and for a gate blocker without readable counters, the service asks the power-check disks every 5 minutes (`GateEpisode.BlindRetry`, spec-mandated): with Windows' default 20-minute disk timeout those disks never spin down while the condition lasts.
- **Outside our code.** In the spike, with TR-VISION HOME (`WFanManager.exe`) open and its `DISPLAY`/`SYSTEM` power requests present, Windows did not turn the disk off; after closing it, it did. An observed interference, not a proven cause: not every `SYSTEM` request has this effect. Windows' own spin-down latency with a 60 s disk timeout was 3 min 44 s to about 5 min 45 s in the M6b checks, with or without our software.

## Open: minor items from the M6b reviews

Small findings the task reviews and the final review accepted and deferred. The final fix wave already fixed the lost main temperature of a bound SSD/NVMe (I1), the one-tick dropout of every service sensor on a drive-table change (I2), the blank SMART of a quiet HDD after resume (I3), the "idle" state in the fixtures README, `smartOn` for a disk in both lists and the `DevicePage` quality wiring test; Tasks 16 and 17 widened the service's activity window and kept suspended readings out of history, statistics and the CSV log. None of the items below changes what the user sees today, unless noted.

| Item | Where | Pick up |
|---|---|---|
| UI: `seedHistory` resets quality to `[]`, so a suspended sensor reads fresh until the next tick; one tick with an empty current value after a history seed; the state tag sits on its own row above the KPIs (the heading belongs to `AdvancedView`); the `.tag` CSS is duplicated from `SensorTable` and the stale rule from `KpiRow`; the tray icon can show a chosen disk temperature as current while it is suspended. | `app/src/`, `app/src-tauri/src/tray_icon.rs` | when touched |
| History and log (Task 17): the mock backend's statistics come from raw snapshot values (they would diverge if the mock suspended a value); no assert pins `size_of::<Cell>()`; no allocation guard on the history/log path; a decimated window widens the gap to the bucket boundary. | `app/src/` (mock), `crates/oma-core/` | when touched |
| Core storage, decisions: `idle` from the service overrides local activity in the label (`decide` returns `DiskPower::Idle` with recent activity, so the page can say «Idle» for up to a round while its own throughput shows I/O; reporting `Active` there would read better); a rediscovery at a 5 s interval closes the activity window (R7's benefit is lost at the longest interval only); one tick without authority on a drive-table change (a `Standby`→`Idle` flap, safe side); the disk class is cached for the life of an id, so a transient failed seek-penalty query freezes an SSD as rotational until restart; `parse_seek_penalty` ignores `Version`/`Size`. | `crates/oma-win/src/storage.rs`, `storage_gate.rs` | when touched |
| Core storage, temperatures: the main sensor's `Source` stays `Win32` when the value comes from the service (wrong source tag in the table); an import overwrites a newer local reading under standby/idle without comparing ages; `forget_another_disk` fires on a transient `None` key; two constants named `MAIN`; the `published()` test does not cover `poll`'s assembly order; `storage.rs` is about 2600 lines (`DiskGate`/`DiskTemperatures` are ready for their own module). | `crates/oma-win/src/storage.rs`, `svc/drives.rs` | after the merge (split) / when touched |
| Core, per-poll cost: `powered_on` opens a handle per disk per poll and the power check opens two (`CreateFileW`) per powered-on disk (open once per poll and share it; keep reopening per poll rather than holding a handle, for USB safe removal); a clone of `DriveIds` and a full scan in `service_disk` per disk; `disk_states.get()` clones the list every tick in `main.rs`. Within budget (V7). | `crates/oma-win/src/storage.rs`, `app/src-tauri/src/main.rs` | when touched |
| Core `svc` and IPC: `standby_sensors` repeats `bind`'s work; `refresh_sources` matches quadratically by `ptr::eq`; no provider-level test for a non-associated standby drive; the `translated_keys` test sits in `provider.rs`; two over-long doc lines; the Rust decoder does not check that a drive key is 64 lowercase hex (C# does); no test that `smart_enabled_drives` is truncated to `MAX_DRIVE_KEYS`; no test that a v2-shaped snapshot (no `held`) is rejected; the `Provider::quality()` doc omits the length contract; the `instance.rs` "suspend gaps reset the timers" wording (means system sleep gaps). | `crates/oma-win/src/svc/`, `crates/oma-ipc/`, `crates/oma-core/` | when touched |
| Service, drive list and requests: no freshness bound on `drives` when `Enumerate()` throws (a stale `standby` would persist; safe for waking); an incompletely described disk is described a second time per round; `CheckPowerStates` duplicates `CheckDrives`' loop; a storage request stays pending for its round and can read `failed` past `ReconfigureTimeout` on a slow round, and a throw before publishing stays pending while "cannot list drives" reports applied; the first schema after an idle hub carries the old drive states until the immediate round ends; an enumeration failure keeps the previous activity reference (harmless, undocumented, untested); `TakeBaseline`/`_baseline` against "reference" naming. | `service/OpenMonitorAdvanced.Service/Sensors/SensorHub.cs` | when touched |
| Service, gate and power checks: keyless answers are shared at the same drive number and unkeyed identities can alias across a hot-swap (fails safe); a flapping identity re-arms the one-shot at every flap; a client in a reconnect loop makes the service ask the disks at its reconnect rate (add a minimum spacing between episode starts); a drive stuck on failed power-state calls after an "off" stays `standby` until Windows answers "on"; an alternating late/on-time round pattern re-arms about every 90 s (the 5-minute limit covers only consecutive late rounds; pathological); `held` is per snapshot, not per subscriber (single-client assumption). | `service/OpenMonitorAdvanced.Service/Sensors/SensorHub.cs`, `DiskActivityProbe.cs`, `DiskPowerProbe.cs` | when touched |
| Service, code and tests: `SensorHub.cs` is about 1900 lines (`GateEpisode` could have its own file); the `SatSense` doc comment lacks `<summary>`; missing tests for the encoder's non-finite/held clamp, a drive missing `physical_drive`/`state`/`blocks_smart`, the both-lists message in `PipeListenerTests`, `EnableStorage` throwing, a drive vanishing from the enumeration and `ToWire`/`AllOff`; the adapted concurrent test no longer covers a kept value across the tick boundary; `EveryDriveIsListedWhileTheGateIsClosed` has an intermediate assertion that cannot be attributed; `HeldOf`/`ValueOf` helpers duplicated; `Generation` doc wording. | `service/OpenMonitorAdvanced.Service/`, `service/OpenMonitorAdvanced.Service.Tests/` | after the merge (split) / when touched |

## Draft: DiskInfoToolkit upstream report

To be published on `github.com/Blacktempel/DiskInfoToolkit` **only on the user's request**. Written from a code reading of DiskInfoToolkit 1.1.2 (commit `25319ea`, the one in its `.nuspec`; analysis in `docs/superpowers/references/m5/f1-service-reconfiguration.md`, F1.1), not reproduced on hardware: we have no disk that the library fails to identify.

> **Title:** Disks that fail identification are re-identified (with a sector-0 read) on every `DBT_DEVNODES_CHANGED`
>
> Hi, and thanks for DiskInfoToolkit. We use it through LibreHardwareMonitor in a hardware monitor that tries hard not to wake sleeping hard disks, and we found a path that can.
>
> **What happens.** The static constructor of `StorageManager` starts a `WM_DEVICECHANGE` listener thread. On every `DBT_DEVNODES_CHANGED`, which Windows sends for any PnP change (a USB device, a Bluetooth adapter, a monitor), `HandleUnpartitionedDrive` re-examines every disk and builds a `new Storage(...)` for each drive number that is not in `_Storages` (`StorageManager.cs`, around lines 336-472, "added" at 426-435). A disk whose identification fails (`IsValid == false`) never enters `_Storages`, so it is identified again at every devnode change. For a disk on a bus other than ATA/SATA or NVMe (USB included), `DeviceIdentifier` calls `DiskHandler.WakeUp(handle)` unconditionally before the SAT and vendor attempts (`DeviceIdentifier.cs:127`), and `WakeUp` reads sector 0 (`DiskHandler.cs:228-234`).
>
> **Effect.** A hard disk behind a USB bridge that the library cannot identify is spun up again by any unrelated PnP event, and Windows' idle timer for it is reset, for as long as the process runs. The user sees a disk that never stays asleep.
>
> **Possible fixes.** Remember the disks whose identification failed (by device instance path, or drive number plus descriptor) and do not retry them on `DBT_DEVNODES_CHANGED`, only when that disk arrives again (`DBT_DEVICEARRIVAL` for its disk interface) or with a long back-off; or skip `WakeUp` on retries; or offer an option to turn the hot-plug thread off, or a filter callback that runs before a disk is opened.
>
> Found by code reading, not reproduced: we do not own a disk that fails identification. Happy to test a change on the hardware we have.

## Draft: request to the PawnIO author

To be sent **only on the user's request**, as an issue on `github.com/namazso/PawnIO.Setup` or by e-mail to the address in the setup's signature. The author's confirmation is a requirement of the 1.0 release (main spec §9, §14).

> **Title:** Permission to redistribute the unmodified PawnIO setup with OpenMonitor Advanced
>
> Hi, and thanks for PawnIO. I maintain OpenMonitor Advanced (https://github.com/Cioscos/OpenMonitorAdvanced), a free, open-source hardware monitor for Windows (GPL-3.0-or-later). Its optional sensor service uses LibreHardwareMonitorLib 0.9.6, which reads CPU, motherboard, RAM SPD and fan sensors through PawnIO.
>
> **What we do.** Our NSIS installer has an optional "Advanced sensors" component (selected by default; it can be cleared, and `/NOSENSORS` leaves it out of a silent install). When it is selected and PawnIO is missing or older than 2.2.0, the installer runs the official `PawnIO_setup.exe` 2.2.0 with `-install -silent`, exactly as published at https://github.com/namazso/PawnIO.Setup/releases/tag/2.2.0. We never modify, repack or re-sign the setup, and we ship no PawnIO module of our own.
>
> **How we check it.** The setup is never committed to our repository. At build time it is downloaded from your release (or read from a cached copy) and accepted only if:
>
> - its SHA-256 is `1F519A22E47187F70A1379A48CA604981C4FCF694F4E65B734AAA74A9FBA3032` (pinned in our repository);
> - its Authenticode signature is valid and the signer is `E=admin@namazso.eu, CN=namazso.eu, O=namazso, L=Debrecen, C=HU` (issued by GLOBALTRUST 2015 CODESIGNING 1), with the certificate thumbprint pinned as well.
>
> Our uninstaller never removes PawnIO, since other programs (such as FanControl) may share it. Our third-party notices credit PawnIO and state its licences (driver GPL-2.0 with the IOCTL exception, library and modules LGPL-2.1).
>
> **The question.** LibreHardwareMonitor and FanControl redistribute the setup in a similar way, but we would rather ask than assume: may we keep redistributing the official, unmodified PawnIO setup inside our installer as described? If you prefer different terms (a download at install time instead of bundling, a specific attribution, a minimum version), we will follow them.
>
> Thank you.

## Draft: release notes for 0.4.0

The hand-written part of the 0.4.0 draft release (above the block that `render-release-notes.ps1` generates), to be pasted into the draft **only when the user prepares the release**.

> ## What's new
>
> ### Disks that are left to sleep (M6b)
>
> - **Hard disks can spin down.** The app reads a hard disk's temperature only after recent activity, and the sensor service no longer asks a disk that Windows turned off, or that has been idle since the previous round, for its power mode or SMART data. Before, these queries woke the disk or reset Windows' idle timer.
> - **Disk power state** on the disk page: *Active*, *Idle* or *In standby*. While a disk is idle or asleep its last values stay visible, greyed out, as *Last reading*; they are left out of the history, the statistics and the rules, and the CSV log writes `suspended`.
> - **USB disks:** SMART reads are off by default and can be switched on per disk in *Settings › Data sources*. Power-mode checks fall back to SAT pass-through for disks behind a USB bridge.
> - *Data sources* names the disks that hold back SMART reads while they sleep.
>
> ### Updates, sensor report and licences (M6c)
>
> - **Optional update check.** *Settings › About* has a *Check now* button and a *Check automatically (once a day)* option, **off by default**. The check is a single HTTPS request to `api.github.com`; it sends your IP address and the app version, nothing else, and it never downloads or installs anything. With the automatic check on, a new version raises one Windows notification and a dot on *About*. See the privacy section of `CODE_SIGNING.md`.
> - **Export sensor report.** An anonymous JSON file with devices, sensors, sources and values, to attach to bug reports. Disk and network identifiers, adapter names and volume GUIDs are replaced; nothing is sent.
> - **Licence texts.** `THIRD_PARTY_LICENSES.txt` lists the licences of every redistributed Rust crate, JavaScript package and NuGet package and of the .NET runtime; it is installed with the app and opens from *About*, next to the third-party notices.
> - The network adapter page shows the adapter type (Ethernet or Wi-Fi).
>
> ### Fixes
>
> - Italian installer texts: the "app is running" messages no longer show a raw `{product_name}` placeholder.
>
> ### Upgrading
>
> **App and service must be the same version (protocol v3); the installer updates both.** Do not mix a 0.4.0 app with a 0.3.0 service or the reverse: the app reports the service as a different version until both are updated.
>
> This release is not code-signed yet: Windows SmartScreen may warn you (see the README, "Verify your download").

## Draft: release notes for 0.4.1

The hand-written part of the 0.4.1 draft release (above the block that `render-release-notes.ps1` generates), to be pasted into the draft **only when the user prepares the release**.

> ## What's new
>
> ### Upgrading without prompts (M7a)
>
> - **The installer closes the app for you.** Running a newer setup over an installed version no longer shows the "product is still running" message. The installer asks the running app to quit, waits up to 10 seconds, forces it closed only if it does not, and starts it again afterwards (with *Run* ticked in the interface, or minimized to the tray in a silent install). The service restarts with the app. Uninstalling from *Settings › Apps* still asks.
> - Upgrading **from 0.4.0** closes the app by force, because 0.4.0 does not know `--quit`; from 0.4.1 on the app quits in an orderly way. `oma-app.exe --quit` can also be run by hand.
> - *Start with Windows* survives the upgrade, also with *Uninstall before installing* chosen in the interface.
> - *Start with Windows* repairs itself: if the program moved and its old location is gone, the entry is rewritten at the next start. Another copy that still exists, such as a portable one, leaves the entry alone.
>
> ### Fixes
>
> - **Charts of a disk in standby** keep the y-axis and say "No readings while the device is idle or in standby" instead of showing a blank plot.
> - **Opening a file or folder** (log folder, licences, report) shows a clear message when it does not exist, and no longer hangs the app if the shell is slow.
> - **Disk names** no longer carry trailing spaces or control characters in the log and in the sensor list.
> - The service link no longer queues commands without limit. After an incompatible version, a service that is then uninstalled or stopped is reported as such ("…the OpenMonitor Advanced service, which is not installed" or "The sensor service is not running or not responding.") instead of the version mismatch.
> - `THIRD_PARTY_LICENSES.txt` now includes the notices of the Microsoft NuGet packages.
>
> ### Upgrading
>
> **App and service must be the same version (protocol v3); the installer updates both.**
>
> This release is not code-signed yet: Windows SmartScreen may warn you (see the README, "Verify your download").

## Draft: release notes for 0.5.0

The hand-written part of the 0.5.0 draft release (above the block that `render-release-notes.ps1` generates), to be pasted into the draft **only when the user prepares the release**.

> ## What's new
>
> ### In-game overlay (M7b, M7c)
>
> - **FPS, frame times, lows and sensors over your game**, drawn by a separate click-through window (`oma-overlay.exe`) that never touches the game: no injection, no hook, no input sent. Frame data comes from Intel's PresentMon 2.6.0, installed next to the service with its SHA-256 checked at every start.
> - Displayed, rendered and presented FPS, frame time chart, 1% and 0.1% lows, stutter, frame generation multiplier, PC and display latency, CPU or GPU bound, and any sensor of the app. Four built-in profiles: *Minimal FPS*, *Gaming*, *Full* and *Horizontal bar*.
> - Global shortcuts (not set by default) to show or hide the overlay and to change profile; a profile per game; games you can exclude.
>
> ### Editor and your own profiles (M7d)
>
> - **A profile editor** (*Settings > Overlay > Open the editor*): drag and resize blocks on a canvas with live values, set anchors, fonts, colours, thresholds and *visible if* rules, undo and redo, and a **preview** window that draws the profile exactly as in game.
> - Duplicate the built-in profiles, import and export profiles as `.omaoverlay.json` files (checked, with a new id and a unique name), and *Use now* to try a profile in game. Unsaved changes are never lost without a question.
>
> ### Benchmark (M7d)
>
> - **A global shortcut starts and stops a capture** of the game in the foreground, up to 60 minutes, with `REC` on the overlay and a summary box at the end. Each capture writes a CSV with one row per displayed frame and a JSON summary in the `benchmarks` folder of the log folder; the settings page lists, opens and deletes them.
>
> ### Fixes
>
> - `meter` and `gauge` blocks with an automatic scale follow the last minute instead of the peak of all time.
> - Errors of *Try again* and *Reload profiles* in the Overlay settings are shown.
>
> ### Limits
>
> - **Exclusive fullscreen:** the overlay is not visible there (use borderless); the measurement works.
> - **Frame generation without Reflex:** the rendered FPS are unknown for DLSS FG and Smooth Motion (the overlay shows *FG?*) and for FSR FG.
> - **Memory in game:** the overlay uses about 65 MB, almost all of the graphics driver.
> - The editor canvas is not pixel-exact for fonts; the preview is. The benchmark needs the overlay turned on.
>
> ### Upgrading
>
> **App, service and overlay must be the same version (service protocol v4, overlay protocol v2); the installer updates all of them.**
>
> This release is not code-signed yet: Windows SmartScreen may warn you (see the README, "Verify your download").

## Open: deferred features (spec §5.2 point 5, §7.3; M3 decision D10)

- NVML `TotalEnergyConsumption` (p95 about 9 ms) and `PcieThroughput` (blocks 31 ms): only with sampling outside the tick. The M5c CSV log does not need them (it logs what the tick already has), so they wait for a request.
- Battery page: appears when a battery provider exists.
- Per-disk SMART switch and per-module switches are done (M5a); the SAT fallback for USB disks is done (M6b), with the SMART of USB disks off by default.
- **Not covered yet, despite LibreHardwareMonitor exposing related sensors — do not assume the mapping surfaces them without re-checking `SchemaBuilder.cs`:**
  - **RAM SPD timings:** `SchemaBuilder.cs` (around the DIMM temperature match, see the comment there) explicitly discards the SPD timing and capacity sensors RAMSPDToolkit exposes on each DIMM. To check availability, enable PawnIO and dump a DIMM's `SensorNode`s: the timing values are present but currently thrown away, not absent from LHM.
- CPU throttling and the disk critical warning were settled in M5b: see the limits in "Open: code" (throttling has no LHM sensor; the critical warning is NVMe only).

## Manual checks owed by a human

- Tray left click and the "Open" menu item re-create the window after it was closed (M1).
- USB disk hot-plug keeps or changes disk ids correctly (M1). Since M3 also: a disk without a serial number (for example a VHDX mounted by an administrator) appears with a `storage/gpt-…` id and keeps it across a restart.
- IGCL telemetry and PCIe link on Intel hardware, including `ctlPciGetState` layout and per-tick cost (unmeasured, no Intel hardware available); ADL on a dedicated Radeon (hardware matrix, spec §12).
- An MBR disk or a VHD without a serial number gets a `storage/mbr-…` id: the MBR identity tier and the geometry IOCTL were never exercised on real hardware (every disk in the M3 hardware matrix is GPT).
- Anti-cheat compatible mode's toggle, service stop/restart and badge persistence were verified live in Task 15 (2026-09-27, dev machine); still owed: a real anti-cheat-protected game with the mode on and off (FACEIT or Vanguard with PawnIO 2.2.0 loaded, spec §13 point 1, Task 15 Phase H, optional).
- D6 (disk standby detection) checks on real hardware: hot-plug of a disk while storage is on; empty card readers' error codes; the R17 bus-class exclusions (virtual disks, Storage Spaces). First open, the sampling cadence, NVMe and a USB stick at service start were exercised in the M6b live checks (2026-10-02/04, SATA HDD, SATA SSD, two NVMe, USB stick).
- The service's memory footprint against the 80 MB budget on the rest of the hardware matrix (the dev machine is done, see "Closed in M4").
- Task 15's VM fault-injection scenarios for the installer and service, still owed as of 2026-09-27 (not attempted on this PC): `/S /NOSENSORS`, the Components page in EN and IT, deselecting the Advanced sensors component, an upgrade from the interface, an uninstall that leaves PawnIO, the reboot PawnIO requests (exit code 3010), STOP stuck, the uninstall helper exiting 1, PawnIO setup exiting neither 0 nor 3010, a refused custom install directory outside `Program Files`, an upgrade with a leftover `service\logs` holding a junction (recursive `icacls /reset /T` must not follow it), and a third-party writable folder inside `Program Files`.

## Manual checks after M7b

The live checks of the M7b plan (`docs/superpowers/plans/2026-10-04-m7b-motore-frame.md`, Task B12 step 8, V1-V8) are done (2026-10-05, local 0.4.1 setup with PresentMon, SHA-256 `FC650CEF...C5F5`, all passed; games: Control Resonant DX12, God of War 2018 DX11, RTX 4080). The agent read the `frames:` lines in `%LOCALAPPDATA%\OpenMonitorAdvanced\logs`.

- V1: Control without FG: 68.6-68.8 displayed FPS against 67 on the Steam overlay (about 2.5 %); Reflex rendered 68.7, `mult=1.00`. `bottleneck=-` because `pcl` does not track the GPU (only `all` does).
- V2: DLSS FG with PCL: 116-117 displayed (Steam 118), 58 rendered (Steam 59), `source=Reflex`, `mult=2.00`; without PCL (`1`): 117 displayed, `source=FG?`.
- V3: FSR FG without PCL: 120 displayed (Steam 120), `source=-`; with PCL: 120 / 60, `source=Reflex`, `mult=2.00`.
- V4: Smooth Motion in God of War with PCL: 157 displayed (NVIDIA overlay 157), 78.5 rendered, `mult=2.00`; without PCL `source=FG?`. `pc_lat_ms=-` in this DX11 game: PresentMon gives frame ids but no PC latency.
- V5: alt-tab: the target dropped 3 s after leaving the game and came back on return; a short exit (about 2 s) kept it. Noted: Windows Terminal, while typing, presents 15-22 FPS and became the target (rule "foreground with at least 10 FPS", as Afterburner); per-profile exclusions are an M7c/M7d topic.
- V6: tray with `OMA_FRAMES_DEBUG=1`, no game: app 0.05 % CPU and 17.9 MB; service 0.03 % CPU and 63.3 MB private; PresentMon 0.003 % CPU and 6.1 MB. Service plus PresentMon about 0.03 %, under the 0.5 % of spec M7 §11.
- V7: `Restart-Service oma-service` with the app open: link back in 7 s, capture `starting` then `running`, target followed again after about 10 s in all.
- V8: with OMA running, «Annulla» on the "app is running" prompt closes the uninstaller at once; OMA and the service keep running.

Also owed with that setup: the installer's PresentMon paths in a VM or Windows Sandbox (`service\presentmon\PresentMon-2.6.0-x64.exe` installed and protected, removed on deselection and uninstall, `logman stop OpenMonitorAdvanced-Frames -ets` run by the uninstaller), and `scripts/verify-signatures.ps1 -Policy none` on the new setup (exactly one PresentMon, Intel signature).

Open: after the setup rebuild, extend the real-lister test in `scripts/tests/VerifySignatures.Tests.ps1` ("the default lister reads a real setup") to `service\presentmon\PresentMon-2.6.0-x64.exe`; the setup on disk at the end of B12 predates PresentMon.

Known, by design: a service that crashes (or is killed) leaves the `OpenMonitorAdvanced-Frames` ETW session running, with PresentMon's buffers (up to 1024 × 64 KB), until the next service start (which stops it by name) or the uninstall (`logman stop OpenMonitorAdvanced-Frames -ets`).

The five "Before M7c" items from the review of the M7b branch are closed: see "Closed in M7c".

## Open: overlay (M7c)

Items left open by the M7c reviews (plan `docs/superpowers/plans/` M7c, tasks C1-C19).

- The overlay client could also check the pipe server's PID against its parent process (`GetNamedPipeServerProcessId`), mirroring the app's check on the child. `crates/oma-overlay/src/link.rs`; when touched.
- Pipe handoff when a process of the same user reuses the child's PID (C8 security review, M1): the user is outside the threat model and the data are only sensors and profiles; harden it with a nonce passed on the child's stdin. `app/src-tauri/src/overlay/host.rs`; before 1.0.
- Cost of the `EVENT_OBJECT_LOCATIONCHANGE` events followed for the game window: not measured in C20 (W8 ran with a still window; W6 moved it without a measure). `crates/oma-win` (foreground); measured in M7d (X8): 0.26 % CPU for `oma-app` while dragging a windowed game for 20 s, against 0.07 % idle. Closed.
- Frame state `starting` on an idle desktop: not checked explicitly in C20. Checked in M7d (X8): Settings › Overlay shows running and never sticks on starting; the engine stays on while the overlay is enabled, by design. Closed.
- Overlay memory: shown in game it uses 56-65 MB private, almost all the discrete GPU driver (D3D11 device about 51 MB); the in-game limit was raised from 40 to 70 MB by the user "this once" (W8). WARP would stay at about 3 MB with nearly the same CPU on a small window; the integrated GPU does not help when the screen is on the discrete one (both drivers load, 78-87 MB). Measure WARP at high scale before switching. `crates/oma-overlay/src/compose.rs`; when memory matters again.
- A foreground window already gone when the runner reads its monitor (the alt-tab switcher) counts as on the game's monitor: the overlay can hide for one step and show again. The desktop window spans all monitors, so a click on the wallpaper of the other monitor may hide the overlay. `app/src-tauri/src/overlay/runner.rs` and `controller.rs`; when touched.
- The app log names the foreground executable at each focus change while the overlay is on (diagnostics added in C20): consider debug level before 1.0, since users attach logs to reports. `app/src-tauri/src/overlay/runner.rs`.
- Any change to `gameProfiles` (even for another game) ends the *Next profile* choice. `app/src-tauri/src/overlay/controller.rs`; if users notice.
- Exclusive-fullscreen notice (DP16) not verified live: Control in DX12 never reports a `Legacy` present mode. Needs a DX9/DX11 game in true exclusive fullscreen.
- Not tried live: God of War 2018 (DX11), a Vulkan and an OpenGL game with the overlay (W1).

## Open: editor and benchmark (M7d)

Items left open by the M7d reviews (plan `docs/superpowers/plans/2026-10-05-m7d-editor-benchmark.md`, tasks D1-D16).

- `FrameReadout::read()` on a full 300 s window at 240 FPS (72 000 frames) with a 300 s low window costs 3.2 ms per call in release (40 ms in debug; test `readout_cost_on_a_300_s_window`, 2026-10-05), above the 2 ms mark. The controller calls it at `textHz`. Most of the cost is the copy of the window and the sort of the lows; only profiles with long windows pay it. `crates/oma-core/src/frames/readout.rs`; if the editor makes long windows common.
- The window shared by the preview and the edited profile's lows can stretch to 300 s; `readout()` shares one swapchain between cursors. `app/src-tauri/src/overlay/controller.rs`; when touched.

Core and benchmark:

- No test for string escaping in a profile (quotes, backslash, non-ASCII in name or label) nor for non-default outline, shadow and gauge in the writer. `crates/oma-core/src/overlay/`.
- Single-frame lows fall back to the displayed FPS and PCL ids with `id1 <= id0` give nil: untested. `crates/oma-core/src/frames/session.rs`.
- The benchmark file list has no last name tiebreak for equal stamp and suffix; the `formula_guard` test mostly retests `escape_field`. `app/src-tauri/src/overlay/benchmark.rs`.
- `overlay-status` is emitted once a second during a capture (the elapsed time); summary JSON `reason` and status `error` can differ when the rows fail in the same step as the limit; `log.folder` accepts UNC paths (older). `app/src-tauri/src/overlay/`.
- `benchmark.end.<reason>` shows the raw key for an unknown reason, and a stale `failure` text outlives a change of state. `app/src/components/settings/`.

Editor commands and files:

- A failed `read_names` of the font list is skipped silently and the font warning repeats while DirectWrite fails. `crates/oma-win`, `app/src-tauri/src/overlay/`.
- `export_file_name` edge cases (trailing dots, reserved names such as `CON`, empty), importing a folder says "not found", crash-left `.tmp` files of a process id are not cleaned, and saving with an unknown uuid creates a file (unbounded files from a compromised WebView). `app/src-tauri/src/overlay/store.rs`; before 1.0.
- A tray *Quit* with a dirty editor never exits if the editor script hangs. `app/src-tauri/src/`; when touched.
- `preview_failure` is not cleared when the editor opens again. `app/src-tauri/src/overlay/`.
- Closing the preview logs a WARN "stopped unexpectedly code=5" before the INFO "the preview was closed": log noise. `app/src-tauri/src/overlay/host.rs`; when touched.
- The overlay hides about 3 s into a drag of the game window: the game stops presenting in Windows' modal move loop and the 3 s tolerance runs out. The user finds it acceptable (X8). Possible fix: ignore the timeout between `EVENT_SYSTEM_MOVESIZESTART` and `EVENT_SYSTEM_MOVESIZEEND`. `crates/oma-win` (foreground), `app/src-tauri/src/overlay/target.rs`.

Editor interface:

- *Export* with unsaved changes and the answer "Discard" exports the saved file, not the edited one; the TypeScript `uniqueName` does not truncate to the name limit (the Rust one does). `app/src/lib/editor/`.
- Any key, even a lone Ctrl or Shift, ends a drag in progress (`Canvas.svelte`); the canvas has a single bottom-right resize handle; the resolution presets assume 100% scaling; a non-current stat copies the full series once per source per repaint.
- Multi-selection shows thresholds and *visible if* read-only as "-"; the alpha of the colour pickers is not editable; the editor-only limits are z +-10000, offset +-400 and 1024 characters of text; a failed save drops the requests queued behind it; `commonValue` compares through `JSON.stringify` (key order); a load error stays visible next to the new-profile fallback. `app/src/components/overlay-editor/`.
- Preview window: no redraw during the modal drag of the border; an unplaceable profile is drawn at (0,0); `set_preview_area` is untested; a stale doc comment on `destroy()`. `crates/oma-overlay/src/window.rs`.
- Drawing the benchmark box builds its row strings at every frame, and the summary shows the stutter count without the percentage. `crates/oma-overlay/src/render/`.

## Manual checks owed after M7d

Task D18 done on 2026-10-05/06 with the user (setup 0.5.0, SHA-256 `14d15424...f2f8`, God of War and Windows Terminal): X1-X8 all passed, two after fixes (`2101dbf` dismissible error banner, `4b9bea6` spacing in the benchmark sessions); results in the plan's «Esito dell'esecuzione», numbers in `docs/perf-budget.md`. Covered: editor bounds while maximized, edit and close with the preview open, the usual editor run, a benchmark with the shortcut, the footprint of editor and preview, and the two M7c items (window-drag cost, `starting` on an idle desktop). Still owed:

- Preview at 1366x768 and 150% scaling: the window fits the work area and the profile is drawn whole.
- Kill the preview process (`oma-overlay.exe --preview`) 3-5 times from Task Manager: the editor button returns to *Preview*, the in-game overlay is not touched, no restart loop.
- The installer paths of 0.5.0 (as for M7c, in a VM or Windows Sandbox).

## Manual checks after M7c

The live checks of the M7c plan (Task C20, W1-W9) are done (2026-10-05, local 0.4.1 setups rebuilt after each fix, last from `8c6fb8d`, SHA-256 `7564952d...6ad4a`; Control Resonant DX12, RTX 4080, two monitors). All passed, some after fixes; results in the plan's «Esito dell'esecuzione» and the budget numbers in `docs/perf-budget.md`.

- W1: FPS match the Steam overlay; GPU % and VRAM are lower than Steam's by method (busiest engine as Task Manager; NVML v2 without the driver reserve). Fixed: a terminal on the second monitor took the overlay away from the game.
- W3: fixed the «Minimal FPS» label and the *Next profile* choice outliving a settings change.
- W7: fixed the overlay staying hidden after alt-tab (late foreground event of the switcher).
- W8: overlay 0.39 % CPU and 65.1 MB in game, CPU total 0.65 % < 1 %; memory limit raised to 70 MB.

## Manual checks owed after M7c

The overlay's installer paths, on a setup built with C19 (in a VM or Windows Sandbox, or by the user; never by an agent on this PC):

- Fresh install: `C:\Program Files\OpenMonitor Advanced\oma-overlay.exe` present, with the product name and version in its properties.
- Upgrade over a running app with the overlay visible over a game: the setup stops the overlay (`OMA_STOP_OVERLAY` in `OmaCloseApp`), copies the new one without a Retry/Ignore prompt, and the reopened app starts it again.
- Uninstall: `oma-overlay.exe` is removed (or scheduled for removal at the next restart if it was locked); with the service stop failing, no file is deleted, the overlay included.
- `scripts/verify-signatures.ps1 -Policy none` on the new setup: exactly one `oma-overlay.exe`, with the collect-pass hash and the product metadata.

## Manual checks after M7a

The live checks of the M7a plan (`docs/superpowers/plans/2026-10-04-m7a-manutenzione.md`, U1-U7) are done (2026-10-04, published 0.4.1 setup, SHA-256 `d45c2eda...465c`, all passed):

- U1: 0.4.0 → 0.4.1 in graphical mode with «Uninstall first», *Start with Windows* on and the app in the tray: no «running» message, the app reopened, the service running, the Run value still there and `autostart: true`.
- U2: `setup.exe /S` over 0.4.1 with the app in the tray: no window, the app came back with `--minimized` (new PID). The app does not log its shutdown, so the orderly `--quit` path is inferred (no window appeared), not proven from a log.
- U3: as U2 with the app closed: it stayed closed.
- U4: uninstall from Settings › Apps with the app open: the «running» question is still there.
- U5: CSV log folder renamed away, *Open folder*: «the file or folder does not exist»; the *About* and *General* buttons work.
- U6: standby HDD, 1 min window: y-axis visible, the suspended notice shown.
- U7: `--quit` with the app open (exits) and closed (no window); then `target\release\oma-app.exe` started and quit: the Run value still points at `Program Files` and no crash marker.

Open: log one INFO line at app shutdown (reason: tray, `--quit`, session end) so a future U2 can prove the orderly path from the log.

## Manual checks owed after M6c

The live checks of Task 13 (spec M6c §8.2, U1-U8, 2026-10-04) are done: the update toast click opening *Settings › About*, *Export sensor report* with a save, a cancelled save dialog (nothing shown) and *Open folder*, and the licence buttons (closed by the user on 2026-10-04: both files open). The first CI run of the "Check third-party licences" step passed on the merge push (run 37209604023, cargo-about installed and cached). What they leave open stays here.

- The WinHTTP TLS 1.2 fallback on a Windows 10 VM (where WinHTTP has no TLS 1.3): *Check now* must still succeed. Not attempted on this PC (Windows 11).

## Manual checks owed after M6b

- A hard disk behind a USB bridge (no such hardware): standby with the SMART switch off (default) and on; the bridge's answer to SAT `CHECK POWER MODE`.
- With the service connected, from the final review: an SSD or NVMe without a local temperature sensor keeps its main temperature (I1); an HDD going active and idle while a CPU chart is on screen shows no dropout of the service sensors (I2); the first service measure of an HDD (it costs a full storage rediscovery in the core).
- PC suspend and resume with a quiet HDD and the service connected: SMART values come back after the resume without waiting for the disk to work (I3, covered by tests only).
- A USB disk plugged in after the service has switched storage on (DiskInfoToolkit hot-plug path, see the limits above).

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
- A USB volume removed while its `volume-used` rule is active: the coverage and the retained alert.
- Toast click while the window is loading and from the notification centre, and the fallback for a removed device (the window-open and window-closed cases passed on 2026-09-30, with the right AUMID).
- TjMax on an Intel CPU and on AMD models other than this 7800X3D; the NVMe critical warning with a real warning bit set.

## Manual checks owed after M5a

- PawnIO scenarios in a VM: driver stopped, uninstalled, and the 3010 reboot state (`rebootPending`, then `ok` after the reboot); the Win32 codes 2/3/5 of the probe live. `ok` was checked on the dev machine.
- Autostart across a real logout/login (the Run value and the Task Manager enable/disable states were verified; the login itself was deferred).
- Per-module switches with two clients (two user sessions): the module stays on while one of them wants it.

## Closed in M7d

- Automatic full scale of `meter` and `gauge` blocks: the maximum of the last 60 s instead of the peak of all time, in the overlay and on the canvas (DD16): task D7.
- Errors from *Try again* and *Reload profiles* in the Overlay settings page are shown: task D16.
- Final review wave: late preview and canvas profile after the editor closes (guard on the editor being open), *Duplicate*, *Import* and *Export* with unsaved changes ask first, `editor.json` limited to the commands of the editor window, a failed preview is reported, editor bounds not saved while maximized, preview window ends on `WM_DESTROY`, checks the DPI rectangle and fits the work area, unique names fit the name limit, the session lows use one sorted copy, deleting the capture in progress is refused and the benchmark errors have i18n keys; *Save as* on a built-in profile gets a unique name (D15).
- Cost of `FrameReadout::read()` on long low windows: measured (3.2 ms in release on 300 s at 240 FPS), see the open item above.

## Closed in M7c

- Live checks C20: target kept across monitors (`7b908b6`, `829dab4`), «Minimal FPS» label, settings ending the profile choice and the hidden-overlay row (`c0c90dd`), foreground taken from Windows and checked every second (`d3edbef`), diagnostic log lines (`83a006d`, `504dc1a`).

- Frame hub idle when no session is enabled (no timers and no empty summaries with `Enabled=false`), and a bounded backlog for the rows beyond 512 per tick instead of counting them in `Dropped`: task C1 (`e6c2859`).
- Stutter median computed incrementally instead of from scratch for each window, and input guards in `oma_core::frames::synthetic()` (FPS, factors and seeds out of range): task C2 (`4572f04`).
- *Try again* after `failed` (`crashing`): the overlay controller sends `enabled: false` and then `enabled: true` (test `retry_after_failed_sends_disabled_then_enabled`): task C15 (`200f5e9`).

## Closed in M7a

- Split of the large files with no change of behaviour: `storage.rs` into a module directory (`4b88fca`), the service link into machine, transport and driver (`fbee564`), the display-key formatter out of `health.rs` (`7d5a650`, tests `1653e60`), `SensorHub` into partial files with `GateEpisode` in its own file (`0dfdb90`).
- LibreHardwareMonitor names with trailing spaces and NUL bytes: the service cleans display names before logging and publishing them (`d1508af`).
- An unknown module name or message `type` echoed unbounded in `bad_request` and in the log: client text is now clipped (`d1508af`).
- Upgrade over an installed version: the user's decision of 2026-10-04 replaces the ruling of 2026-10-02. The installer no longer asks: it closes the running app by itself (`--quit`, with a forced kill after 10 s, or at once if `DisplayVersion` is unreadable) and reopens it afterwards; from 0.4.0 the app is closed by force, from 0.4.1 in an orderly way (`08daff0`, `12d6445`). A graphical upgrade with «Uninstall first» keeps the user's *Start with Windows* value: the setup reads it before the old uninstaller deletes it and writes it back afterwards (`78f9e44`). Only the upstream report of the malformed placeholders stays open.
- `used_pct` duplicated in the memory and storage providers: one shared function (`0df0c00`).
- PDH item count unchecked before `from_raw_parts` and a null `szName` unguarded: the arrays are bounded and `item_name` is an `unsafe fn` with a documented contract (`0df0c00`, `ad147f6`).
- `shell_open` without a timeout, without `SEE_MASK_NOASYNC`/`FLAG_NO_UI` and without an existence check: it now uses `ShellExecuteExW` on a bounded thread, reports a missing path, and the log folder button shows a translated message (`e42b819`, `8cb9f75`).
- Stale autostart path: at startup a release build rewrites a Run value whose executable no longer exists; another existing copy (`target\release`, a portable copy) and a debug build never do (`a977a65`, `64c422d`). The standard-user Run value left by an uninstall stays open as a documented limit.
- Service link: the command queue is bounded (a full queue returns an error at once) and the held state follows the service: `NotInstalled` after an uninstall, unreachable and disconnected once it stops (`f7ea18e`).
- Flaky timing assert of `reads_disk_temperatures_on_this_machine` under parallel load: it is still a timing test, now judged on the fastest of three reads per disk, so a load spike no longer fails it while a disk that is always slow still does (`0df0c00`).
- `THIRD_PARTY_LICENSES.txt` now includes the third-party notices of the Microsoft NuGet packages (`6181767`).
- M5c `log::session` timing tests flaky under CPU load: they wait on conditions instead of sleeping (`98bd9f7`, with two more found under load).
- Chart window with the whole series suspended: the y-axis stays and the text "No readings while the device is idle or in standby" explains the blank plot (`1e957a5`).

## Closed in M6c

- Privacy statement for the update check: `CODE_SIGNING.md` (Privacy) now describes the only network request of the app (the update check to `api.github.com`: the manual *Check now*, or the daily automatic check, off by default), what it sends (the IP address, implicitly, and a User-Agent with the app version), that it never downloads or installs anything, and that the service never uses the network; the README sections match it. This fits the SignPath Foundation terms (spec M6a §7): no network transmission unless the user asks for it.
- `Mono.Posix.NETStandard` 1.0.0: its `.nuspec` has `licenseUrl` `https://go.microsoft.com/fwlink/?linkid=869050` (copyright "© Microsoft Corporation. All rights reserved."), which redirects (302, checked 2026-10-04) to `https://github.com/mono/mono/blob/master/LICENSE`: "the runtime and its class libraries are licensed under the terms of the MIT license". Redistribution is permitted; recorded as MIT in `THIRD_PARTY_NOTICES.md` and in `THIRD_PARTY_LICENSES.txt` (override in `scripts/generate-licenses.ps1`).
- Third-party licence texts: `THIRD_PARTY_LICENSES.txt` is generated by `scripts/generate-licenses.ps1` (Rust crates with cargo-about 0.9.2, bundled JS packages from the Vite manifest, the service's NuGet runtime packages, the .NET runtime and its third-party notices), checked in CI with `-Check`, installed with the app and opened from *Settings › About*.

## Closed in M6b

- HDD standby (M5a live check, inconclusive): cause found in the M6b spike. The core's disk temperature query (`StorageDeviceTemperatureProperty`, every 30 s) reaches a SATA HDD, wakes it from standby and resets Windows' idle timer; the service's `CHECK POWER MODE`, native or SAT, does the same even with SMART off, and powers up a disk Windows turned off. The earlier "never spun down, even with everything closed" matched TR-VISION HOME being open (see the limits). Fixed: the core reads a rotational disk only after recent PDH activity (spec M6b §5.2); the service sends nothing to a disk Windows turned off or that showed no read/write activity (`IOCTL_DISK_PERFORMANCE` on an access-0 handle) since the end of the previous round (§4.4, window widened in Task 16). Live (2026-10-02/04): Windows turned the HDD off with the service alone, with app and service, and with the app alone in anti-cheat mode (V3 in both modes, V8); a forced standby held with the service connected (V1) and in anti-cheat mode (V2); the counters see I/O from other handles (prova 2).
- USB disks and the D6 gate: SAT `ATA PASS-THROUGH(16)` fallback for `CHECK POWER MODE`, and USB disks off by default (spec M6b §4.1, §4.2). Live (V4): with the stick plugged in at service start the other disks' SMART is on and the stick is `smartOff` without blocking.
- `smartGateClosed`: replaced by the per-disk `drives` table of protocol v3 (`physicalDrive`, `key`, `model`, `state`, `blocksSmart`); the Sources view names the blocking disks from it, also without a model ("Disk N").
- Rules with an HDD in standby: a confirmed standby suspends only the disk's temperature and SMART rules, «Idle» only its temperature rule (D7). Live: V5 (banner «All clear» with the HDD in standby) and V9 (idle, service disconnected and reconnected, no new alarm).
- Per-sensor quality (`Fresh`, `Held`, `Suspended`) from the providers to the UI; a suspended value is muted with «Last reading», stays out of history and statistics, and is written as `suspended` in the CSV log (Task 17, user decision of 2026-10-02).
- Final review: the lost main temperature of a bound SSD/NVMe (I1), the one-tick dropout of the service sensors on a drive-table change (I2) and the blank SMART of a quiet HDD after a resume or a late round (I3) are fixed. The «—» seen once in min/max/avg right after a reconnect (V9) was the transient of the reconnect's schema revisions: drive power states are not part of the core's inventory, so a state change does not bump the app's schema revision (checked 2026-10-04).
- Budget (V7): within every limit, `docs/perf-budget.md` (M6b).

## Closed in M6a

- Raw `{product_name}` in the Italian "app is running" messages of the installer (seen by the user on 2026-10-02): Tauri's own `Italian.nsh` (tauri-cli 2.11.5) writes the placeholder as `{product_name}}` or `{product_name}` in `appRunning`, `appRunningOkKill` and `failedToKillApp`, and `CheckIfAppIsRunning` only replaces `{{product_name}}`. We ship a corrected copy, `app/src-tauri/nsis/Italian.nsh`, through `bundle.windows.nsis.customLanguageFiles`; compare it with upstream when tauri-cli is upgraded. Checked on a local build (generated strings), not yet seen in a running installer.
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
