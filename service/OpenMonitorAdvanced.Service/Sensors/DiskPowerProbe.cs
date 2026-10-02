using System.Collections.Concurrent;
using System.Globalization;
using System.Runtime.InteropServices;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Win32.SafeHandles;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// Disk checks for decision D6, done without LHM/DiskInfoToolkit so they can run before the
/// storage group exists and never wake a disk:
/// <list type="bullet">
/// <item><see cref="Describe"/>: <c>STORAGE_DEVICE_DESCRIPTOR</c> (model, serial, bus type) and
/// <c>StorageDeviceSeekPenaltyProperty</c> through <c>IOCTL_STORAGE_QUERY_PROPERTY</c> on
/// <c>\\.\PhysicalDriveN</c> opened with access 0 (metadata only);</item>
/// <item><see cref="IsSpunDown"/>: ATA <c>CHECK POWER MODE</c> (0xE5) through
/// <c>IOCTL_ATA_PASS_THROUGH</c>, a non-media command that never spins a drive up (needs
/// read/write access: the service runs as LocalSystem). A drive whose driver rejects it (a USB
/// bridge) is asked the same command as <c>ATA PASS-THROUGH(16)</c> through
/// <c>IOCTL_SCSI_PASS_THROUGH</c>; the route that answered is remembered per drive;</item>
/// <item><see cref="CheckGate"/>: the gate of controller ruling R17 over those
/// facts (<see cref="DriveFacts.RequiresPowerCheck"/>, <see cref="CheckDrives"/>);
/// <see cref="Enumerate"/> lists the drives alone, with no power command.</item>
/// </list>
/// The IOCTLs need an elevated process and a real disk (verified in Task 15); the decision
/// logic, the register interpretation, the error logging and the struct layouts are unit-tested.
/// </summary>
public sealed class DiskPowerProbe : IDiskPowerProbe
{
    /// <summary>Same probe range as DiskInfoToolkit's fallback enumeration (<c>\\.\PhysicalDrive0…63</c>).</summary>
    private const int MaxProbedDrives = 64;

    private const byte AtaCheckPowerMode = 0xE5;
    private const byte AtaStatusError = 0x01;
    private const uint AtaTimeoutSeconds = 5;
    private const int SenseLength = 32;

    /// <summary>How long a drive that answered on neither route is left alone.</summary>
    private static readonly TimeSpan DeadRouteRetry = TimeSpan.FromMinutes(5);

    private const string OpenMetadata = "open (access 0)";
    private const string OpenReadWrite = "open (read/write)";
    private const string QueryDescriptor = "IOCTL_STORAGE_QUERY_PROPERTY(Device)";
    private const string QuerySeekPenalty = "IOCTL_STORAGE_QUERY_PROPERTY(SeekPenalty)";
    private const string CheckPowerMode = "IOCTL_ATA_PASS_THROUGH(CHECK POWER MODE)";
    private const string SatCheckPowerMode = "IOCTL_SCSI_PASS_THROUGH(CHECK POWER MODE)";

    private readonly Func<IReadOnlyList<DriveFacts>> _enumerateDrives;
    private readonly Func<int, DriveFacts?> _describe;
    private readonly Func<int, bool?> _nativeCheck;
    private readonly Func<int, bool?> _satCheck;
    private readonly TimeProvider _time;
    private readonly ILogger _log;
    private readonly Win32ErrorLog _errors;
    private readonly object _routeLock = new();
    private readonly Dictionary<int, RouteMemory> _routes = [];
    private readonly object _gateLogLock = new();
    private string? _lastBlockers;

    public DiskPowerProbe(ILogger<DiskPowerProbe> log)
    {
        _log = log;
        _errors = new Win32ErrorLog(log);
        _enumerateDrives = EnumeratePhysicalDrives;
        _describe = DescribePhysicalDrive;
        _nativeCheck = QueryCheckPowerMode;
        _satCheck = QuerySatCheckPowerMode;
        _time = TimeProvider.System;
    }

