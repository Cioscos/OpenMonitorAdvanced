# OpenMonitor Advanced

**English** · [Italiano](README.it.md)

[![CI](https://github.com/Cioscos/OpenMonitorAdvanced/actions/workflows/ci.yml/badge.svg)](https://github.com/Cioscos/OpenMonitorAdvanced/actions/workflows/ci.yml)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)
![Platform: Windows 10/11](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D4.svg)

A free, open-source hardware monitor for Windows 10 and 11 with a modern interface.

- The **Simple view** tells you at a glance whether your PC is doing fine.
- The **Advanced view** shows every sensor, with history charts and statistics.

CPU, RAM, disks, network and GPUs (NVIDIA, AMD, Intel) are read **without administrator rights**.
An optional Windows service adds the sensors that need them: temperatures, voltages, fans and
SMART data.

> **Status:** early development (version 0.1.0). Expect rough edges and breaking changes between
> versions.

## Features

- **Two views.** The Simple view has one tile per component. Click a tile to open that
  component's page in the Advanced view.
- **One page per component** in the Advanced view: the CPU, each GPU (integrated ones too),
  RAM, each disk and each network adapter. Each page shows:
  - four key figures;
  - a history chart for the last 1, 5, 30 or 60 minutes, with up to 8 series and 2 units. The
    30- and 60-minute windows draw a min/max band, so short peaks stay visible;
  - a table of every sensor with its current, minimum, maximum and average value. Statistics are
    kept from app start, also while the app sits in the tray. Hover a sensor to see where its
    value comes from;
  - static device details, such as the PCIe link, power limits and temperature thresholds;
  - for GPUs, the processes using the GPU, with their load and memory.
- **GPU support** for NVIDIA, AMD and Intel, through Windows and the libraries that come with the
  graphics driver.
- **Light on resources.** The monitor should not distort what it measures. Its budget is under 1% CPU at idle,
  under 30 MB in the tray, under 200 MB with the window open (WebView2 included).
- **English and Italian** interface and installer.

## Download and install

1. Download `OpenMonitor.Advanced_<version>_x64-setup.exe` from the
   [latest release](https://github.com/Cioscos/OpenMonitorAdvanced/releases/latest).
2. Run it. The installer is **not code-signed** yet, so Windows SmartScreen may warn you: choose
   *More info* → *Run anyway*.
3. Keep the **Advanced sensors** component selected if you want temperatures, voltages, fans and
   SMART data (see [Sensor service](#sensor-service)). Clear it for an installation that never
   touches drivers or services.

Requirements: Windows 10 or 11, 64-bit, with the Microsoft Edge WebView2 runtime (preinstalled on
Windows 11). Administrator rights are needed only during setup.

For unattended installs, `/S` runs the installer silently and `/NOSENSORS` leaves out the
Advanced sensors component.

## GPU support

GPU data never needs administrator rights. Every GPU gets per-engine load and dedicated/shared
memory from Windows performance counters, plus core temperature, clocks and power where the driver
reports them. On top of that the app uses the vendor libraries installed with the graphics driver:

| Vendor | Library | What it adds |
|---|---|---|
| NVIDIA | NVML | Temperature, clocks, board power and limit, fan, VRAM, throttle reasons |
| NVIDIA | NVAPI | Hotspot and memory junction temperatures, core voltage (*experimental*: undocumented calls) |
| AMD | ADL | Temperatures, clocks, power, fan and voltage, where the GPU exposes them |
| Intel Arc / Xe | IGCL | Implemented, not yet verified on Intel hardware |

Vendor libraries are loaded from `System32` only and are never redistributed.

**Safe mode.** A crash inside a vendor library cannot be caught. Start `oma-app.exe --safe` to
skip vendor libraries and use Windows data only. After a native crash the app starts in safe mode
by itself the next time. In both cases a notice offers **Re-enable** to load the libraries again
without restarting.

## Sensor service

The optional `oma-service` Windows service reads the sensors that need administrator rights: CPU
and motherboard temperatures and voltages, RAM SPD, fan and RGB controllers, and disk SMART/NVMe
health. It uses [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)
and the Microsoft-signed [PawnIO](https://pawnio.eu/) driver, and runs as `LocalSystem`.

- **The app works without it.** Without the service, CPU, RAM, disks, network and GPUs keep
  working. With it, the CPU, RAM and disk pages show the extra sensors and a Motherboard page
  appears. A badge in the top bar explains why the service is not connected and offers the right
  action.
- **It does not start with Windows.** The app starts the service when needed, and the service stops
  itself 2 minutes after the last client disconnects.
- **Anti-cheat compatible mode.** From the tray menu you can stop the service and keep the app from
  restarting it, for games with an anti-cheat. This closes the service and every handle it holds on
  PawnIO. It does not unload the PawnIO driver, which Windows loads at boot and other programs
  (such as FanControl) may share. FACEIT accepts PawnIO 2.2.0; no block by Vanguard, EAC or
  BattlEye is known. None of this has been tested against a real anti-cheat-protected game yet.
- **Known limits.** On a PC with several signed-in users, any of them can stop the service for
  everyone. A disk that Windows reports in standby is not queried, so it shows no health data while
  asleep.

## Build from source

### Prerequisites

- Windows 10/11 x64
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/) with the *Desktop
  development with C++* workload (MSVC and the Windows SDK)
- [Rust](https://rustup.rs/): `rustup` installs the pinned toolchain (1.90.0, from
  `rust-toolchain.toml`) on first build
- [Node.js](https://nodejs.org/) 22 and [pnpm](https://pnpm.io/) 10
- [.NET SDK](https://dotnet.microsoft.com/download) 10.0.303 or a later 10.0.3xx patch (pinned in
  `global.json`), for the sensor service
- PowerShell 7 (`pwsh`), for the build scripts

The Tauri bundler downloads NSIS by itself the first time you build the installer.

### Run in development

```bash
cd app
pnpm install
pnpm tauri dev    # the full app
pnpm dev          # the UI only, in a browser, with a mock backend and hot reload
```

### Tests and checks

```bash
cd app && pnpm install && pnpm build && cd ..   # the Rust build needs app/dist
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && pnpm test && pnpm check && cd ..
dotnet test service/OpenMonitorAdvanced.slnx
cargo test -p oma-win -- --include-ignored      # also runs tests that need real GPU hardware
```

### Build the installer

```bash
pwsh scripts/build-installer-payload.ps1
cd app
pnpm tauri build --bundles nsis
```

The payload script publishes `oma-service` (self-contained, trimmed, single file, win-x64) and
downloads the official PawnIO 2.2.0 setup, checked against the SHA-256 in
`app/src-tauri/nsis/pawnio.sha256`. PawnIO is never committed to this repository. The installer
ends up in `target/release/bundle/nsis/`.

## Project layout

| Path | Contents |
|---|---|
| `crates/oma-core` | Data model, sampling scheduler, per-source merge, history. No Windows code. |
| `crates/oma-win` | Windows providers: PDH, D3DKMT, DXGI, NVML, NVAPI, ADL, IGCL, disks, network |
| `crates/oma-ipc` | Protocol types, MessagePack encoding and framing for talking to `oma-service` |
| `app/src-tauri` | Tauri 2 shell (`oma-app`): commands, tray, window, safe mode, NSIS template and hooks |
| `app/src` | Svelte 5 + TypeScript UI, English and Italian translations |
| `service/` | `oma-service`, a .NET 10 Windows service built on LibreHardwareMonitorLib, with its tests |
| `protocol/fixtures/` | Reference MessagePack messages shared by the Rust and .NET protocol tests |
| `docs/` | Design spec, milestone plans and performance budget (in Italian) |
| `scripts/` | Installer payload, footprint measurement and other build scripts |

## Contributing

Issues and pull requests are welcome. Before opening a pull request, run the checks under
[Tests and checks](#tests-and-checks). Code, comments and commit messages are in English and follow
[Conventional Commits](https://www.conventionalcommits.org/). The design documents in `docs/` are
written in Italian.

## License

OpenMonitor Advanced is free software, licensed under the
[GNU General Public License v3.0 or later](LICENSE). You may use, study, share and modify it; if
you distribute a modified version, you must release its source code under the same license.

Third-party components and their licenses are listed in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
