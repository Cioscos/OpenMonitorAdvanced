# Follow-ups

Items consciously left open, with where they live and when they are expected to be picked up.
Updated at the end of every milestone (last update: M3).

## Open: code

| Item | Where | Pick up |
|---|---|---|
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

## Open: deferred features (spec §5.2 point 5, §7.3; M3 decision D10)

- NVML `TotalEnergyConsumption` (p95 about 9 ms) and `PcieThroughput` (blocks 31 ms): only with sampling outside the tick, when the CSV log or the rules need them (M5).
- Battery page: appears when a battery provider exists.
- "N more sensors available with the service": M4.
- Advanced view state (section, chart window, series) lives in the WebView `localStorage` until `settings.json` (M5).

## Manual checks owed by a human

- Tray left click and the "Open" menu item re-create the window after it was closed (M1).
- USB disk hot-plug keeps or changes disk ids correctly (M1). Since M3 also: a disk without a serial number (for example a VHDX mounted by an administrator) appears with a `storage/gpt-…` id and keeps it across a restart.
- IGCL telemetry and PCIe link on Intel hardware; ADL on a dedicated Radeon (hardware matrix, spec §12).

## Closed in M3

- Disks without a unique readable serial are no longer dropped (identity fallback chain, Task 4).
- The "monitoring for N min" banner counts from the core's first tick, also after reopening from the tray (`startedAtMs`, Tasks 1 and 10).
- A panic in `Engine::tick` no longer stops sampling; `History::push` no longer panics on a length mismatch; the UI shows *Data not updating* (Tasks 2 and 10).
- Implausible-value debug logs are rate-limited to one line per sensor per minute (Task 2).
- The GPU PCI address is kept per LUID across enumerations (Task 6).
- A second launch while the window is closed re-creates the window: exercised by the full-history measurement (Task 14, Step 7).