    /// <summary>Test seam: the decision logic over scripted drive facts and power-mode answers.</summary>
    internal DiskPowerProbe(Func<IReadOnlyList<DriveFacts>> enumerateDrives, Func<int, bool?> isSpunDown, ILogger? log = null)
        : this(enumerateDrives, isSpunDown, _ => null, TimeProvider.System, log)
    {
    }

    /// <summary>Test seam: as above, with the answers of the native and of the SAT route scripted apart.</summary>
    internal DiskPowerProbe(Func<IReadOnlyList<DriveFacts>> enumerateDrives, Func<int, bool?> nativeCheck, Func<int, bool?> satCheck, TimeProvider time, ILogger? log = null)
    {
        _log = log ?? NullLogger.Instance;
        _errors = new Win32ErrorLog(_log);
        _enumerateDrives = enumerateDrives;
        _describe = n => enumerateDrives().FirstOrDefault(d => d.DriveNumber == n);
        _nativeCheck = nativeCheck;
        _satCheck = satCheck;
        _time = time;
    }

    private enum PowerRoute
    {
        /// <summary>Neither route gave an answer that could be interpreted.</summary>
        None,
        Native,
        Sat,
    }

    /// <inheritdoc />
    public bool? IsSpunDown(int driveNumber, string? model, string? serial)
    {
        if (driveNumber < 0)
        {
            return null;
        }

        long now = _time.GetTimestamp();
        RouteMemory? known;
        lock (_routeLock)
        {
            if (_routes.TryGetValue(driveNumber, out known) && (known.Model != model || known.Serial != serial))
            {
                _routes.Remove(driveNumber); // another disk took this drive number
                known = null;
            }
        }

        if (known is { Route: PowerRoute.None } && _time.GetElapsedTime(known.Since, now) < DeadRouteRetry)
        {
            return null; // unknown between the retries: no earlier answer is ever reused
        }

        // The remembered route first; the other one whenever it gives no answer.
        PowerRoute route = known?.Route == PowerRoute.Sat ? PowerRoute.Sat : PowerRoute.Native;
        bool? spunDown = Ask(route, driveNumber);
        if (spunDown is null)
        {
            route = route == PowerRoute.Sat ? PowerRoute.Native : PowerRoute.Sat;
            spunDown = Ask(route, driveNumber);
        }

        if (spunDown is null)
        {
            route = PowerRoute.None;
        }

        // The route only, never the answer; a failed retry of "none" restarts its five minutes.
        // Without a model and a serial the next disk at this number could not be told apart, so
        // nothing is remembered and both routes are asked every time.
        bool identified = !string.IsNullOrEmpty(model) && !string.IsNullOrEmpty(serial);
        if (identified && (known?.Route != route || route == PowerRoute.None))
        {
            if (known?.Route != route)
            {
                _log.LogDebug("PhysicalDrive{Drive}: CHECK POWER MODE route is now {Route}", driveNumber, route);
            }

            lock (_routeLock)
            {
                _routes[driveNumber] = new RouteMemory(route, model, serial, now);
            }
        }

        return spunDown;
    }

    /// <inheritdoc />
    public DriveFacts? Describe(int driveNumber) => driveNumber < 0 ? null : _describe(driveNumber);

    /// <inheritdoc />
    public IReadOnlyList<DriveCheck> CheckGate()
    {
        IReadOnlyList<DriveCheck> checks = CheckDrives(Enumerate(), drive => IsSpunDown(drive.DriveNumber, drive.Model, drive.Serial));
        LogBlockersOnChange([.. checks.Where(c => c.Blocks)]);
        return checks;
    }

    /// <inheritdoc />
    public IReadOnlyList<DriveFacts> Enumerate()
    {
        IReadOnlyList<DriveFacts> drives = _enumerateDrives();
        ReconcileRoutes(drives);
        return drives;
    }

