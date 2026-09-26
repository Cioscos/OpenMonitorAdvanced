using System.Collections.Concurrent;
using System.Globalization;
using System.Runtime.InteropServices;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Win32.SafeHandles;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// Disk power checks for decision D6, done without LHM/DiskInfoToolkit so they can run before
/// the storage group exists:
/// <list type="bullet">
/// <item>rotational = <c>IOCTL_STORAGE_QUERY_PROPERTY</c> / <c>StorageDeviceSeekPenaltyProperty</c>
/// on <c>\\.\PhysicalDriveN</c> opened with access 0 (metadata only); unknown counts as rotational;</item>
/// <item>standby = ATA <c>CHECK POWER MODE</c> (0xE5) through <c>IOCTL_ATA_PASS_THROUGH</c>, a
/// non-media command that never spins a drive up (needs read/write access: the service runs as
/// LocalSystem).</item>
/// </list>
/// The IOCTLs need an elevated process and a real disk (verified in Task 15); the decision
/// logic, the register interpretation and the struct layouts are unit-tested.
/// </summary>
public sealed class DiskPowerProbe : IDiskPowerProbe
{
    /// <summary>Same probe range as DiskInfoToolkit's fallback enumeration (<c>\\.\PhysicalDrive0…63</c>).</summary>
    private const int MaxProbedDrives = 64;

    private const byte AtaCheckPowerMode = 0xE5;
    private const byte AtaStatusError = 0x01;
    private const uint AtaTimeoutSeconds = 5;

    private readonly Func<IEnumerable<int>> _enumerateDrives;
    private readonly Func<int, bool?> _hasSeekPenalty;
    private readonly Func<int, bool?> _isSpunDown;
    private readonly ILogger _log;

    /// <summary>Last Win32 error logged per (drive, operation): a failure repeated every 30 s is logged once, until it changes.</summary>
    private readonly ConcurrentDictionary<(int Drive, string Operation), int> _lastLoggedError = new();

    public DiskPowerProbe(ILogger<DiskPowerProbe> log)
    {
        _log = log;
        _enumerateDrives = EnumeratePhysicalDrives;
        _hasSeekPenalty = QuerySeekPenalty;
        _isSpunDown = QueryCheckPowerMode;
    }

    /// <summary>Test seam: the decision logic over scripted drive answers.</summary>
    internal DiskPowerProbe(Func<IEnumerable<int>> enumerateDrives, Func<int, bool?> hasSeekPenalty, Func<int, bool?> isSpunDown, ILogger? log = null)
    {
        _enumerateDrives = enumerateDrives;
        _hasSeekPenalty = hasSeekPenalty;
        _isSpunDown = isSpunDown;
        _log = log ?? NullLogger.Instance;
    }

    /// <inheritdoc />
    public bool? IsSpunDown(int driveNumber) => driveNumber < 0 ? null : _isSpunDown(driveNumber);

    /// <summary>
    /// <c>StorageDeviceSeekPenaltyProperty</c> on <c>\\.\PhysicalDriveN</c> (access 0):
    /// <see langword="true"/> rotational, <see langword="false"/> solid state,
    /// <see langword="null"/> unknown (callers treat unknown as rotational).
    /// </summary>
    public bool? HasSeekPenalty(int driveNumber) => driveNumber < 0 ? null : _hasSeekPenalty(driveNumber);

