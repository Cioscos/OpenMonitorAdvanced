# OpenMonitor Advanced

Open-source hardware monitor for Windows with a modern UI: a Simple view that tells you at a glance
whether your PC is fine, and an Advanced view with every sensor.

**Status:** milestone 4 (service) — CPU, RAM, disks, network and GPUs (NVIDIA, AMD, Intel) without admin rights, with a page per component in the Advanced view, plus an optional Windows service for extra sensors (temperatures, voltages, fans, SPD) via LibreHardwareMonitor and PawnIO.
Design: `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`.

## Build

Prerequisites: Windows 10/11, Rust stable ≥ 1.85 (MSVC), Node 22, pnpm 10, WebView2 runtime (preinstalled on Windows 11).

    cd app
    pnpm install
    pnpm tauri dev        # run the Tauri shell; the UI comes from the Vite dev server (pnpm dev)
    pnpm dev              # UI only, in the browser, with a mock backend, with hot reload

Tests: `cargo test --workspace` (after `pnpm build` in `app/`), `pnpm test` in `app/` and
`dotnet test service/OpenMonitorAdvanced.slnx` for the service.
Before closing a milestone, also run the hardware tests on real Windows hardware:
`cargo test -p oma-win -- --include-ignored`.

### Building the installer

The NSIS installer (Tauri bundler) packages the app and, in its "Sensori avanzati"
component (on by default; `/NOSENSORS` at silent-install time to exclude it), the
service and the official PawnIO 2.2.0 setup:

    pwsh scripts/build-installer-payload.ps1   # publishes oma-service and stages PawnIO_setup.exe
    cd app
    pnpm tauri build --bundles nsis

The payload script downloads (or reuses a cached, SHA-256-verified copy of)
`PawnIO_setup.exe` — never committed to the repository — and publishes
`oma-service` self-contained, trimmed, single-file, win-x64. The installer
requires the install folder to be under `Program Files` for the Advanced
sensors component; it stops the service before copying files and installs
PawnIO silently when it is missing or older than 2.2.0 (exit code 3010 means
a reboot is needed).

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
some SATA drives), refreshed every 30 seconds; a disk that Windows reports in standby is not
queried. Whether the periodic query keeps an idle HDD from spinning down is not yet verified on
hardware. Disks
without a readable serial number (virtual machines, some RAID or USB enclosures) are still shown:
they are identified by their GPT disk GUID, their MBR signature or, as a last resort, the port they
are connected to.

## Sensor service

An optional Windows service, `oma-service`, adds sensors that need
administrator rights to read directly: CPU/motherboard temperatures and
voltages, RAM SPD, fan/RGB controllers and disk SMART/NVMe health, via
[LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)
and the [PawnIO](https://pawnio.eu/) driver. It is installed by the "Sensori
avanzati" component of the NSIS installer (on by default; see "Building the
installer" below) and runs as `LocalSystem`, because PawnIO needs that.

- **Base mode vs. service connected.** The app works without the service:
  everything under "Advanced view" above (CPU, RAM, disks, network, GPUs)
  keeps working from the admin-free providers. With the service connected,
  the CPU, RAM and disk pages also show the extra sensors, and a
  Motherboard page appears. Without it, those pages show one generic notice
  ("Temperatures, voltages and other sensors available with the service")
  instead of listing the missing sensors one by one. A badge in the top bar
  explains why the app is in base mode (service not installed, stopped,
  starting, unreachable, anti-cheat mode, or an incompatible protocol
  version) and offers the matching action.
- **Manual start.** The service does not start with Windows
  (`SERVICE_DEMAND_START`): the app starts it, over a named pipe
  (`OpenMonitorAdvanced.Sensors.v1`, protocol version 1), when it is
  installed, not in anti-cheat mode, and the first service-status check at
  launch comes back `Stopped`. It opens LibreHardwareMonitor (and PawnIO)
  on the first subscription and keeps it open — closing and reopening it
  costs about 4.5 s with disks attached — until it stops itself after 2
  minutes with no client.
- **Anti-cheat compatible mode.** From the tray menu, this stops the
  service and keeps the app from restarting it until the mode is turned
  off again (from the tray or the top-bar badge); the preference persists
  across restarts in `%LOCALAPPDATA%\OpenMonitorAdvanced\` (moving to
  `settings.json` in M5). Turning the mode on requests the stop and waits
  for the SCM to confirm it (30 s timeout). **What this does and does not
  stop:** it closes the service process and every handle it held on
  PawnIO, which is what a heuristic anti-cheat can notice. It does **not**
  unload the PawnIO driver itself: PawnIO is a Plug-and-Play device
  (`ROOT\PAWNIO\0000`) that Windows loads at boot regardless of who uses
  it, and does not unload when the last handle closes; removing it needs
  administrator rights and would affect other programs that share it (for
  example FanControl), so the app never does that. With PawnIO 2.2.0
  (Microsoft-signed), FACEIT accepts the driver being loaded; the
  compatibility issue with earlier PawnIO builds was about their signing
  certificate, not the driver's presence. No block by Vanguard, EAC or
  BattlEye is known as of this writing. This has not been verified against
  a real anti-cheat-protected game.
- **Known limits.** On a machine with more than one interactive user, any
  of them can stop the service for the others (accepted limit — a shared
  machine loses the extra sensors for everyone until someone restarts it).
  A disk that Windows reports as in standby is not queried for SMART, so
  it reports no health sensors while asleep. The named-pipe protocol is
  versioned (`protocol_version` in the `Hello` message); an app and
  service built from different, incompatible milestones will refuse to
  talk to each other and the badge asks to update the service.

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