    /// <summary>Whether the D6 gate is open: no drive of <see cref="CheckGate"/> blocks.</summary>
    public bool AllRotationalDisksActive() => !CheckGate().Any(c => c.Blocks);

    /// <summary>
    /// Controller ruling R17, drive by drive: one whose <see cref="DriveFacts.RequiresPowerCheck"/>
    /// is false is not asked (and never blocks); every other drive is asked through
    /// <paramref name="isSpunDown"/> and blocks unless it answers <see langword="false"/>.
    /// </summary>
    internal static IReadOnlyList<DriveCheck> CheckDrives(IEnumerable<DriveFacts> drives, Func<DriveFacts, bool?> isSpunDown)
    {
        var checks = new List<DriveCheck>();
        foreach (DriveFacts drive in drives)
        {
            checks.Add(drive.RequiresPowerCheck ? new DriveCheck(drive, Asked: true, isSpunDown(drive)) : new DriveCheck(drive, Asked: false, SpunDown: null));
        }

        return checks;
    }

    /// <summary>
    /// ATA CHECK POWER MODE result, from the returned Sector Count register: 0x00/0x01 standby
    /// (<see langword="true"/>); 0x40/0x41/0x80/0x81/0x82/0x83/0xFF active or idle
    /// (<see langword="false"/>); anything else unknown (<see langword="null"/>).
    /// </summary>
    public static bool? InterpretCheckPowerMode(byte sectorCount) => sectorCount switch
    {
        0x00 or 0x01 => true,
        0x40 or 0x41 or 0x80 or 0x81 or 0x82 or 0x83 or 0xFF => false,
        _ => null,
    };

    /// <summary>An aborted command (Status ERR set) is unknown; otherwise <see cref="InterpretCheckPowerMode"/>.</summary>
    internal static bool? InterpretAtaResult(byte status, byte sectorCount) =>
        (status & AtaStatusError) != 0 ? null : InterpretCheckPowerMode(sectorCount);

    /// <summary>
    /// The answer of <c>ATA PASS-THROUGH(16)</c> CHECK POWER MODE: the registers come from the
    /// sense data whatever the SCSI status is (a SATA disk answers GOOD, a USB bridge CHECK
    /// CONDITION). Only the sense bytes within the <paramref name="returned"/> bytes of the reply
    /// and within the length the reply itself declares are read; a reply declaring no length (a
    /// driver may leave it at zero on a GOOD status) is bounded by the returned bytes alone, and
    /// the parser still follows the length the sense data states. <see langword="null"/> when
    /// they hold no registers, or when the reply places its sense data anywhere but in our buffer.
    /// </summary>
    internal static bool? InterpretSatReply(in NativeMethods.ScsiPassThroughWithSense reply, uint returned)
    {
        uint senseOffset = (uint)Marshal.SizeOf<NativeMethods.ScsiPassThrough>();
        if (reply.Spt.SenseInfoOffset != senseOffset || returned <= senseOffset || reply.Sense is not { Length: SenseLength } sense)
        {
            return null;
        }

        uint declared = reply.Spt.SenseInfoLength == 0 ? (uint)SenseLength : reply.Spt.SenseInfoLength;
        int available = (int)Math.Min(Math.Min(returned - senseOffset, declared), SenseLength);
        return SatSense.TryReadRegisters(sense.AsSpan(0, available), out byte status, out byte sectorCount)
            ? InterpretAtaResult(status, sectorCount)
            : null;
    }

