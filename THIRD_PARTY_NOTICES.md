# Third-party notices

OpenMonitor Advanced is licensed under GPL-3.0-or-later (see `LICENSE`). It does not
redistribute any GPU vendor library: `nvml.dll`, `nvapi64.dll`, `atiadlxx.dll` and
`ControlLib.dll` are loaded at run time, only from `System32`, from the installed graphics
driver. This file lists the third-party material the source code relies on.

## NVIDIA NVAPI

`crates/oma-win/src/gpu/nvapi.rs` uses function ids, structure layouts and status values
from the NVAPI SDK headers (https://github.com/NVIDIA/nvapi), which are distributed under
the MIT license:

    SPDX-FileCopyrightText: Copyright (c) 2019-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
    SPDX-License-Identifier: MIT

    Permission is hereby granted, free of charge, to any person obtaining a
    copy of this software and associated documentation files (the "Software"),
    to deal in the Software without restriction, including without limitation
    the rights to use, copy, modify, merge, publish, distribute, sublicense,
    and/or sell copies of the Software, and to permit persons to whom the
    Software is furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in
    all copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
    THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
    FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
    DEALINGS IN THE SOFTWARE.

## LibreHardwareMonitor (credit)

Two NVAPI functions used here are not part of the public SDK:
`NvAPI_GPU_ThermalGetSensors` (id `0x65FE3AAD`) and `NvAPI_GPU_ClientVoltRailsGetStatus`
(id `0x465F9BCF`). Their ids, structure layouts and sensor indices are interoperability
facts first documented by LibreHardwareMonitor
(https://github.com/LibreHardwareMonitor/LibreHardwareMonitor, MPL-2.0), credited here
with thanks. No LibreHardwareMonitor code is included. Readings from these calls are marked
"experimental" in the app.

## NVIDIA NVML, AMD ADL, Intel IGCL

`crates/oma-win/src/gpu/nvml.rs`, `adl.rs` and `igcl.rs` contain hand-written
interoperability declarations (exported symbol names, structure layouts and constant
values), written from the vendors' public API documentation and checked against the
installed libraries. No NVIDIA, AMD or Intel header file, or text from one, is included
in this repository or downloaded at build time. AMD ADLX is not used: its license is not
compatible with free software licenses.

## oma-service (bundled with the "Sensori avanzati" installer component)

`oma-service` is a self-contained .NET publish. It links the following NuGet
packages unmodified; none of their source is copied into this repository.

- **LibreHardwareMonitorLib 0.9.6** (MPL-2.0) —
  https://github.com/LibreHardwareMonitor/LibreHardwareMonitor. `service/OpenMonitorAdvanced.Service/Sensors/`
  reads its `Hardware`/`Sensor` tree through the public API only; no
  LibreHardwareMonitor source is included here. MPL-2.0 source for the exact
  published version stays available upstream at the tag/commit above.
- **DiskInfoToolkit 1.1.2** (MPL-2.0) — https://github.com/Blacktempel/DiskInfoToolkit,
  a LibreHardwareMonitor dependency (disk SMART/NVMe access).
- **RAMSPDToolkit-NDD 1.4.2** (MPL-2.0) — https://github.com/Blacktempel/RAMSPDToolkit,
  a LibreHardwareMonitor dependency (RAM SPD over PawnIO SMBus).
- **BlackSharp.Core 1.0.7** (MPL-2.0) — https://github.com/Blacktempel/BlackSharp,
  a shared dependency of DiskInfoToolkit and RAMSPDToolkit-NDD.
- **HidSharp 2.6.4** (Apache-2.0) — a LibreHardwareMonitor dependency (HID
  enumeration for fan/RGB controllers), copyright 2010-2025 James F. Bellinger.
- **System.Management 10.0.2** (MIT, part of the .NET runtime) — WMI access used
  by LibreHardwareMonitorLib on a small number of code paths (for example
  `Ipmi.IsBmcPresent()`).
- **MessagePack-CSharp 3.1.10** (MIT) — https://github.com/MessagePack-CSharp/MessagePack-CSharp,
  the .NET side of the wire protocol codec (`service/OpenMonitorAdvanced.Service/Protocol/`);
  the Rust side (`crates/oma-ipc`) hand-writes the same framing without this
  library (§6 of the design spec).
- **.NET 10 runtime** (MIT) — the self-contained publish embeds the .NET 10
  runtime and its base class libraries.

On the Rust side, `crates/oma-ipc` depends on `rmp` 0.8.15 and `rmp-serde` 1.3.1
(both MIT, https://github.com/3Hren/msgpack-rust) for MessagePack decoding on
the client.

## PawnIO

The "Sensori avanzati" installer component redistributes the official,
Microsoft-signed `PawnIO_setup.exe` 2.2.0 unmodified
(https://github.com/namazso/PawnIO.Setup/releases/tag/2.2.0), with its
SHA-256 pinned in `scripts/build-installer-payload.ps1` and
`app/src-tauri/nsis/pawnio.sha256`; the setup file itself is never committed
to this repository, only downloaded (or read from a cached, verified copy) at
build time. PawnIO's kernel driver is GPL-2.0 with an exception for programs
that only use its IOCTL interface (https://github.com/namazso/PawnIO); its
user-mode library and modules are LGPL-2.1. No PawnIO source is included
here, and this project ships no PawnIO module of its own (§9 of the design
spec). Follow-up before the 1.0 release: ask the PawnIO author to confirm
this redistribution (`docs/follow-ups.md`).
