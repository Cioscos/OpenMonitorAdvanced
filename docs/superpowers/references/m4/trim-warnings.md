# Accepted trimming warnings — LibreHardwareMonitorLib 0.9.6 (`oma-service`, .NET 10, `TrimMode=full`)

Source: `<scratchpad>\m4\lhm-dump\trim-warnings.txt` (20 lines, `TrimmerSingleWarn=false`,
full list from `dotnet publish` of the S1 spike's `LhmDump` console app) and
[`s1-lhm.md`](s1-lhm.md) §4 and §9.2. See that report for the elevated verdict:
**same 11 hardware ids, same 206 sensor ids, same 6 null sensors, trimmed vs
untrimmed — full trim is safe to ship.**

Any new trim warning introduced by a change to `oma-service` or an
LHM/DiskInfoToolkit/RAMSPDToolkit/BlackSharp version bump **must fail the
build** (treat trim warnings as errors in the service's publish profile,
`TrimmerSingleWarn=false` for full visibility in CI logs) unless it is
reviewed and added to this table with the same reasoning applied here: same
sensor set with and without trimming, only on a Windows-only /
disabled-module / non-generic-instantiation code path.

## Accepted (library code, applies to `oma-service`)

| Code | Origin (assembly.type.member) | Motivation |
|---|---|---|
| IL2026 | `LibreHardwareMonitor.Hardware.OpCode.Open()` — `Assembly.GetType("Mono.Unix.Native.Syscall")` | Unix-only branch (`OpCode.cs:210-285`), never reached on Windows. Confirmed by the elevated diff: identical sensor set trimmed vs untrimmed. |
| IL2026 | `LibreHardwareMonitor.Hardware.OpCode.Close()` — same `Assembly.GetType` call | Same as above, `Close()` side. |
| IL2075 | `LibreHardwareMonitor.Hardware.OpCode.Open()` — `Type.GetField`/`GetMethod` on the reflectively-loaded Unix type | Unreachable on Windows: the `Assembly.GetType` above always returns `null` first. |
| IL2075 | `LibreHardwareMonitor.Hardware.OpCode.Close()` — `Type.GetMethod` on the reflectively-loaded Unix type | Same as above. |
| IL2075 | `LibreHardwareMonitor.Hardware.OpCode.Open()` — second `Type.GetMethod` call on the same Unix-only path | Same as above; the trimmer emits one warning per reflective call site, and `OpCode.Open`/`Close` each make more than one. |
| IL2067 | `System.Management.MTAHelper.CreateInMTA(Type)` | Only exercised when a WMI call is made from an STA thread. LHM's only WMI use reachable from the modules we enable is `Ipmi.IsBmcPresent()` (`LpcIO.cs:30`, `Ipmi.cs:269-281`), called from `Computer.Open()` on the caller's thread. `oma-service` must keep that thread MTA (the CLR default for a non-UI thread), which this spike ran under, so the annotation gap is never hit at runtime. |
| IL2077 | `System.Management.MTAHelper.WorkerThread()` | Same MTAHelper reasoning as IL2067 above; same code path, keep the worker thread MTA. |
| IL2075 | `HidSharp.Platform.Linux.NativeMethods.<>c__DisplayClass10_0.<uname>b__0(String)` | Linux-only P/Invoke path inside HidSharp's platform detection; never taken on Windows (HidSharp is used here only for the Controller/PSU HID enumeration, which is unaffected). |
| IL2091 | `LibreHardwareMonitor.Hardware.Controller.MSI.MsiCoreLiquidController.BytesToStruct<T>(Byte[])` | `T : struct` generic used for sequential blittable structs (`MsiCoreLiquidController.cs:325`); no constructor is required so trimming cannot remove anything this call needs. **Not exercised on this machine** (no MSI AIO controller attached) — smoke-test the trimmed service if one becomes available (§9.2 residual risk in `s1-lhm.md`). |
| IL2091 | `LibreHardwareMonitor.Hardware.PowerMonitor.WireViewPro2.BytesToStructure<T>(Byte[])` | Same generic-struct pattern as above, in the disabled PowerMonitor module (`oma-service` does not enable `Computer.IsPowerMonitorEnabled` per spec §5.3's LHM group selection). |

That covers the 5 distinct library warning shapes from `s1-lhm.md` §4
(`IL2026 ×2, IL2075 ×3` on `OpCode`; `IL2067, IL2077` on `MTAHelper`; `IL2075`
on HidSharp `uname`; `IL2091` on `MsiCoreLiquidController`; `IL2091` on
`WireViewPro2`) — 10 of the 20 raw lines in `trim-warnings.txt` (some warning
shapes are printed more than once per call site by the trimmer).

## Not applicable to `oma-service` (the spike's own `Program` code)

`s1-lhm.md` §4 notes explicitly: "the 6 in `Program` are the spike's own
reflection/JsonNode code" — `LhmDump`'s own diagnostic dump routine, not part
of LibreHardwareMonitorLib or any dependency `oma-service` will ship. These
do not apply to `oma-service`, which does not reflect over arbitrary
`IHardware`/`ISensor` instances or serialize through `System.Text.Json.Nodes`:

| Code | Origin (spike-only) | Why it does not apply |
|---|---|---|
| IL2026 | `Program.Main(String[])` | `LhmDump`'s own JSON-dump entry point (`JsonArray.Add<T>` on non-primitive types). `oma-service` uses the hand-written MessagePack encoding from [`s2-msgpack.md`](s2-msgpack.md), not `System.Text.Json.Nodes`. |
| IL2026 | `Program.ThreadNames()` | Same spike-only JSON dump helper. |
| IL2026 | `Program.UpdateAll(Computer, Dictionary<String,List<Double>>, JsonArray)` | Same spike-only JSON dump helper. |
| IL2075 | `Program.ReflectMembers(Object, Int32)` — `Type.GetFields(BindingFlags)` | `LhmDump`'s generic reflection dump of `DiskInfoToolkit.Storage`/`StorageDevice` members (§1 of `s1-lhm.md`), built only to answer the S1 spike's own questions about the third-party storage wrapper. `oma-service` binds to the specific public members named in `s1-lhm.md` §6a (`DriveNumber`, `SerialNumber`, `Model`, …), not generic reflection. |
| IL2075 | `Program.ReflectMembers(Object, Int32)` — `Type.GetProperties(BindingFlags)` | Same as above. |
| IL2075 | `Program.StorageIdentity(IHardware)` — `Type.GetProperty(String, BindingFlags)` | Same spike-only reflective storage inspection, superseded in the real service by the direct `StorageDevice.Storage` property access documented in `s1-lhm.md` §6a. |

The remaining 4 raw lines in `trim-warnings.txt` beyond the 10 (library) + 6
(spike `Program`) above are duplicate/truncated re-emissions of the same
`Program.*` reflection and `JsonArray.Add<T>` warning shapes already listed
in the "not applicable" table (the .NET trimmer sometimes prints a warning
once with the full member signature and once in a shortened, unattributed
form for the same call site). They carry no additional library-code warning
and are not separately actionable.