    /// <summary>
    /// CHECK POWER MODE as <c>ATA PASS-THROUGH(16)</c>: protocol non-data (3 &lt;&lt; 1), CK_COND
    /// set so the registers come back in the sense data, no data transfer. The 12-byte form is
    /// rejected by SATA disks, so it is not used.
    /// </summary>
    internal static NativeMethods.ScsiPassThroughWithSense SatCheckPowerModeRequest()
    {
        byte[] cdb = new byte[16];
        cdb[0] = 0x85; // ATA PASS-THROUGH(16)
        cdb[1] = 0x06; // protocol: non-data
        cdb[2] = 0x20; // CK_COND
        cdb[14] = AtaCheckPowerMode;
        return new NativeMethods.ScsiPassThroughWithSense
        {
            Spt = new NativeMethods.ScsiPassThrough
            {
                Length = (ushort)Marshal.SizeOf<NativeMethods.ScsiPassThrough>(),
                CdbLength = (byte)cdb.Length,
                SenseInfoLength = SenseLength,
                DataIn = NativeMethods.ScsiIoctlDataUnspecified,
                TimeOutValue = AtaTimeoutSeconds,
                SenseInfoOffset = (uint)Marshal.SizeOf<NativeMethods.ScsiPassThrough>(),
                Cdb = cdb,
            },
            Sense = new byte[SenseLength],
        };
    }

    /// <summary><c>DEVICE_SEEK_PENALTY_DESCRIPTOR.IncursSeekPenalty</c> (offset 8), or <see langword="null"/> when the reply is too short.</summary>
    internal static bool? ParseSeekPenalty(ReadOnlySpan<byte> descriptor) =>
        descriptor.Length > 8 ? descriptor[8] != 0 : null;

    private static bool IsNoMedia(int error) => error is NativeMethods.ErrorNotReady or NativeMethods.ErrorNoMediaInDrive;

    private static string DrivePath(int drive) => @"\\.\PhysicalDrive" + drive.ToString(CultureInfo.InvariantCulture);

    private bool? Ask(PowerRoute route, int drive) => route == PowerRoute.Sat ? _satCheck(drive) : _nativeCheck(drive);

    /// <summary>
    /// Forgets the route of every drive that is gone or that changed model or serial: a drive
    /// number is reused by whatever is plugged in next.
    /// </summary>
    private void ReconcileRoutes(IReadOnlyList<DriveFacts> drives)
    {
        lock (_routeLock)
        {
            foreach ((int number, RouteMemory known) in _routes.ToArray())
            {
                DriveFacts? facts = drives.FirstOrDefault(d => d.DriveNumber == number);
                if (facts is null || facts.Model != known.Model || facts.Serial != known.Serial)
                {
                    _routes.Remove(number);
                }
            }
        }
    }

    private void LogBlockersOnChange(IReadOnlyList<DriveCheck> blockers)
    {
        string key = string.Join(';', blockers.Select(b => $"{b.Drive.DriveNumber}:{b.SpunDown}"));
        lock (_gateLogLock)
        {
            if (key == _lastBlockers)
            {
                return;
            }

            _lastBlockers = key;
        }

        if (blockers.Count == 0)
        {
            _log.LogInformation("No drive keeps storage disabled any more");
            return;
        }

        foreach (DriveCheck blocker in blockers)
        {
            _log.LogInformation(
                "PhysicalDrive{Drive} (bus {Bus}, model {Model}) keeps storage disabled: {State}",
                blocker.Drive.DriveNumber,
                blocker.Drive.BusType is uint bus ? "0x" + bus.ToString("X2", CultureInfo.InvariantCulture) : "unknown",
                blocker.Drive.Model ?? "unknown",
                blocker.SpunDown == true ? "in standby" : "power state unknown");
        }
    }

    private IReadOnlyList<DriveFacts> EnumeratePhysicalDrives()
    {
        var drives = new List<DriveFacts>();
        for (int drive = 0; drive < MaxProbedDrives; drive++)
        {
            if (DescribePhysicalDrive(drive) is { } facts)
            {
                drives.Add(facts);
            }
        }

        return drives;
    }

