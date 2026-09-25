# OpenMonitor Advanced

Open-source hardware monitor for Windows with a modern UI: a Simple view that tells you at a glance
whether your PC is fine, and an Advanced view with every sensor.

**Status:** milestone 3 (Advanced view) — CPU, RAM, disks, network and GPUs (NVIDIA, AMD, Intel) without admin rights, with a page per component in the Advanced view.
Design: `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`.

## Build

Prerequisites: Windows 10/11, Rust stable ≥ 1.85 (MSVC), Node 22, pnpm 10, WebView2 runtime (preinstalled on Windows 11).

    cd app
    pnpm install
    pnpm tauri dev        # run the Tauri shell; the UI comes from the Vite dev server (pnpm dev)
    pnpm dev              # UI only, in the browser, with a mock backend, with hot reload

Tests: `cargo test --workspace` (after `pnpm build` in `app/`) and `pnpm test` in `app/`.
Before closing a milestone, also run the hardware tests on real Windows hardware:
`cargo test -p oma-win -- --include-ignored`.

## Advanced view

A sidebar lists every component: the CPU, each GPU (the integrated one too), RAM, each disk and each
network adapter. The view reopens on the last page you visited. Each page shows:

- four key figures for the component;
- a history chart for the last 1 minute, 5 minutes, 30 minutes or 1 hour, with up to 8 series and
  2 units at once; the 30-minute and 1-hour windows draw a min/max envelope, so peaks stay visible;
- a table of every sensor, grouped by category, with current, minimum, maximum and average values.
  The monitor keeps these statistics from the moment the app starts, also while it sits in the tray;
  the reset button clears them for the sensors of that page. Hover a sensor to see where its value
  comes from; *experimental* marks readings from undocumented calls;
- the device's static details, such as PCIe link, power limits and temperature thresholds;
- for GPUs, the processes using the GPU, with their load and dedicated/shared memory.

Clicking a tile in the Simple view opens the matching page. If no data arrives for a few seconds, the
top bar shows *Data not updating*.

**Disks.** Drive temperatures come from the drive itself where it reports them (most NVMe drives,
some SATA drives), refreshed every 30 seconds; a spun-down disk is not woken up to read it. Disks
without a readable serial number (virtual machines, some RAID or USB enclosures) are still shown:
they are identified by their GPT disk GUID, their MBR signature or, as a last resort, the port they
are connected to.

## GPU support

No admin rights are needed for GPU data. Every GPU gets per-engine load and dedicated/shared memory
(Windows performance counters), plus core temperature, clocks and power as a percentage of the limit
where the driver reports them (D3DKMT). On top of that, the app uses the libraries installed with the
graphics driver:

- **NVIDIA:** NVML (temperature, clocks, board power and limit, fan, VRAM, throttle reasons) and NVAPI
  (hotspot and memory junction temperatures, core voltage; marked *experimental* because these calls
  are undocumented);
- **AMD:** ADL (`atiadlxx.dll`): temperatures, clocks, power, fan and voltage where the GPU exposes them;
- **Intel Arc / Xe:** IGCL (`ControlLib.dll`), implemented but not yet verified on Intel hardware.

Vendor libraries are loaded from `System32` only and never redistributed; see `THIRD_PARTY_NOTICES.md`.
The Simple view shows one tile per discrete GPU (the integrated GPU only when there is no discrete one).

**Safe mode.** A crash inside a vendor library cannot be caught. Start with `oma-app.exe --safe` to skip
the vendor libraries and keep only the Windows data. After a native crash the app writes
`%LOCALAPPDATA%\OpenMonitorAdvanced\crash.txt` and starts the next time in safe mode by itself.
In both cases a notice under the top bar offers **Re-enable**, which loads the libraries without a restart.

**Known limits.** If the NVIDIA driver is updated or unloaded while the app runs, NVML is not initialised
again: its fields fall back to the Windows data (D3DKMT) until the app is restarted.

## Performance budget

The monitor must not distort what it measures. Budgets and the latest measurements are in
`docs/perf-budget.md`; run `scripts/measure-footprint.ps1` on a release build to reproduce them.

## License

GPL-3.0-or-later. See `LICENSE`. Third-party notices: `THIRD_PARTY_NOTICES.md`.