    /// <inheritdoc />
    public bool AllRotationalDisksActive()
    {
        foreach (int drive in _enumerateDrives())
        {
            if (HasSeekPenalty(drive) == false)
            {
                continue; // solid state: identification cannot wake it
            }

            bool? spunDown = IsSpunDown(drive);
            if (spunDown != false)
            {
                _log.LogDebug(
                    "PhysicalDrive{Drive} is rotational (or unknown) and {State}: storage stays disabled",
                    drive,
                    spunDown == true ? "in standby" : "of unknown power state");
                return false;
            }
        }

        return true;
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

    /// <summary><c>DEVICE_SEEK_PENALTY_DESCRIPTOR.IncursSeekPenalty</c> (offset 8), or <see langword="null"/> when the reply is too short.</summary>
    internal static bool? ParseSeekPenalty(ReadOnlySpan<byte> descriptor) =>
        descriptor.Length > 8 ? descriptor[8] != 0 : null;

    private static string DrivePath(int drive) => @"\\.\PhysicalDrive" + drive.ToString(CultureInfo.InvariantCulture);

    private IEnumerable<int> EnumeratePhysicalDrives()
    {
        var drives = new List<int>();
        for (int drive = 0; drive < MaxProbedDrives; drive++)
        {
            // SAFETY: access 0 opens the device for metadata queries only; no media I/O.
            using SafeFileHandle handle = NativeMethods.CreateFileW(DrivePath(drive), 0, NativeMethods.FileShareReadWrite, IntPtr.Zero, NativeMethods.OpenExisting, 0, IntPtr.Zero);
            if (!handle.IsInvalid)
            {
                drives.Add(drive);
                continue;
            }

            int error = Marshal.GetLastPInvokeError();
            if (error is not (NativeMethods.ErrorFileNotFound or NativeMethods.ErrorPathNotFound))
            {
                LogWin32Error(drive, "open (access 0)", error);
            }
        }

        return drives;
    }

    private bool? QuerySeekPenalty(int drive)
    {
        // SAFETY: access 0 opens the device for metadata queries only; no media I/O.
        using SafeFileHandle handle = NativeMethods.CreateFileW(DrivePath(drive), 0, NativeMethods.FileShareReadWrite, IntPtr.Zero, NativeMethods.OpenExisting, 0, IntPtr.Zero);
        if (handle.IsInvalid)
        {
            LogWin32Error(drive, "open (access 0)", Marshal.GetLastPInvokeError());
            return null;
        }

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
            LogWin32Error(drive, "IOCTL_STORAGE_QUERY_PROPERTY(SeekPenalty)", Marshal.GetLastPInvokeError());
            return null;
        }

        ReadOnlySpan<byte> bytes = MemoryMarshal.AsBytes(MemoryMarshal.CreateReadOnlySpan(ref descriptor, 1));
        return ParseSeekPenalty(bytes[..(int)Math.Min(returned, (uint)bytes.Length)]);
    }

    private bool? QueryCheckPowerMode(int drive)
    {
        // SAFETY: read/write access is what IOCTL_ATA_PASS_THROUGH requires; opening the handle
        // issues no media I/O, and CHECK POWER MODE is answered without spinning up.
        using SafeFileHandle handle = NativeMethods.CreateFileW(
            DrivePath(drive),
            NativeMethods.GenericRead | NativeMethods.GenericWrite,
            NativeMethods.FileShareReadWrite,
            IntPtr.Zero,
            NativeMethods.OpenExisting,
            0,
            IntPtr.Zero);
        if (handle.IsInvalid)
        {
            LogWin32Error(drive, "open (read/write)", Marshal.GetLastPInvokeError());
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
            LogWin32Error(drive, "IOCTL_ATA_PASS_THROUGH(CHECK POWER MODE)", Marshal.GetLastPInvokeError());
            return null;
        }

        _lastLoggedError.TryRemove((drive, "IOCTL_ATA_PASS_THROUGH(CHECK POWER MODE)"), out _);
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

    private void LogWin32Error(int drive, string operation, int error)
    {
        if (_lastLoggedError.TryGetValue((drive, operation), out int last) && last == error)
        {
            return;
        }

        _lastLoggedError[(drive, operation)] = error;
        _log.LogWarning("PhysicalDrive{Drive}: {Operation} failed with Win32 error {Error}", drive, operation, error);
    }

    internal static class NativeMethods
    {
        internal const uint GenericRead = 0x80000000;
        internal const uint GenericWrite = 0x40000000;
        internal const uint FileShareReadWrite = 0x00000001 | 0x00000002;
        internal const uint OpenExisting = 3;
        internal const int ErrorFileNotFound = 2;
        internal const int ErrorPathNotFound = 3;

        /// <c>CTL_CODE(IOCTL_STORAGE_BASE, 0x0500, METHOD_BUFFERED, FILE_ANY_ACCESS)</c> (winioctl.h).
        internal const uint IoctlStorageQueryProperty = 0x002D1400;

        /// <c>CTL_CODE(IOCTL_SCSI_BASE, 0x040b, METHOD_BUFFERED, FILE_READ_ACCESS | FILE_WRITE_ACCESS)</c> (ntddscsi.h).
        internal const uint IoctlAtaPassThrough = 0x0004D02C;

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
    }
}