    private DriveFacts? DescribePhysicalDrive(int drive)
    {
        // SAFETY: access 0 opens the device for metadata queries only; no media I/O.
        using SafeFileHandle handle = NativeMethods.CreateFileW(DrivePath(drive), 0, NativeMethods.FileShareReadWrite, IntPtr.Zero, NativeMethods.OpenExisting, 0, IntPtr.Zero);
        if (handle.IsInvalid)
        {
            int error = Marshal.GetLastPInvokeError();
            if (error is NativeMethods.ErrorFileNotFound or NativeMethods.ErrorPathNotFound)
            {
                return null; // no such drive
            }

            if (IsNoMedia(error))
            {
                return new DriveFacts(drive, DriveAvailability.NoMedia, null, null, null, null);
            }

            _errors.Failed(drive, OpenMetadata, error);
            return new DriveFacts(drive, DriveAvailability.Unreadable, null, null, null, null);
        }

        _errors.Succeeded(drive, OpenMetadata);

        byte[]? descriptor = DriveDescriptor.Query(handle, out int descriptorError);
        if (descriptor is null && IsNoMedia(descriptorError))
        {
            return new DriveFacts(drive, DriveAvailability.NoMedia, null, null, null, null);
        }

        if (descriptor is null && descriptorError == 0)
        {
            _errors.EmptyReply(drive, QueryDescriptor);
        }
        else if (descriptor is null)
        {
            _errors.Failed(drive, QueryDescriptor, descriptorError);
        }
        else
        {
            _errors.Succeeded(drive, QueryDescriptor);
        }

        (string? model, string? serial) = descriptor is null ? (null, null) : DriveDescriptor.Parse(descriptor);
        uint? busType = descriptor is null ? null : DriveDescriptor.ParseBusType(descriptor);

        bool? seekPenalty = QuerySeekPenaltyOf(handle, drive, out int seekError);
        if (seekPenalty is null && IsNoMedia(seekError))
        {
            return new DriveFacts(drive, DriveAvailability.NoMedia, model, serial, busType, null);
        }

        return new DriveFacts(drive, DriveAvailability.Present, model, serial, busType, seekPenalty);
    }

    private bool? QuerySeekPenaltyOf(SafeFileHandle handle, int drive, out int error)
    {
        var query = new NativeMethods.StoragePropertyQuery
        {
            PropertyId = NativeMethods.StorageDeviceSeekPenaltyProperty,
            QueryType = NativeMethods.PropertyStandardQuery,
        };

        // SAFETY: `handle` is valid; both structs are blittable, passed by reference and sized
        // with Marshal.SizeOf (layouts asserted in DiskPowerProbeTests).
        bool ok = NativeMethods.DeviceIoControl(
            handle,
            NativeMethods.IoctlStorageQueryProperty,
            ref query,
            (uint)Marshal.SizeOf<NativeMethods.StoragePropertyQuery>(),
            out NativeMethods.DeviceSeekPenaltyDescriptor descriptor,
            (uint)Marshal.SizeOf<NativeMethods.DeviceSeekPenaltyDescriptor>(),
            out uint returned,
            IntPtr.Zero);
        if (!ok)
        {
            error = Marshal.GetLastPInvokeError();
            if (!IsNoMedia(error))
            {
                _errors.Failed(drive, QuerySeekPenalty, error);
            }

            return null;
        }

        error = 0;
        _errors.Succeeded(drive, QuerySeekPenalty);
        ReadOnlySpan<byte> bytes = MemoryMarshal.AsBytes(MemoryMarshal.CreateReadOnlySpan(ref descriptor, 1));
        return ParseSeekPenalty(bytes[..(int)Math.Min(returned, (uint)bytes.Length)]);
    }

