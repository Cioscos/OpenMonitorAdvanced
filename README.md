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

> **Status:** early development (version 0.4.1). Expect rough edges and breaking changes between
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
- **Settings view.** Language, temperature in °C or °F, network speed in bit/s or byte/s,
  sampling interval (0.5 to 5 s), chart refresh rate (60, 30 or 15 FPS), default view, closing
  to the tray, starting with Windows and the sensor shown by the tray icon. The *Rules and
  alerts* section lists the built-in rules and your own (see below). The *CSV log* section sets
  the log's folder, sensors, interval, size limit and hotkeys (see below). The *Data sources*
  section switches each GPU vendor library, anti-cheat compatible mode, each service module
  and, per disk, SMART reads on or off, and shows the PawnIO status. *About* lists the versions and
  the licences, checks for updates (see [Update check](#update-check)) and exports the sensor
  report (see [Sensor report](#sensor-report)). Settings are stored in `%APPDATA%\OpenMonitorAdvanced\settings.json`.
- **Tray icon.** The icon shows the chosen sensor live: a temperature as a number, a load as a
  vertical bar. It turns amber or red while a rule is in warning or critical, and the tooltip
  then starts with the problem before listing CPU, GPU and RAM. The menu opens the Simple or the
  Advanced view directly.
- **Rules and alerts.** Built-in rules watch CPU, GPU and disk temperatures, GPU throttling, RAM
  use, full volumes, NVMe wear and the NVMe critical warning. The CPU thresholds follow the
  processor's own TjMax when it is known. Each rule has a warning and a critical level, each with
  a threshold and how long the value must stay past it, plus a hysteresis. You can change them,
  switch them off, restore them, or add your own rule on any sensor, also with *Create rule…*
  from a sensor in the Advanced view. The Simple view's banner says whether everything is fine,
  or which problems are active and for how long. Entering the critical level also raises a
  Windows notification (the warning level can do so too, if you switch it on), at most once every
  5 minutes per rule, device and level; clicking it opens that device's page.
- **CSV log.** A tape recorder in the top bar opens a pop-up deck with record, pause and stop;
  the tray menu has the same items, and a small dot on the tray icon shows while a recording runs.
  A global hotkey, **Ctrl+Alt+Shift+R** by default, starts and stops it even with the window
  closed, and an optional second hotkey pauses and resumes. The file is UTF-8 with a BOM,
  comma-separated, with CRLF line ends: one row per sampled tick (or every 1 to 60 ticks), a
  local timestamp with its UTC offset, and one column per sensor, named
  `Device / Sensor [unit] {id}`. A cell is empty when the sensor has no value, and holds the
  word `suspended` (in every language) while a reading is on hold because the disk is asleep
  or idle. Files go to `Documents\OpenMonitor Advanced\logs` unless you pick another folder.
  A new part (`-part2`, `-part3`, ...) starts when the file reaches the size limit
  (100 MiB by default, 10 to 2048) or when the columns change: language, units or the selected
  sensors. *Settings › CSV log* sets the folder, the sensors, the interval, the size limit and
  the hotkeys. A failure such as a removed USB drive stops the recording with a notification and
  the reason in the deck.
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

Upgrading needs no confirmation: running a newer setup over an installed version closes the
running app by itself and starts it again afterwards. From 0.4.1 on, the app is asked to quit
and is forced closed only if it is still running after 10 seconds; 0.4.0 does not know how to quit
on request, so an upgrade from 0.4.0 closes it at once. *Start with Windows* stays as it was.
A silent install (`/S`) brings the app back minimized to the tray, and only if it was running.
Uninstalling from *Settings › Apps* still asks before closing the app. You can also
close the running app from a terminal with `oma-app.exe --quit`.

## Verify your download

From 0.3.0 on, each release lists the installer's SHA-256 in `SHA256SUMS.txt`, and GitHub attests that the
installer was built from this repository by the release workflow.

```powershell
# Compare this hash with the one in SHA256SUMS.txt
Get-FileHash -Algorithm SHA256 .\OpenMonitor.Advanced_<version>_x64-setup.exe

# Check the build provenance (needs the GitHub CLI)
gh attestation verify .\OpenMonitor.Advanced_<version>_x64-setup.exe --repo Cioscos/OpenMonitorAdvanced
```

Code signing is planned but not active yet; see [CODE_SIGNING.md](CODE_SIGNING.md) for the policy
and for what it will cover.

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
  everyone. To let hard disks sleep, a hard disk that is asleep or idle is not queried: its
  temperature and SMART values are not updated until it works again, with or without the
  service, and the page shows the last value, greyed out, as *Last reading*. A standby that the
  disk chooses by itself (its own firmware timer), which Windows does not know about, shows as
  *Idle* rather than *In standby*. If a hard disk is asleep when the service starts, no disk's
  SMART values (NVMe included) appear until that disk wakes up. SMART reads of USB disks are off
  by default, because standby behind a USB adapter could not be tested: you can switch them on
  per disk in *Data sources*, but some adapters may then keep the disk awake. A disk plugged in
  while SMART reads are on, that LibreHardwareMonitor cannot identify, may be woken whenever
  another device is plugged in or removed. CPU thermal throttling is not available from
  LibreHardwareMonitor 0.9.6, so that rule has no sensor. The disk critical
  warning covers NVMe drives only. The processor's TjMax is known for Intel CPUs and for the AMD
  desktop models in the built-in table; other CPUs use the fallback thresholds (85/95 °C). Excel
  with a semicolon as the list separator (many European locales) shows the CSV log in one column:
  open it with *Data* → *From Text/CSV* and choose the comma.
  The *Start with Windows* entry of a standard user survives an uninstall of the per-machine
  installer and then points at a program that is gone: remove it in *Settings › Apps › Startup*
  (or turn the option off before uninstalling).

## Frame metrics (preview)

The Advanced sensors component also installs Intel's
[PresentMon](https://github.com/GameTechDev/PresentMon) 2.6.0 console, unmodified and signed by
Intel, next to the service. The service runs it to read the frame times of the game in the
foreground: displayed and rendered FPS, frametimes, 1% and 0.1% lows, stutter, latency. They are
meant for an in-game overlay, which is still being built.

- **Off unless asked.** PresentMon runs only while the app asks for frame metrics. In this
  version only a diagnostic switch does: set the environment variable `OMA_FRAMES_DEBUG` to `1`,
  `pcl` or `all` before starting the app, and it writes one `frames:` line per second to its log
  in `%LOCALAPPDATA%\OpenMonitorAdvanced\logs`. PresentMon traces through its own ETW session,
  `OpenMonitorAdvanced-Frames`, and never stops the sessions of other tools.
- **The game is not touched.** No handle on the game's process, no reads of its memory, no input
  sent to it. The service checks PresentMon's SHA-256 before every start and refuses a modified
  copy.
- **PC latency is not entirely passive.** Measuring PC latency (`pcl` or `all`) turns on
  NVIDIA's PCL Stats events. A game that supports Reflex then starts a ping every 100–300 ms
  while it is in the foreground: a window message to its own window or, if the game is set up
  that way, synthesized F13–F15 key presses. This is how NVIDIA designed it (FrameView and
  PresentMon rely on it) and it is harmless, but a program that reacts to F13–F15 may see those
  keys. It is never on unless you ask for it.
- **Known limits.**
  - Frame generation: with PC latency on, the rendered FPS are known for DLSS FG, FSR FG and
    NVIDIA Smooth Motion in games with Reflex, and for Intel XeSS-FG and AMD AFMF from the
    driver. Without Reflex, or with PC latency off, the rendered FPS are unknown: DLSS FG and
    Smooth Motion show *FG?* instead of a number, FSR FG only the displayed FPS.
  - G-Sync/FreeSync: a visible overlay window may make Windows compose the game instead of
    using independent flip, on hardware without a free MPO plane. On an RTX 4080 it did not:
    independent flip and G-Sync stayed on with the window visible, empty or hidden.
  - Exclusive fullscreen: the overlay will not be visible over a game in true exclusive
    fullscreen; the measurement works.
  - Anti-cheat: some anti-cheat systems refuse the ETW session if it starts after the game
    (turn the frame metrics on before starting the game), and some block external windows too.
  - HDR: the overlay content is SDR, composed by the desktop window manager.

## Update check

The app can tell you when a newer release is out. In *Settings › About*, **Check now** asks
GitHub once; **Check automatically (once a day)** does the same daily and is **off by default**.

- **What it sends.** A single HTTPS request to `api.github.com` for the latest release of this
  repository. GitHub sees your IP address, as with any connection, and a User-Agent with the app
  version. No identifiers, settings or sensor data are sent.
- **What it does.** It shows whether you have the latest version and, if not, a link to the
  release page. It never downloads or installs anything. With the automatic check on, a new
  version raises one Windows notification per version, and a dot on *About* and on the settings gear stays while the
  update is available.
- Without a click on *Check now* or the automatic check switched on, the app makes no network
  request at all. The service never uses the network. See the privacy section of
  [CODE_SIGNING.md](CODE_SIGNING.md#privacy).

## Sensor report

**Export sensor report** in *Settings › About* saves `oma-report-YYYYMMDD-HHMMSS.json` in a folder
you choose. Nothing is sent: attach the file to an issue yourself.

- **What it contains:** the app, service and protocol versions, the Windows version, the service
  and anti-cheat state, safe mode, which data sources are on, every device with its model, vendor
  and a fixed list of static details, and every sensor with its current value, quality and the
  session's minimum, average and maximum.
- **What it does not contain:** disk and network identifiers (replaced by `storage/disk-N` and
  `network/adapter-N`), network adapter names (replaced by *Ethernet N*, *Wi-Fi N* or
  *Adapter N*), volume GUIDs (replaced by running numbers), paths, user or computer names,
  network addresses, rules, other settings and history.

## Reporting a problem

Open an [issue](https://github.com/Cioscos/OpenMonitorAdvanced/issues) and describe what you
expected and what happened. Please **attach a sensor report** (see [Sensor report](#sensor-report)):
it tells us which hardware, sensors and sources your PC has, without identifying it. For a sensor
that is missing or wrong, export the report while the problem is visible.

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

The payload script publishes `oma-service` (self-contained, trimmed, single file, win-x64),
downloads the official PawnIO 2.2.0 setup, checked against the SHA-256 in
`app/src-tauri/nsis/pawnio.sha256`, and the official PresentMon 2.6.0 console, checked against
`app/src-tauri/nsis/presentmon.sha256` and Intel's signature. Neither is ever committed to this
repository. The installer
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
| `testdata/presentmon/` | Anonymised PresentMon CSV captures used by the frame-metric tests |
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

Third-party components are described in two files, both installed with the app and opened from
*Settings › About*:

- [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), written by hand: the sources of the GPU
  bindings, LibreHardwareMonitor, PawnIO and the other bundled components;
- [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt), generated by
  `pwsh scripts/generate-licenses.ps1` and checked in CI: the licence texts of every redistributed
  Rust crate, JavaScript package and NuGet package, and of the .NET runtime.
