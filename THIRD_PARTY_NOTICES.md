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