    /// <summary>The drive opened for a pass-through IOCTL; invalid (and logged) when the open fails.</summary>
    private SafeFileHandle OpenForPassThrough(int drive)
    {
        // SAFETY: read/write access is what IOCTL_ATA_PASS_THROUGH and IOCTL_SCSI_PASS_THROUGH
        // require (both are FILE_READ_ACCESS | FILE_WRITE_ACCESS); opening the handle issues no
        // media I/O, and CHECK POWER MODE is answered without spinning up.
        SafeFileHandle handle = NativeMethods.CreateFileW(
            DrivePath(drive),
            NativeMethods.GenericRead | NativeMethods.GenericWrite,
            NativeMethods.FileShareReadWrite,
            IntPtr.Zero,
            NativeMethods.OpenExisting,
            0,
            IntPtr.Zero);
        if (handle.IsInvalid)
        {
            _errors.Failed(drive, OpenReadWrite, Marshal.GetLastPInvokeError());
        }
        else
        {
            _errors.Succeeded(drive, OpenReadWrite);
        }

        return handle;
    }

    private bool? QueryCheckPowerMode(int drive)
    {
        using SafeFileHandle handle = OpenForPassThrough(drive);
        if (handle.IsInvalid)
        {
            return null;
        }

        var request = new NativeMethods.AtaPassThroughEx
        {
            Length = (ushort)Marshal.SizeOf<NativeMethods.AtaPassThroughEx>(),
            AtaFlags = NativeMethods.AtaFlagsDrdyRequired,
            TimeOutValue = AtaTimeoutSeconds,
            PreviousTaskFile = new byte[8],
            CurrentTaskFile = new byte[8],
        };
        request.CurrentTaskFile[6] = AtaCheckPowerMode; // IDEREGS: Features, SectorCount, LBA low/mid/high, Device, Command, Reserved

        // SAFETY: `handle` is valid; the request is marshalled in and the reply out as separate
        // native copies of ATA_PASS_THROUGH_EX (48 bytes on x64, asserted in DiskPowerProbeTests);
        // no data buffer (DataTransferLength 0).
        bool ok = NativeMethods.DeviceIoControl(
            handle,
            NativeMethods.IoctlAtaPassThrough,
            in request,
            request.Length,
            out NativeMethods.AtaPassThroughEx reply,
            request.Length,
            out uint _,
            IntPtr.Zero);
        if (!ok)
        {
            _errors.Failed(drive, CheckPowerMode, Marshal.GetLastPInvokeError());
            return null;
        }

        _errors.Succeeded(drive, CheckPowerMode);
        byte[]? taskFile = reply.CurrentTaskFile;
        if (taskFile is not { Length: 8 })
        {
            return null;
        }

        bool? spunDown = InterpretAtaResult(status: taskFile[6], sectorCount: taskFile[1]);
        if (spunDown is null)
        {
            _log.LogDebug("PhysicalDrive{Drive}: CHECK POWER MODE answered status 0x{Status:X2}, sector count 0x{Count:X2} (unknown)", drive, taskFile[6], taskFile[1]);
        }

        return spunDown;
    }

    private bool? QuerySatCheckPowerMode(int drive)
    {
        using SafeFileHandle handle = OpenForPassThrough(drive);
        if (handle.IsInvalid)
        {
            return null;
        }

        NativeMethods.ScsiPassThroughWithSense request = SatCheckPowerModeRequest();
        uint size = (uint)Marshal.SizeOf<NativeMethods.ScsiPassThroughWithSense>();

        // SAFETY: `handle` is valid; the request is marshalled in and the reply out as separate
        // native copies of SCSI_PASS_THROUGH followed by its sense buffer (56 + 32 bytes on x64,
        // asserted in DiskPowerProbeTests); no data buffer (DataTransferLength 0).
        bool ok = NativeMethods.DeviceIoControl(
            handle,
            NativeMethods.IoctlScsiPassThrough,
            in request,
            size,
            out NativeMethods.ScsiPassThroughWithSense reply,
            size,
            out uint returned,
            IntPtr.Zero);
        if (!ok)
        {
            _errors.Failed(drive, SatCheckPowerMode, Marshal.GetLastPInvokeError());
            return null;
        }

        _errors.Succeeded(drive, SatCheckPowerMode);
        bool? spunDown = InterpretSatReply(reply, returned);
        if (spunDown is null)
        {
            _log.LogDebug("PhysicalDrive{Drive}: CHECK POWER MODE through SAT answered SCSI status 0x{Status:X2} in {Returned} bytes (unknown)", drive, reply.Spt.ScsiStatus, returned);
        }

        return spunDown;
    }

