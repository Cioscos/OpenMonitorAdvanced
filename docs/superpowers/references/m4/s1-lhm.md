# Spike S1 — LibreHardwareMonitorLib 0.9.6 on this machine

Throwaway feasibility spike for M4 (`oma-service`). Machine: Ryzen 7 7800X3D, Gigabyte B650 GAMING X AX (BIOS FC4c), 2× Corsair DDR5 CMH32GX5M2B6400C36, 4 disks (Seagate ST2000DM008 SATA HDD, Corsair Force LS SATA SSD, Fanxiang S880 NVMe, SK hynix SHPP41-2000GM NVMe), Windows 11 26200, .NET SDK 10.0.303 (runtime 10.0.12), PawnIO 2.2.0 installed.

**Status: complete. Unelevated §1–§8; elevated results (run by the user on 2026-09-26) in §9, which supersedes the "to be measured/confirmed" notes above.**

Everything lived in the spike folder `<spike>\lhm-dump\` (temporary; the folder itself is not guaranteed to exist in a clean checkout — see the project README in this directory):

| Path | Content |
|---|---|
| `LhmDump/` | console app (net10.0-windows, win-x64, `LibreHardwareMonitorLib` 0.9.6) |
| `pub-untrimmed/LhmDump.exe`, `pub-trimmed/LhmDump.exe` | self-contained single-file builds |
| `dump-*-user.json` + `*.report.txt` | unelevated dumps + LHM `Computer.GetReport()` |
| `compare.ps1` | diff of two dumps (see §7) |
| `compare-user.txt` | untrimmed vs trimmed, unelevated |
| `trim-warnings.txt`, `publish-trimmed.log` | every trim warning (`TrimmerSingleWarn=false`) — versioned copy in [`trim-warnings.md`](trim-warnings.md) |
| `run-admin.txt` | the elevated command of §7 |
| `lhm-src/` (tag v0.9.6, `3d331e3`), `dit-src/` (DiskInfoToolkit `25319ea` = NuGet 1.1.2), `spd-src/` (RAMSPDToolkit `3b47b96` = 1.4.2), `bs-src/` (BlackSharp.Core `c70b735` = 1.0.7) | sources read for §6 (third-party; cited by tag/commit, not copied here) |

## 1. What LhmDump does

`new Computer { Cpu, Motherboard, Memory, Storage, Controller, Psu = true; Gpu, Network, Battery = false }` → `Open()` (timed) → `Update()` every hardware and sub-hardware (flattened, each timed) → 1 s → update → **dump** the tree (HardwareType, `Identifier`, Name, parent, .NET type, `Properties`, every sensor with Identifier/Name/SensorType/Index/Value/Min/Max/IsDefaultHidden/Parameters; for storage also every public and non-public member of `DiskInfoToolkit.Storage` and of its internal `StorageDevice`, one level deep) → 10 more cycles at 1 s (per-hardware median/max) → `GetReport()` → `Close()` (timed). Process private bytes / working set / handles / threads are recorded at start, after `Open`, after the 12 cycles, after `GetReport`, after `Close`, after `Close`+GC. Also recorded: elevation, PawnIO version (`LibreHardwareMonitor.PawnIo.PawnIo.Version`), `HardwareAdded/Removed` events with timestamps, and — to compare identity with the core — the `STORAGE_DEVICE_DESCRIPTOR` (vendor/product/revision/raw serial/bus) of every `\\.\PhysicalDriveN` opened with access 0, exactly as the unprivileged core reads it. With `--handles` it also counts the process's own handles named `\Device\PawnIO` at each step (own-process `NtQuerySystemInformation(64)` + `NtQueryObject`; self-tested by matching `\Device\`), then calls `RAMSPDToolkit…DriverManager.UnloadDriver()` and counts again (see §6d). The scan allocates, so it is opt-in and the memory figures come from runs without it.

## 2. Unelevated result (both builds identical)

`Open()` throws nothing. 4 hardware, 68 sensors, 16 of them `null`. No storage, no DIMMs, no Super I/O, no controllers/PSUs (none attached).

| HardwareType | Identifier | Name | .NET type | Parent | Sensors |
|---|---|---|---|---|---|
| Motherboard | `/motherboard` | Gigabyte B650 GAMING X AX | `…Motherboard.Motherboard` | — | 0 (no `/lpc/…` sub-hardware: LPC needs PawnIO) |
| Cpu | `/amdcpu/0` | AMD Ryzen 7 7800X3D | `…Cpu.Amd17Cpu` | — | 62 |
| Memory | `/vram` | Virtual Memory | `…Memory.VirtualMemory` | — | 3 |
| Memory | `/ram` | Total Memory | `…Memory.TotalMemory` | — | 3 |

`Properties` is empty for all of them (in LHM only GPU hardware fills it: `Hardware.cs:70`, `Motherboard.cs:143` return a new empty dictionary).

CPU sensors unelevated (`/amdcpu/0/…`):

| Identifier | Name | Unelevated value |
|---|---|---|
| `load/0` | CPU Total | real (≈ PDH total) |
| `load/1` | CPU Core Max | real |
| `load/2` … `load/17` | CPU Core #1 … #16 | real; **one per logical processor** (index = thread + 2) |
| `power/0` | Package | **0** (bogus) |
| `clock/1`, `clock/2` | Cores (Average), Cores (Average Effective) | **0** |
| `clock/3,5,…,17` | Core #1…#8 | `null` |
| `clock/4,6,…,18` | Core #n (Effective) | **0** |
| `factor/0…7` | Core #n (multiplier) | `null` |
| `power/1…8` | Core #n (SMU) | **0** |
| `voltage/2…9` | Core #n VID | **1.55** (bogus: VID formula applied to a zero register) |
| `temperature/2` | Core (Tctl/Tdie) | **0** |

`/ram`: `data/0` Memory Used (GB), `data/1` Memory Available (GB), `load/0` Memory (%). `/vram` (commit charge): `data/2` Used, `data/3` Available, `load/1` Memory.

**Warning for the conversion:** without a working PawnIO handle LHM does not return `null`, it returns plausible zeros and a constant 1.55 V. `PawnIo.Execute` returns a zero-filled array when the module is not loaded (`PawnIo/PawnIo.cs:98-121`). The service must refuse to publish PawnIO-backed sensors unless it is elevated and `PawnIo.IsInstalled`; the per-module `IsLoaded` is private, so a sanity check (e.g. Tctl == 0) is the only runtime signal.

Identifier format (from source, `Identifier.cs:21-52`, `Sensor.cs:84`): hardware `/<segments>`, sensor `<hardware id>/<sensortype lower-case>/<index>`. Hardware ids in 0.9.6: `/amdcpu/<n>`, `/intelcpu/<n>`, `/genericcpu/<n>` (`GenericCpu.cs:134-142`); `/motherboard`; `/lpc/<chip lower>/<n>` (Super I/O, sub-hardware of the motherboard, `SuperIOHardware.cs:33`); `/lpc/ec`; `/ram`, `/vram`, `/memory/dimm/<spd index>` (`MemoryGroup.cs:189`); storage `/nvme|ssd|hdd/<DriveNumber>` (§6a); `/psu/corsair/<n>`, `/psu/msi/<n>`; HID controllers `new Identifier(HidDevice)` (VID/PID/serial), `/heatmaster/<port>`, `/bigng/<n>`.

## 3. Timings and memory (unelevated; elevated to be measured)

| | untrimmed | trimmed |
|---|---|---|
| `Open()` | 146 ms (2 runs: 146, 151) | 183–212 ms |
| `Close()` | 5.2 ms | 10.4 ms |
| `/amdcpu/0` `Update()` median / max | 0.9–8.5 ms / 2–156 ms | 1.3–22 ms / 12–282 ms |
| `/ram`, `/vram`, `/motherboard` `Update()` | < 0.01 ms | < 0.01 ms |

The CPU update variance is not trimming: it is the same exe across runs. `GenericCpu.Update` pins the calling thread to each logical processor in turn (`ThreadAffinity.Set`, `GenericCpu.cs:230-262`, `Amd17Cpu.cs:164/563/674`), so it waits for each core to be schedulable; with the user working on the PC this costs up to ~0.3 s. **The service must update LHM on its own thread, never on the pipe I/O thread, and publish the last completed snapshot.**

| Memory (MB, private / working set) | untrimmed | trimmed |
|---|---|---|
| process start | 8.1 / 27.0 | 11.3 / 27.1 |
| after `Open()` | 13.0 / 42.6 | 14.1 / 36.8 |
| after 12 cycles | 13.4 / 44.2 | 11.6 / 34.4 |
| after `Close()` (includes a `GetReport()`) | 18.0 / 49.3 | 16.3 / 39.7 |
| after `Close()` + GC | 18.1 / 50.2 | 16.1 / 40.3 |

Threads go 8 → 34 at `Open()` and stay after `Close()` (DiskInfoToolkit `StorageManager` starts a device-change listener thread and a hidden-window message loop in its static constructor, `StorageManager.cs:37-52`, never stopped; plus HidSharp watchers and pool threads). Handles 225 → 433 at `Open()`.

**Long-running memory risk (source):** every `Sensor` keeps a history of values averaged every 4 samples with a default window of **1 day** (`Sensor.cs:26`, `Sensor.cs:111-131`, pruned with `List.RemoveAt(0)`). At 1 s that is 21 600 entries × ~16 B per sensor per day, i.e. ~0.35 MB per sensor; with a few hundred sensors elevated, tens of MB and an O(n) prune per set. The service must set `sensor.ValuesTimeWindow = TimeSpan.Zero` on every sensor (also from `SensorAdded`/`HardwareAdded`).

## 4. Sizes and trimming

| Build | exe | deflate-9 | LZMA (≈ NSIS solid LZMA) |
|---|---|---|---|
| untrimmed, self-contained single-file | 74.9 MiB (78 556 268 B) | 32.3 MiB | 24.3 MiB |
| trimmed (`PublishTrimmed=true`, **`TrimMode=full` works**) | 17.5 MiB (18 394 149 B) | 7.6 MiB | 5.8 MiB |

(`IncludeNativeLibrariesForSelfExtract=true`, no `EnableCompressionInSingleFile`; LhmDump itself is a few kB.)

Trim warnings (versioned, accepted list in [`trim-warnings.md`](trim-warnings.md)), library ones only (the 6 in `Program` are the spike's own reflection/JsonNode code, see `trim-warnings.md`):

| Warning | Where | Relevance on Windows |
|---|---|---|
| IL2026 ×2, IL2075 ×3 | `LibreHardwareMonitor.Hardware.OpCode.Open/Close` — `Assembly.GetType("Mono.Unix.Native.Syscall")` | Unix-only branch (`OpCode.cs:210-285`); not reached on Windows |
| IL2067, IL2077 | `System.Management.MTAHelper` | only when WMI is called from an STA thread; LHM's only WMI use in our modules is `Ipmi.IsBmcPresent()` (`LpcIO.cs:30`, `Ipmi.cs:269-281`) called from `Open()` on the caller's thread — keep the service's thread MTA (default) |
| IL2075 | `HidSharp.Platform.Linux.NativeMethods.uname` | Linux only |
| IL2091 | `MsiCoreLiquidController.BytesToStruct<T>` (Controller module) | `T : struct` (`MsiCoreLiquidController.cs:325`), no constructor needed; sequential structs keep their fields — safe, but untestable here (no MSI AIO) |
| IL2091 | `WireViewPro2.BytesToStructure<T>` | PowerMonitor module, disabled |

LibreHardwareMonitorLib does not declare `IsTrimmable` and contains no `UnconditionalSuppressMessage`. **Unelevated diff untrimmed vs trimmed: identical** — same 4 hardware ids, same 68 sensor ids, same 16 null sensors (`<spike>\compare-user.txt`; the only differences are live load values). The deciding comparison (spec §5.3: trimmed only if identical) is the elevated one, still to be run (§7). The trimmed build uses ~6–8 MB less working set.

## 5. Mapping to the core (answers 2e and inputs for the conversion)

Core ids read from `crates/oma-win/src/{cpu,memory,storage,storage_temperature}.rs`.

| LHM sensor | Core sensor (same quantity?) | Proposal |
|---|---|---|
| `/amdcpu/0/load/0` CPU Total | `cpu/0/load/total` (PDH `% Processor Utility`) | canonical name `total` → duplicate, core wins |
| `/amdcpu/0/load/{t+2}` "CPU Core #n" | `cpu/0/load/thread-<group>-<number>` | map `thread-0-<t>` only when there is a single processor group (LHM's `Thread` is a flat index); LHM also mis-groups threads on this CPU (16 "cores", no "Thread #" suffix — CPUID APIC grouping, `GenericCpu.cs:62-79`), so never infer cores from these names |
| `/amdcpu/0/load/1` CPU Core Max | — | new sensor |
| `/amdcpu/0/clock/2` Cores (Average Effective) | `cpu/0/clock/effective` (PDH estimate) | **decision needed:** same name → core's estimate wins and LHM's is dropped; different name → both shown |
| `/amdcpu/0/clock/*`, `power/*`, `voltage/*`, `temperature/*`, `factor/*` | — | new (per-core effective clock is what spec §5.1 expects from the service) |
| `/ram/load/0` Memory (%) | `memory/0/load/used` | duplicate, core wins |
| `/ram/data/0` Memory Used (GB, 2^30) | `memory/0/data/used` (bytes) | duplicate (unit differs) |
| `/ram/data/1` Memory Available | — (core has `data/total`) | drop: derivable, redundant |
| `/vram/*` (commit charge) | — | drop (not physical memory; LHM's "Virtual Memory") |
| `/memory/dimm/<i>` temperature + timings/limits/capacity | — | temperature = sensor; timings, thermal limits, capacity are constants exposed as sensors (`DimmMemory.cs:77-115`) → turn into device properties |
| storage `temperature/0` (Temperature / Composite Temperature) | `storage/…/temperature/drive` | duplicate, core wins (core reads it unprivileged) |
| storage `temperature/1…8` (NVMe "Temperature #n", only if SMART reports them) | `storage/…/temperature/sensor-<n>` | likely duplicate but indices come from different sources (NVMe SMART TS1–TS8 vs driver property positions): verify on the elevated dump before sharing names |
| storage `temperature/10,11` Warning/Critical (NVMe) | device properties `tempWarningC`, `tempCriticalC` | properties, not sensors |
| storage `load/53` Total Activity | `storage/…/load/active` (PDH % idle) | duplicate |
| storage `throughput/54,55` Read/Write Rate | `storage/…/throughput/read`, `write` | duplicate |
| storage `load/51,52` Read/Write Activity | — | drop or new (low value) |
| storage `load/30` Used Space, `data/31` Free Space, `data/32` Total Space | core has per-volume `percent/volume-<id>`, `data/volume-<id>-free` | different granularity (whole disk, GB): drop |
| storage `level/20` Life, `data/21,22` Data Read/Written, `factor/23` Power On Count, `factor/24` Power On Hours, SMART attribute sensors | — | new; these are the SMART/health values that spec §5.3 wants every 30 s |

## 6. Source-code answers

Links: LHM `https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/v0.9.6/LibreHardwareMonitorLib/…`, DiskInfoToolkit `https://github.com/Blacktempel/DiskInfoToolkit/blob/25319eae5781e75bcf141e844ceab2afe94d40ea/DiskInfoToolkit/…`, RAMSPDToolkit `https://github.com/Blacktempel/RAMSPDToolkit/blob/3b47b960e0830fef344624ad5e389675d5f0a1ce/RAMSPDToolkit/…`, BlackSharp `https://github.com/Blacktempel/BlackSharp/blob/c70b735c6cec123ee8a046ac4a0bc6c606f52cf0/BlackSharp.Core/…`. Paths below are relative to those roots.

### (a) Storage ↔ `\\.\PhysicalDriveN`

- LHM storage is a thin wrapper over **DiskInfoToolkit** (`Hardware/Storage/StorageGroup.cs:35-46` → `StorageManager.ReloadStorages()`).
- Identifier = `/<nvme|ssd|hdd>/<DriveNumber>` (`Hardware/Storage/StorageDevice.cs:48`, prefix from `StorageGroup.cs:72-80`: `IsNVMe` → `nvme`, else `IsSSD` → `ssd`, else `hdd`).
- `DriveNumber` is `IOCTL_STORAGE_GET_DEVICE_NUMBER.DeviceNumber` on the disk interface path (`StorageDetector.cs:126-128/158`, `270-283`; `Storage.cs:317-325`), or the probe index `i` for disks found only by probing `\\.\PhysicalDrive0…63` (`StorageManager.cs:125-161`). So it **is** the N of `\\.\PhysicalDriveN` (−1 if the IOCTL fails).
- **Publicly available, no reflection:** `LibreHardwareMonitor.Hardware.Storage.StorageDevice` is `public sealed` with `public DiskInfoToolkit.Storage Storage` (`StorageDevice.cs:23, 59`); `DiskInfoToolkit.Storage` exposes `DriveNumber`, `PhysicalPath` (the SetupAPI interface path `\\?\scsi#disk&…`, not `\\.\PhysicalDriveN`), `DeviceID` (PnP instance id), `SerialNumber`, `Model`, `Firmware`, `FirmwareRev`, `VendorID`, `BusType`, `IsNVMe`, `IsSSD`, `TotalSize` (`Storage.cs:146-247`). The dump records all of them in `storageIdentity` (elevated only). The CPU hint is also public: `GenericCpu.Index` (`GenericCpu.cs`, `public class`).
- **Serial caveat (important for the identity check):** DiskInfoToolkit first takes the descriptor serial (`Storage.cs:579`) but then **overwrites** it with the ATA/NVMe IDENTIFY serial (`Disk/DiskHandler.cs:245` NVMe, `:352` ATA). The core hashes the `STORAGE_DEVICE_DESCRIPTOR` serial. On this PC the descriptor serials are `"            <SERIAL-HDD>"` (HDD, space-padded), `"<SERIAL-SSD>"` (SATA SSD), `<EUI64-NVME-1>.` and `<EUI64-NVME-2>.` (NVMe: EUI-64-style, not the IDENTIFY serial). **They will not match LHM's `SerialNumber` for NVMe.** Proposal: the service reads the descriptor itself on `\\.\PhysicalDriveN` (access 0, as the core does) and sends that as the check value, or sends `DriveNumber` + IDENTIFY serial and the client compares loosely. The elevated dump puts both side by side (`physicalDrivesDescriptor` vs `storageIdentity`).
- `DriveNumber` is not stable across reboots or hot-plug, so LHM's storage identifier is not stable either: never hash it into an id.

### (b) Power state / sleeping HDD, what a storage `Update()` does

- **No power-state check anywhere.** DiskInfoToolkit has no CHECK POWER MODE (0xE5), no `GetDevicePowerState`, nothing (grep of the whole source). Instead it **wakes the drive on purpose**: `DiskHandler.WakeUp` reads sector 0 with `ReadFile` (`Disk/DiskHandler.cs:228-234`); it is called when a SMART read fails for physical-drive/CSMI commands (`:80`, `:117`), **unconditionally before every read** for USB/SAT bridges and Realtek (`:142`, `:152`), and during identification at `Open()` (`Identifiers/DeviceIdentifier.cs:41, 53, 127`). ATA SMART READ DATA itself may spin up a drive in standby.
- `StorageDevice.Update()` (`StorageDevice.cs:65-94`), on **every call** unless the static `StorageDevice.ThrottleInterval` (default `TimeSpan.Zero`, global for all disks; the LHM GUI sets 30 s, `LibreHardwareMonitor/UI/MainForm.cs:439`) says otherwise:
  1. opens the disk with `FileAccess.ReadWrite` and reads `IOCTL_DISK_PERFORMANCE` (activity and rates) (`:265-313`);
  2. `Storage.Update()` (`Storage.cs:294-303`): opens `GENERIC_READ|GENERIC_WRITE` (`SafeFileHandler.OpenHandle`, BlackSharp `Interop/Windows/Utilities/SafeFileHandler.cs:26-29`), full SMART read (`DiskHandler.UpdateSmartInfo`, `Disk/DiskHandler.cs:34-226`: NVMe SMART/health log or ATA SMART attributes, every call), then re-reads the partition layout and volume free space;
  3. refreshes all sensors.
- Consequences: SMART cannot be separated from the rate/activity sensors (one `Update()` does all); since those are duplicates of the core, the service can simply update storage hardware only every 30 s (spec §4.1/§5.3). To avoid waking the ST2000DM008, the service needs its own check before `Update()` for rotational disks (`!IsNVMe && !IsSSD`): e.g. ATA CHECK POWER MODE via `IOCTL_ATA_PASS_THROUGH` (the service is elevated) — not provided by LHM; to be verified on the HDD in a follow-up.

### (c) `Computer.Open()` without admin

`Open()` (`Hardware/Computer.cs:508-521`) creates `SMBios`, `Mutexes.Open()`, `OpCode.Open()`, then `AddGroups()` (`:523-573`) with **no try/catch around group constructors** — any throwing group would abort `Open()`. Observed unelevated: nothing throws.

| Module | Unelevated behaviour | Source |
|---|---|---|
| Mutexes (`Global\Access_ISABUS.HTP.Method`, `Access_PCI`, `Access_EC`, …) | created or opened; failures caught → `null` | `Hardware/Mutexes.cs:19-55` |
| SMBIOS | works (`GetSystemFirmwareTable`) | — |
| Motherboard | `/motherboard` with 0 sensors; LPC port probes read through a PawnIO module that is not loaded → reads return 0 → no Super I/O; `Ipmi.IsBmcPresent()` WMI query, exceptions caught | `Motherboard/Lpc/LpcIO.cs:20-31`, `Lpc/LpcPort.cs`, `Lpc/Ipmi.cs:269-281` |
| CPU | CPUID works (user-mode `OpCode`), PawnIO modules fail silently (`LoadModuleFromResource` returns a not-loaded instance, `PawnIo/PawnIo.cs:65-90`), all MSR/SMU readings are 0 (see §2) | `Cpu/Amd17Cpu.cs:21-33` |
| Memory | `/ram`, `/vram` always; RAMSPDToolkit driver "loads" (`RAMSPDToolkitDriver.Load()` always true), SMBus module loads fail → no DIMM; a retry task tries 4 more times every 2.5 s | `Memory/MemoryGroup.cs:30-51, 125-143`, `RAMSPDToolkitDriver.cs:16-22` |
| Storage | every disk open needs `GENERIC_READ|GENERIC_WRITE` → access denied → `Storage.IsValid = false` → **zero storage hardware, silently** | `Storage.cs:58-66`, BlackSharp `SafeFileHandler.cs:26-29` |
| Controller (HID/serial), PSU (HID) | HidSharp enumeration works unprivileged; nothing attached here | — |

### (d) Does `Computer.Close()` close the PawnIO handle(s)?

**Partly.** `Close()` (`Computer.cs:651-670`) removes each group → `group.Close()`:

- CPU: `Amd17Cpu.Close()` closes `AmdFamily17` and `RyzenSMU` (`Cpu/Amd17Cpu.cs:81-86`); Intel/other families likewise.
- Motherboard: Super I/O chips close their `LpcPort` (PawnIO `LpcIO` module, `Lpc/LpcPort.cs:116`), EC and Gigabyte ISA bridge close theirs.
- **Memory: not closed.** `MemoryGroup.Close()` (`MemoryGroup.cs:82-96`) never calls `DriverManager.UnloadDriver()`. The SMBus modules loaded by RAMSPDToolkit (`I2CSMBus/SMBusPawnIO.cs:152-183`: I801, else PIIX4 twice — two handles on AMD — else NCT6793) stay in `RAMSPDToolkitDriver._pawnIOModules` (`RAMSPDToolkitDriver.cs:14, 64-68`) and in the static `SMBusManager` list. The static `DriverManager.Driver` is reused on the next `Open()` (its `IsOpen` is always `true`), and `DetectSMBuses()` clears the list and loads new modules (`SMBusManager.cs:85-87`), so **every Open/Close cycle leaks the SMBus handles**.
- Failed module loads (e.g. the I801 attempt on AMD) leave their `SafeFileHandle` unclosed until the GC finalizes it (`PawnIo.cs:85-89`).
- Workaround for the service: after `computer.Close()` call `RAMSPDToolkit.Windows.Driver.DriverManager.UnloadDriver()` (public, `Windows/Driver/DriverManager.cs:123-127`; it sets `Driver = null`, so the next `MemoryGroup` recreates it), then `GC.Collect(); GC.WaitForPendingFinalizers()`. The `--handles` elevated run (§7) measures the `\Device\PawnIO` handle count after `Open`, after the cycles, after `Close`, after GC and after `UnloadDriver`, to confirm that this returns to 0 (spec §2.2 requires "stopping the service closes every PawnIO handle"; with the service process exiting that is guaranteed anyway, but not on the "last client leaves" path).
  - **Superseded in M4 Task 15:** the service no longer does this. `UnloadDriver()` nulls the modules (`RAMSPDToolkitDriver.cs:64-68, 82-89`) that RAMSPDToolkit's `~SPDAccessor` still uses to reset the SPD page (`SPD/SPDAccessor.cs:33-37`), so the forced GC that followed crashed the stopping service with a NullReferenceException on the finalizer thread (exit 1, event 7031, SCM restart). LHM now stays open until the process exits and `Dispose` is `Close()` only; the OS releases the handles at exit.

### (e) Duplicates of the unprivileged core

See the table in §5: CPU total and per-thread load; RAM load and used; storage read/write rate, total activity and drive temperature (plus the NVMe extra temperatures, to verify); NVMe warning/critical thresholds (core device properties). Storage used/free space overlaps in meaning but not in granularity. `/vram` and `/ram` "Available" have no core counterpart and should be dropped.

## 7. Elevated run (for the user)

Run in an **elevated PowerShell** (about 40 s; it starts and stops LHM three times; it may spin up the ST2000DM008 if it is asleep — DiskInfoToolkit wakes disks, §6b):

```powershell
$d='<scratchpad>\m4\lhm-dump'; & "$d\pub-untrimmed\LhmDump.exe" "$d\dump-untrimmed-admin.json"; & "$d\pub-trimmed\LhmDump.exe" "$d\dump-trimmed-admin.json"; & "$d\pub-untrimmed\LhmDump.exe" "$d\dump-untrimmed-admin-handles.json" --handles
```

(Also saved in `<spike>\run-admin.txt`.) The session scratchpad must still exist when it is run.

Analysis afterwards (normal PowerShell, in `<spike>`):

```powershell
.\compare.ps1 dump-untrimmed-admin.json dump-trimmed-admin.json   # trimming decision (spec §5.3)
.\compare.ps1 dump-untrimmed-user.json dump-untrimmed-admin.json  # what elevation adds
```

`compare.ps1` prints: elevation, open/close ms, hardware/sensor/null counts, memory per step, PawnIO handle counts, update errors; hardware ids and sensor ids present in only one dump; sensors `null` in one but not the other; sensors zero in one and non-zero in the other (informational); per-hardware `Update()` median/max side by side. The admin dumps additionally contain `storageIdentity` (all DiskInfoToolkit members) per disk and `physicalDrivesDescriptor` for the serial comparison of §6a.

Expected from source (to confirm): `/lpc/<chip>/0` Super I/O sub-hardware under `/motherboard` (ITE on this Gigabyte board), `/memory/dimm/<i>` ×2 via SMBus PIIX4, `/nvme/2`, `/nvme/3`, `/ssd/1`, `/hdd/0`, and non-zero CPU temperatures/power/clocks.

## 8. Summary for the M4 plan

- LHM 0.9.6 works in-process on .NET 10, `Open()` ≈ 150–210 ms, `Close()` ≈ 5–10 ms, ~13–14 MB private / 35–45 MB working set unelevated (elevated to be measured).
- **Trimmed full works** and is 17.5 MiB (≈ 5.8 MiB LZMA) vs 74.9 MiB (≈ 24 MiB LZMA) untrimmed; library trim warnings are on Unix-only, STA-only or disabled paths; unelevated sensor sets are identical. Final decision waits for the elevated diff.
- Service must: run LHM updates on a dedicated thread (CPU `Update()` up to ~0.3 s under load); set `ValuesTimeWindow = TimeSpan.Zero` on all sensors; ~~call `DriverManager.UnloadDriver()` + GC after `Close()`~~ (dropped in Task 15: it crashed the stop, see §6d; LHM closes only at process exit); update storage hardware only every 30 s and gate rotational disks with its own power-mode check (LHM/DiskInfoToolkit wake sleeping disks); never trust PawnIO-backed values when not elevated (zeros, 1.55 V).
- Identity hints: CPU `GenericCpu.Index`; disk `((StorageDevice)hw).Storage.DriveNumber` (= PhysicalDriveN, public); the serial check must use the descriptor serial the core uses, not `Storage.SerialNumber` (IDENTIFY serial, differs on NVMe).

## 9. Elevated results (user run, 2026-09-26 13:45)

All three admin dumps exist and are complete (`elevated: true`, 11 hardware, 206 sensors each, no `openException`/`closeException`, no update errors): `dump-untrimmed-admin.json`, `dump-trimmed-admin.json`, `dump-untrimmed-admin-handles.json` (+ `.report.txt`). The trim diff is saved as `<spike>\compare-admin-trim.txt`.

### 9.1 Hardware and sensors (elevated)

| Identifier | HardwareType | Name | .NET type | Sensors |
|---|---|---|---|---|
| `/motherboard` | Motherboard | Gigabyte B650 GAMING X AX | `…Motherboard.Motherboard` | 0 |
| `/lpc/it8689e/0` (sub-hardware of `/motherboard`) | SuperIO | ITE IT8689E | `…Motherboard.SuperIOHardware` | 27 |
| `/amdcpu/0` | Cpu | AMD Ryzen 7 7800X3D | `…Cpu.Amd17Cpu` | 64 |
| `/ram`, `/vram` | Memory | Total Memory, Virtual Memory | `…Memory.TotalMemory`, `…Memory.VirtualMemory` | 3 + 3 |
| `/memory/dimm/1`, `/memory/dimm/3` | Memory | Corsair - CMH32GX5M2B6400C36 (#1), (#3) | `…Memory.DimmMemory` | 21 + 21 |
| `/nvme/3`, `/nvme/2` | Storage | SHPP41-2000GM, Fanxiang S880 2TB | `…Storage.StorageDevice` | 21 + 21 |
| `/hdd/0` | Storage | ST2000DM008-2FR102 | `…Storage.StorageDevice` | 11 |
| `/ssd/1` | Storage | Corsair Force LS SSD | `…Storage.StorageDevice` | 14 |

`Properties` is still empty everywhere. Sensors (values from the untrimmed run, idle desktop; full list in the JSON, which is not checked in — see the project README; `n` = core 1…8, `t` = logical processor 0…15, `i` = SPD index):

| Hardware | Sensor identifier(s) | Name | Type | Value | Null |
|---|---|---|---|---|---|
| cpu | `load/0`, `load/1`, `load/{t+2}` | CPU Total, CPU Core Max, CPU Core #1…#16 | Load | 1.6 %, 7.5 %, 0–7.5 % | no |
| cpu | `temperature/2`, `temperature/3` | Core (Tctl/Tdie), CCD1 (Tdie) | Temperature | 48.1, 39.1 °C | no |
| cpu | `power/0`, `power/{n}` | Package, Core #n (SMU) | Power | 25.8 W, 0.02–0.89 W | no |
| cpu | `voltage/{n+1}` | Core #n VID | Voltage | 0.49–0.54 V (varies) | no |
| cpu | `clock/0`, `clock/1`, `clock/2` | Bus Speed, Cores (Average), Cores (Average Effective) | Clock | 99.8, 3508, 120 MHz | no |
| cpu | `clock/{2n+1}`, `clock/{2n+2}` | Core #n, Core #n (Effective) | Clock | 2662–4841, 11–350 MHz | no |
| cpu | `factor/{n-1}` | Core #n (multiplier) | Factor | 33.7–48.5 | no |
| lpc | `temperature/0…5` | System, PCH, CPU, PCIe x16, VRM MOS, VSoC MOS | Temperature | 33, 42, 48, 38, 44, 45 °C | no |
| lpc | `voltage/0…9` | Vcore, +3.3V, +12V, +5V, Vcore SoC, Vcore Misc, Dual DDR5 5V, +3V Standby, CMOS Battery, AVCC3 | Voltage | 1.02, 3.32, 11.95, 5.01, 1.22, 1.15, 4.98, 3.31, 3.24, 3.07 V | no |
| lpc | `fan/0…4` | CPU Fan, System Fan #1, #2, #3, System Fan #4 / Pump | Fan | 1772, 934, **0**, 885, 2679 RPM | no |
| lpc | `control/0…5` | fan PWM controls | Control | — | **yes (by design)** |
| ram | `data/0`, `data/1`, `load/0` | Memory Used, Memory Available, Memory | Data (GB), Load | 10.5 GB, 20.6 GB, 33.8 % | no |
| vram | `data/2`, `data/3`, `load/1` | Memory Used, Memory Available, Memory (commit charge) | Data (GB), Load | 12.3 GB, 38.8 GB, 24.1 % | no |
| dimm | `temperature/0` | DIMM #i | Temperature | 40.25, 39.5 °C | no |
| dimm | `temperature/1…5` | Resolution, Low, High, Critical Low, Critical High limit | Temperature (constants) | 0.25, **0**, 55, **0**, 85 | no |
| dimm | `timing/20…33` | tCKAVGmin … tRFCsb_dlr | Timing (ns, constants) | 0.416 … 295; the three 3DS ones **0** | no |
| dimm | `data/50` | Capacity | Data (GB) | 16 | no |
| nvme | `temperature/0`, `/1`, `/2` | Composite Temperature, Temperature #1, #2 | Temperature | 47, 40.9, 50.9 °C (nvme/3) | no |
| nvme | `temperature/10`, `/11` | Warning, Critical Temperature | Temperature (constants) | 85/86 (nvme/3), 89/94 (nvme/2) | no |
| nvme, ssd | `level/20`, `data/21`, `data/22` | Life, Data Read, Data Written (GB) | Level, Data | 100/95/100 %, … | no |
| all disks | `factor/23`, `factor/24` | Power On Count, Power On Hours | Factor | e.g. 2414, 9115 | no |
| nvme | `level/100…102` | Available Spare, Available Spare Threshold, Percentage Used | Level | 100, 10 or 1, 0 or 5 % | no |
| hdd, ssd | `temperature/0` | Temperature | Temperature | 41, 30 °C | no |
| all disks | `load/30`, `data/31`, `data/32`, `load/51…53`, `throughput/54`, `throughput/55` | Used/Free/Total Space, Read/Write/Total Activity, Read/Write Rate | Load, Data, Throughput | — | no |

**Fake values:** none of the unelevated ones remain (package 25.8 W, Tctl 48 °C, VID 0.44–0.54 V and varying). Residual oddities: `System Fan #2` = 0 RPM is a real reading of an empty header (LHM cannot tell it apart); DIMM low limits and 3DS timings are 0 because they do not apply; every storage and CPU clock sensor has `Min = 0` because LHM creates it with `Value = 0` before the first update (ignore LHM `Min`/`Max`, our history computes its own); the 6 `control/*` are always null. The HDD answered SMART, so it was spinning during the run.

### 9.2 Trimmed vs untrimmed (elevated) — verdict: full trim OK

Same 11 hardware ids, same 206 sensor ids, same 6 null sensors (the fan controls), no sensor null in one build and not in the other; only live values differ (loads, disk rates). `Open()` 4.51 vs 4.57 s, `Close()` 15 vs 20 ms, identical update timings. The trimmed build uses 3–4 MB less private memory and about 12 MB less working set, and costs about 5.8 MiB LZMA instead of about 24 MiB. The library trim warnings (§4) are on paths not taken on Windows or in disabled modules. Residual risk: the Controller (MSI Coreliquid `BytesToStruct<T : struct>`) and PSU modules could not be exercised (no devices here); smoke-test the trimmed service on a machine with a USB controller if one becomes available.

### 9.3 What elevation adds (user → admin)

+7 hardware (`/lpc/it8689e/0`, 2 DIMMs, 4 disks) and +138 sensors (4 → 11 hardware, 68 → 206 sensors). The CPU gains `temperature/3` (CCD1) and `clock/0` (Bus Speed), and all its MSR/SMU values become real. Null sensors 16 → 6 (only the fan controls). Nothing present unelevated disappears.

### 9.4 Disk identity

Core side read unprivileged exactly as `storage_identity.rs` does (`identity_from_descriptor`, L76-90: `sha256(vendor \0 model \0 serial)` with vendor/model/serial trimmed, from `IOCTL_STORAGE_QUERY_PROPERTY`/`StorageDeviceProperty` on `\\.\PhysicalDriveN` opened with access 0 through `storage_ioctl.rs` `PhysicalDrive::open`):

| N | LHM id | LHM `Model` | LHM `SerialNumber` (IDENTIFY) | LHM `Firmware` | LHM `DeviceID` (PnP) | Descriptor product | Descriptor serial (raw) | Core serial-tier id |
|---|---|---|---|---|---|---|---|---|
| 0 | `/hdd/0` | ST2000DM008-2FR102 | `<SERIAL-HDD>` | 0001 | `SCSI\DISK&VEN_&PROD_ST2000DM008-2FR1\<INSTANCE-HDD>` | ST2000DM008-2FR102 | `"            <SERIAL-HDD>"` | `storage/device-<hash-hdd>` |
| 1 | `/ssd/1` | Corsair Force LS SSD | `<SERIAL-SSD>` | S9FM01.8 | `SCSI\DISK&VEN_CORSAIR&PROD_FORCE_LS_SSD\<INSTANCE-SSD>` | Corsair Force LS SSD | `<SERIAL-SSD>` | `storage/device-<hash-ssd>` |
| 2 | `/nvme/2` | Fanxiang S880 2TB | `<IDENTIFY-SERIAL-NVME-1>` | SN11273 | `SCSI\DISK&VEN_NVME&PROD_FANXIANG_S880_2T\<INSTANCE-NVME-1>` | Fanxiang S880 2TB | `<EUI64-NVME-1>.` | `storage/device-<hash-nvme-1>` |
| 3 | `/nvme/3` | SHPP41-2000GM | `<IDENTIFY-SERIAL-NVME-2>` | 51060A20 | `SCSI\DISK&VEN_NVME&PROD_SHPP41-2000GM\<INSTANCE-NVME-2>` | SHPP41-2000GM | `<EUI64-NVME-2>.` | `storage/device-<hash-nvme-2>` |

`DriveNumber` = N for all four; LHM's model equals the descriptor product exactly; the SATA serials match after trimming; **both NVMe serials differ** (IDENTIFY serial vs EUI-64-style descriptor serial). The descriptor vendor is empty on all four (the core hashes `""`).

**Recommendation.** For each storage hardware the service sends the hint `{ physicalDrive: N, descriptorModel, descriptorSerial }`, where the two strings are read by the service itself from `\\.\PhysicalDriveN` (access 0, `StorageDeviceProperty`, same trimming as the core; the serial as raw bytes if not UTF-8), not taken from DiskInfoToolkit. The Rust `svc` provider binds to the storage device that the core assigned to PhysicalDrive N (the storage provider already has the index → id map) **only if** the core's own descriptor for N has the same model and serial. This guards against renumbering between the two reads, and works whichever tier (serial, GPT, MBR, PnP) the core chose, because the check is on the drive, not on the tier. On a mismatch or an unknown N the disk becomes a device of its own, `storage/lhm-<sha256(model \0 IDENTIFY serial)>` (stable; LHM's `/nvme/N` identifier is not). DiskInfoToolkit's IDENTIFY serial and firmware can go out as device properties (for NVMe they are the real serials). A new `Schema` is due on `HardwareAdded/Removed`, since hot-plug renumbers N.

### 9.5 PawnIO handles and close sequence

The name-based scan found **0** handles named `*PawnIO*` at every step, even right after an elevated `Open()`, so it cannot answer by name: `NtQueryObject(ObjectNameInformation)` evidently does not return a PawnIO name (the same query returns `\Device\Null` and `\Device\Harddisk0\DR0` unprivileged; the PawnIO device cannot be opened unprivileged to investigate further). The count of *File* handles, however, moves exactly as the source predicts (§6d):

| Step | File handles |
|---|---|
| after `Open()` | 58 |
| after 12 cycles | 58 |
| after `Close()` | 55 (−3: AmdFamily17, RyzenSMU, IT8689E `LpcIO`) |
| after GC | 55 |
| after `DriverManager.UnloadDriver()` | 53 (−2: the two PIIX4 SMBus modules) |
| unelevated baseline (no PawnIO) | 51 |

Two extra file handles remain that cannot be attributed (not necessarily PawnIO). Recommended close sequence: `computer.Close()` → `RAMSPDToolkit.Windows.Driver.DriverManager.UnloadDriver()` → `GC.Collect(); GC.WaitForPendingFinalizers(); GC.Collect();`. **Fixed in Task 15:** this order crashes the stop (the forced GC runs `~SPDAccessor` after the unload nulled its PawnIO module; the measurement above ran GC *before* the unload, which is why it did not show); the service only calls `Close()` and exits. Since the residue cannot be proven to be zero, and `Close()` returns no memory either (§9.6), the clean option is for the service to **exit** when the last client leaves (the "2 minutes without clients" stop of spec §2.2 already exits with code 0). ~~using the close sequence only as a best effort for short gaps between clients~~ — **superseded (Task 15, 9c9eea8):** LHM stays open for the whole life of the service, not just for "short gaps between clients"; it is only disposed (`Close()`, no forced GC, no `UnloadDriver()`) when the service itself exits. As a result, the SMBus and PawnIO handles this section measured are released at process exit, not before — relevant when checking with `handle64.exe`. Manual check for the user in M4: Sysinternals `handle64.exe -p <service pid>` (elevated) after the last client has left.

### 9.6 Timings, memory, CPU budget (elevated)

| Hardware | `Update()` median / max (ms), untrimmed | trimmed |
|---|---|---|
| `/amdcpu/0` | 0.90 / 3.0 | 1.06 / 31.5 |
| `/lpc/it8689e/0` | 0.60 / 0.74 | 0.63 / 0.71 |
| `/memory/dimm/1`, `/memory/dimm/3` | 1.20 / 1.21 each | 1.20 / 1.22 each |
| `/ram`, `/vram`, `/motherboard` | < 0.01 | < 0.01 |
| `/nvme/2` | 7.5 / 122.7 | 8.1 / 11.4 |
| `/nvme/3` | 3.4 / 3.6 | 3.3 / 3.6 |
| `/ssd/1` (SATA) | **137.3** / 137.6 | 137.5 / 137.6 |
| `/hdd/0` (SATA) | **253.5** / 262.5 | 256.9 / 261.1 |
| whole cycle | ≈ 405 ms (max 516) | ≈ 410 ms |

- `Open()` takes **4.4–4.6 s** (events: CPU and motherboard ready at 0.36 s, DIMMs at 2.7 s after SMBus/SPD detection, disks at 4.5 s); `Close()` 5–20 ms. The first `Schema` therefore arrives about 4.5 s after the first subscription: open LHM on the worker thread and never block the pipe; optionally enable Storage only after the first `Schema` (the `IsStorageEnabled` setter adds the group to an already open `Computer`).
- The two SATA disks cost about 390 ms per update, on every call (SMART over ATA pass-through; the thread is blocked on the device, not computing). Updating storage every 1 s is not acceptable; at 30 s it is about 13 ms/s of blocked time. Storage needs its own schedule, ideally its own thread, so it never delays the 1 s snapshot.
- **CPU estimate** (not measured directly: the dump has no process CPU time): the CPU-bound work per 1 s tick is CPU + Super I/O + 2 DIMMs ≈ 0.9 + 0.6 + 2.4 ≈ 4 ms (the Super I/O and SMBus reads are busy-waits inside PawnIO) → about 0.4 % of one logical processor, about **0.03 % of the 16-thread machine**. Even counting the whole storage pass as CPU at 30 s (+13 ms/s) the total stays around 0.1 % of the machine: **within the < 1 % budget**. The real CPU time of the service must be measured in M4 (`scripts/measure-footprint.ps1`).
- **Memory:** 63–67 MB private and 89–101 MB working set right after `Open()`; 66–79 MB private during the cycles (the managed heap swings between 7 and 43 MB depending on when the GC runs: `Open()` leaves about 40 MB of garbage, probably DiskInfoToolkit's PCI/USB id tables and SMART setup); 78–84 MB private after `Close()` and GC. Trimmed is 3–4 MB lower. **Borderline against < 80 MB private.** Plan a compacting, decommitting GC right after `Open()` (`GC.Collect(2, GCCollectionMode.Aggressive, blocking: true, compacting: true)`), the workstation non-concurrent GC (`<ConcurrentGarbageCollection>false</ConcurrentGarbageCollection>`), `ValuesTimeWindow = TimeSpan.Zero` (§3), and re-measure in M4. `Close()` frees no memory, one more reason to exit the process instead (§9.5).
- Threads 8 → 33 after `Open()`, unchanged after `Close()`. Handles 222 → 432.

### 9.7 Proposed canonical mapping

Rules: LHM sensor names are hard-coded English strings in the library, so the service matches on `(hardware type, SensorType, Name pattern)`, never on LHM indices (they shift with the core count). **DUP** = the core already has this exact id: the service emits the core's id and the cross-provider merge drops it (core first). Sensors not in the table keep a stable fallback name `lhm-<sensortype>-<index>` under their kind and LHM's text as label (spec §5.3). Units are converted to ours (LHM "GB" = 2^30 bytes). Label keys are `Label.key` (the UI prefixes `sensor.`); the existing ones are `cpu.load.total`, `cpu.load.thread`, `cpu.clock.effective`, `memory.load`, `memory.used`, `memory.total`, `storage.read`, `storage.write`, `storage.active`, `storage.temperature`, `storage.temperatureSensor`. Category = kind unless stated. Source `lhm` (new `Source::Lhm`).

CPU (device `cpu/0`, hint `GenericCpu.Index`; core ids from `crates/oma-win/src/cpu.rs` L134-160):

| LHM | Our id | Label key (arg) | Kind / unit | Status |
|---|---|---|---|---|
| Load "CPU Total" | `cpu/0/load/total` | `cpu.load.total` | load / percent | DUP |
| Load "CPU Core #k" (index t+2, with or without "Thread #j") | `cpu/0/load/thread-0-<t>` (single processor group only; otherwise fallback) | `cpu.load.thread` (t) | load / percent | DUP |
| Load "CPU Core Max" | `cpu/0/load/core-max` | `cpu.load.coreMax` | load / percent | new |
| Temperature "Core (Tctl/Tdie)" | `cpu/0/temperature/tctl` | `cpu.temperature.tctl` | temperature / celsius | new |
| Temperature "CCD<k> (Tdie)" | `cpu/0/temperature/ccd-<k>` | `cpu.temperature.ccd` (k) | temperature / celsius | new |
| Temperature "CPU Package", "Core #k" (Intel) | `cpu/0/temperature/package`, `cpu/0/temperature/core-<k>` | `cpu.temperature.package`, `cpu.temperature.core` (k) | temperature / celsius | new (not on this PC) |
| Power "Package" | `cpu/0/power/package` | `cpu.power.package` | power / watt | new |
| Power "Core #n (SMU)" | `cpu/0/power/core-<n>` | `cpu.power.core` (n) | power / watt | new |
| Power / Voltage "SoC" (other Zen parts) | `cpu/0/power/soc`, `cpu/0/voltage/soc` | `cpu.power.soc`, `cpu.voltage.soc` | power / watt, voltage / volt | new (not on this PC) |
| Voltage "Core #n VID" | `cpu/0/voltage/core-<n>-vid` | `cpu.voltage.coreVid` (n) | voltage / volt | new |
| Clock "Bus Speed" | `cpu/0/clock/bus` | `cpu.clock.bus` | clock / megahertz | new |
| Clock "Cores (Average)" | `cpu/0/clock/average` | `cpu.clock.average` | clock / megahertz | new |
| Clock "Cores (Average Effective)" | `cpu/0/clock/average-effective` | `cpu.clock.averageEffective` | clock / megahertz | new — **not** `clock/effective`: LHM's effective clock counts sleep time (120 MHz at idle), the core's `effective` is PDH performance × base clock; different quantities |
| Clock "Core #n" | `cpu/0/clock/core-<n>` | `cpu.clock.core` (n) | clock / megahertz | new |
| Clock "Core #n (Effective)" | `cpu/0/clock/core-<n>-effective` | `cpu.clock.coreEffective` (n) | clock / megahertz | new |
| Factor "Core #n" (multiplier) | — | — | — | drop (= clock / bus) |

Memory (device `memory/0`; core ids from `memory.rs` L45-68; DIMMs become sensors of that single device, spec §5.3):

| LHM | Our id | Label key (arg) | Kind / unit | Status |
|---|---|---|---|---|
| `/ram` Load "Memory" | `memory/0/load/used` | `memory.load` | load / percent | DUP |
| `/ram` Data "Memory Used" | `memory/0/data/used` | `memory.used` | data / bytes (×2^30) | DUP |
| `/ram` Data "Memory Available", all of `/vram` | — | — | — | drop |
| DIMM Temperature "DIMM #i" | `memory/0/temperature/dimm-<i>` | `memory.temperature.dimm` (i) | temperature / celsius | new |
| DIMM High and Critical High limit, Capacity, timings | device properties (`dimm<i>TempHighC`, `dimm<i>TempCriticalC`, `dimm<i>CapacityBytes`, timings) | `property.…` | — | properties (constants) |
| DIMM Resolution, Low and Critical Low limits, 3DS timings (0) | — | — | — | drop |

Storage (device = the core's `storage/…` via §9.4, fallback `storage/lhm-…`; core ids from `storage.rs` L199-260 and `storage_temperature.rs` L93-108):

| LHM | Our id | Label key (arg) | Kind / unit | Status |
|---|---|---|---|---|
| Temperature "Temperature" / "Composite Temperature" | `…/temperature/drive` | `storage.temperature` | temperature / celsius | DUP where the core reads it (HDD, NVMe); **fills a gap for the Corsair SSD** (its driver answers `ERROR_INVALID_FUNCTION`, the core has no sensor) |
| Temperature "Temperature #n" | `…/temperature/sensor-<n>` | `storage.temperatureSensor` (n) | temperature / celsius | DUP — verified: driver Index n = NVMe TSn (core 47/41/48 vs LHM 47/40.9/50.9 °C, read seconds apart) |
| Temperature "Warning" / "Critical Temperature" | — | — | — | drop: core properties `tempWarningC`/`tempCriticalC` (LHM is 1 °C lower: 85/86 vs driver 86/87, 89/94 vs 90/95) |
| Load "Total Activity" | `…/load/active` | `storage.active` | load / percent | DUP |
| Throughput "Read Rate" / "Write Rate" | `…/throughput/read`, `…/throughput/write` | `storage.read`, `storage.write` | throughput / bytes_per_second | DUP |
| Load "Read/Write Activity", "Used Space"; Data "Free/Total Space" | — | — | — | drop |
| Level "Life" | `…/percent/life` | `storage.life` | percent / percent | new |
| Level "Available Spare", "Percentage Used" | `…/percent/available-spare`, `…/percent/wear` | `storage.availableSpare`, `storage.percentUsed` | percent / percent | new |
| Level "Available Spare Threshold" | property `availableSpareThresholdPct` | `property.availableSpareThresholdPct` | — | property |
| Data "Data Read" / "Data Written" | `…/data/host-read`, `…/data/host-written` | `storage.hostRead`, `storage.hostWritten` | data / bytes (×2^30) | new |
| Factor "Power On Hours", "Power On Count" | `…/<kind>/power-on-hours`, `…/<kind>/power-cycles` | `storage.powerOnHours`, `storage.powerCycles` | **decision needed:** the model has no hours/count unit — add `Unit::Hours` and `Unit::Count` (with a kind such as `counter`), or publish them as device properties refreshed with SMART | new |
| other SMART attribute sensors (per model) | fallback `lhm-<type>-<index>` | LHM text | by type | new |

Super I/O `/lpc/<chip>/<n>` → a device of its own, `motherboard/lhm-<sha256("/lpc/it8689e/0")>`, kind `motherboard`, name "ITE IT8689E" (the bare `/motherboard` hardware has no sensors and is not published). Its labels are board-specific texts from LHM's per-board tables, shown as is:

| LHM | Our id | Label | Kind / unit | Status |
|---|---|---|---|---|
| Temperature `temperature/<i>` | `…/temperature/lhm-<i>` | LHM text ("System", "PCH", "CPU", "PCIe x16", "VRM MOS", "VSoC MOS") | temperature / celsius | new |
| Voltage `voltage/<i>` | `…/voltage/lhm-<i>` | LHM text ("Vcore", "+12V", "CMOS Battery"…) | voltage / volt | new |
| Fan `fan/<i>` | `…/fan/lhm-<i>` | LHM text ("CPU Fan", "System Fan #1"…) | fan / rpm | new (an empty header reads 0) |
| Control `control/<i>` | — | — | — | drop (write path; always null) |

Controllers and PSUs (none on this PC) follow the fallback rule, on devices `fan_controller/lhm-<hash>` and `psu/lhm-<hash>` (hash of the HID-based LHM identifier, which contains VID/PID/serial and is stable).