    /// <summary>The route that last answered for a drive, with the identity it was learnt for and when (monotonic).</summary>
    private sealed record RouteMemory(PowerRoute Route, string? Model, string? Serial, long Since);

    /// <summary>
    /// Logs a Win32 failure once per (drive, operation) until its error code changes; any success
    /// of that operation resets it, so a failure that comes back after a success is logged again.
    /// Every call made every 30 s would otherwise flood the log.
    /// </summary>
    internal sealed class Win32ErrorLog(ILogger log)
    {
        private const int EmptyReplyMarker = int.MinValue;

        private readonly ConcurrentDictionary<(int Drive, string Operation), int> _last = new();

        public void Failed(int drive, string operation, int error)
        {
            if (_last.TryGetValue((drive, operation), out int last) && last == error)
            {
                return;
            }

            _last[(drive, operation)] = error;
            log.LogWarning("PhysicalDrive{Drive}: {Operation} failed with Win32 error {Error}", drive, operation, error);
        }

        public void Succeeded(int drive, string operation) => _last.TryRemove((drive, operation), out _);

        /// <summary>A call that succeeded but returned no data (not a Win32 error); deduplicated like <see cref="Failed"/>.</summary>
        public void EmptyReply(int drive, string operation)
        {
            if (_last.TryGetValue((drive, operation), out int last) && last == EmptyReplyMarker)
            {
                return;
            }

            _last[(drive, operation)] = EmptyReplyMarker;
            log.LogWarning("PhysicalDrive{Drive}: {Operation} returned an empty reply", drive, operation);
        }
    }

    internal static class NativeMethods
    {
        internal const uint GenericRead = 0x80000000;
        internal const uint GenericWrite = 0x40000000;
        internal const uint FileShareReadWrite = 0x00000001 | 0x00000002;
        internal const uint OpenExisting = 3;
        internal const int ErrorFileNotFound = 2;
        internal const int ErrorPathNotFound = 3;
        internal const int ErrorNotReady = 21;
        internal const int ErrorNoMediaInDrive = 1112;

        /// <c>CTL_CODE(IOCTL_STORAGE_BASE, 0x0500, METHOD_BUFFERED, FILE_ANY_ACCESS)</c> (winioctl.h).
        internal const uint IoctlStorageQueryProperty = 0x002D1400;

        /// <c>CTL_CODE(IOCTL_SCSI_BASE, 0x040b, METHOD_BUFFERED, FILE_READ_ACCESS | FILE_WRITE_ACCESS)</c> (ntddscsi.h).
        internal const uint IoctlAtaPassThrough = 0x0004D02C;

        /// <c>CTL_CODE(IOCTL_SCSI_BASE, 0x0401, METHOD_BUFFERED, FILE_READ_ACCESS | FILE_WRITE_ACCESS)</c> (ntddscsi.h).
        internal const uint IoctlScsiPassThrough = 0x0004D004;

        /// <c>SCSI_IOCTL_DATA_UNSPECIFIED</c> (ntddscsi.h): the command transfers no data.
        internal const byte ScsiIoctlDataUnspecified = 2;

        /// <c>STORAGE_PROPERTY_ID.StorageDeviceSeekPenaltyProperty</c>.
        internal const int StorageDeviceSeekPenaltyProperty = 7;

        /// <c>STORAGE_QUERY_TYPE.PropertyStandardQuery</c>.
        internal const int PropertyStandardQuery = 0;

        /// <c>ATA_FLAGS_DRDY_REQUIRED</c> (ntddscsi.h).
        internal const ushort AtaFlagsDrdyRequired = 0x01;

        /// <summary><c>ATA_PASS_THROUGH_EX</c> (ntddscsi.h): 48 bytes on x64.</summary>
        [StructLayout(LayoutKind.Sequential)]
        internal struct AtaPassThroughEx
        {
            public ushort Length;
            public ushort AtaFlags;
            public byte PathId;
            public byte TargetId;
            public byte Lun;
            public byte ReservedAsUchar;
            public uint DataTransferLength;
            public uint TimeOutValue;
            public uint ReservedAsUlong;
            public nuint DataBufferOffset;

            [MarshalAs(UnmanagedType.ByValArray, SizeConst = 8)]
            public byte[] PreviousTaskFile;

            [MarshalAs(UnmanagedType.ByValArray, SizeConst = 8)]
            public byte[] CurrentTaskFile;
        }

        /// <summary><c>SCSI_PASS_THROUGH</c> (ntddscsi.h): 56 bytes on x64.</summary>
        [StructLayout(LayoutKind.Sequential)]
        internal struct ScsiPassThrough
        {
            public ushort Length;
            public byte ScsiStatus;
            public byte PathId;
            public byte TargetId;
            public byte Lun;
            public byte CdbLength;
            public byte SenseInfoLength;
            public byte DataIn;
            public uint DataTransferLength;
            public uint TimeOutValue;
            public nuint DataBufferOffset;
            public uint SenseInfoOffset;

            [MarshalAs(UnmanagedType.ByValArray, SizeConst = 16)]
            public byte[] Cdb;
        }

        /// <summary><see cref="ScsiPassThrough"/> followed by the sense buffer its <c>SenseInfoOffset</c> points at.</summary>
        [StructLayout(LayoutKind.Sequential)]
        internal struct ScsiPassThroughWithSense
        {
            public ScsiPassThrough Spt;

            [MarshalAs(UnmanagedType.ByValArray, SizeConst = SenseLength)]
            public byte[] Sense;
        }

        /// <summary><c>STORAGE_PROPERTY_QUERY</c> (winioctl.h): 12 bytes.</summary>
        [StructLayout(LayoutKind.Sequential)]
        internal struct StoragePropertyQuery
        {
            public int PropertyId;
            public int QueryType;
            public byte AdditionalParameters;
        }

        /// <summary><c>DEVICE_SEEK_PENALTY_DESCRIPTOR</c> (winioctl.h): 12 bytes.</summary>
        [StructLayout(LayoutKind.Sequential)]
        internal struct DeviceSeekPenaltyDescriptor
        {
            public uint Version;
            public uint Size;
            public byte IncursSeekPenalty;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        internal static extern SafeFileHandle CreateFileW(
            string lpFileName,
            uint dwDesiredAccess,
            uint dwShareMode,
            IntPtr lpSecurityAttributes,
            uint dwCreationDisposition,
            uint dwFlagsAndAttributes,
            IntPtr hTemplateFile);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool DeviceIoControl(
            SafeFileHandle hDevice,
            uint dwIoControlCode,
            ref StoragePropertyQuery lpInBuffer,
            uint nInBufferSize,
            out DeviceSeekPenaltyDescriptor lpOutBuffer,
            uint nOutBufferSize,
            out uint lpBytesReturned,
            IntPtr lpOverlapped);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool DeviceIoControl(
            SafeFileHandle hDevice,
            uint dwIoControlCode,
            in AtaPassThroughEx lpInBuffer,
            uint nInBufferSize,
            out AtaPassThroughEx lpOutBuffer,
            uint nOutBufferSize,
            out uint lpBytesReturned,
            IntPtr lpOverlapped);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool DeviceIoControl(
            SafeFileHandle hDevice,
            uint dwIoControlCode,
            in ScsiPassThroughWithSense lpInBuffer,
            uint nInBufferSize,
            out ScsiPassThroughWithSense lpOutBuffer,
            uint nOutBufferSize,
            out uint lpBytesReturned,
            IntPtr lpOverlapped);
    }
}
