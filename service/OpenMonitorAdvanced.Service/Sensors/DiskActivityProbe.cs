using System.Runtime.InteropServices;
using Microsoft.Extensions.Logging;
using Microsoft.Win32.SafeHandles;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// The passive sources of design M6b §4.4, each on <c>\\.\PhysicalDriveN</c> opened with access 0:
/// <c>GetDevicePowerState</c> (whether Windows turned the disk off) and
/// <c>IOCTL_DISK_PERFORMANCE</c> (the driver's read and write counters). Measured harmless: asked
/// of a disk Windows turned off they answer in milliseconds and leave it off, and Windows still
/// turns off a disk that is asked every few seconds. The calls need a real disk (verified live);
/// the rule over their answers (<see cref="DiskActivity"/>) and the struct layout are unit-tested.
/// </summary>
public sealed class DiskActivityProbe(ILogger<DiskActivityProbe> log) : IDiskActivityProbe
{
    private const string OpenMetadata = "open (access 0)";
    private const string QueryPerformance = "IOCTL_DISK_PERFORMANCE";
    private const string QueryPowerState = "GetDevicePowerState";

    private readonly DiskPowerProbe.Win32ErrorLog _errors = new(log);

    /// <inheritdoc />
    public DiskCounters? Read(int driveNumber)
    {
        using SafeFileHandle? handle = Open(driveNumber);
        return handle is null ? null : ReadCounters(handle, driveNumber);
    }

    /// <inheritdoc />
    public DiskSample Sample(int driveNumber, bool withCounters)
    {
        using SafeFileHandle? handle = Open(driveNumber);
        return handle is null
            ? default
            : new DiskSample(ReadPowerState(handle, driveNumber), withCounters ? ReadCounters(handle, driveNumber) : null);
    }

    private DiskCounters? ReadCounters(SafeFileHandle handle, int driveNumber)
    {
        uint size = (uint)Marshal.SizeOf<NativeMethods.DiskPerformance>();

        // SAFETY: `handle` is valid; no input buffer; the reply is marshalled out as a native
        // DISK_PERFORMANCE of `size` bytes (88, asserted in DiskActivityTests).
        bool ok = NativeMethods.DeviceIoControl(
            handle,
            NativeMethods.IoctlDiskPerformance,
            IntPtr.Zero,
            0,
            out NativeMethods.DiskPerformance performance,
            size,
            out uint returned,
            IntPtr.Zero);
        if (!ok)
        {
            _errors.Failed(driveNumber, QueryPerformance, Marshal.GetLastPInvokeError());
            return null;
        }

        if (returned < size)
        {
            _errors.EmptyReply(driveNumber, QueryPerformance);
            return null;
        }

        _errors.Succeeded(driveNumber, QueryPerformance);
        return new DiskCounters(performance.ReadCount, performance.WriteCount);
    }

    private bool? ReadPowerState(SafeFileHandle handle, int driveNumber)
    {
        // SAFETY: `handle` is valid; the BOOL is written to a local.
        if (!NativeMethods.GetDevicePowerState(handle, out bool on))
        {
            _errors.Failed(driveNumber, QueryPowerState, Marshal.GetLastPInvokeError());
            return null;
        }

        _errors.Succeeded(driveNumber, QueryPowerState);
        return on;
    }

    /// <summary>The drive opened for metadata; <see langword="null"/> when it is not there or the open fails (logged).</summary>
    private SafeFileHandle? Open(int drive)
    {
        if (drive < 0)
        {
            return null;
        }

        // SAFETY: access 0 opens the device for metadata queries only; no media I/O.
        SafeFileHandle handle = DiskPowerProbe.NativeMethods.CreateFileW(
            DiskPowerProbe.DrivePath(drive),
            0,
            DiskPowerProbe.NativeMethods.FileShareReadWrite,
            IntPtr.Zero,
            DiskPowerProbe.NativeMethods.OpenExisting,
            0,
            IntPtr.Zero);
        if (!handle.IsInvalid)
        {
            _errors.Succeeded(drive, OpenMetadata);
            return handle;
        }

        int error = Marshal.GetLastPInvokeError();
        handle.Dispose();
        if (error is not (DiskPowerProbe.NativeMethods.ErrorFileNotFound or DiskPowerProbe.NativeMethods.ErrorPathNotFound))
        {
            _errors.Failed(drive, OpenMetadata, error);
        }

        return null;
    }

    internal static class NativeMethods
    {
        /// <c>CTL_CODE(IOCTL_DISK_BASE, 0x0008, METHOD_BUFFERED, FILE_ANY_ACCESS)</c> (winioctl.h).
        internal const uint IoctlDiskPerformance = 0x00070020;

        /// <summary><c>DISK_PERFORMANCE</c> (winioctl.h): 88 bytes.</summary>
        [StructLayout(LayoutKind.Sequential)]
        internal struct DiskPerformance
        {
            public long BytesRead;
            public long BytesWritten;
            public long ReadTime;
            public long WriteTime;
            public long IdleTime;
            public uint ReadCount;
            public uint WriteCount;
            public uint QueueDepth;
            public uint SplitCount;
            public long QueryTime;
            public uint StorageDeviceNumber;

            [MarshalAs(UnmanagedType.ByValArray, SizeConst = 8)]
            public ushort[] StorageManagerName;
        }

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool DeviceIoControl(
            SafeFileHandle hDevice,
            uint dwIoControlCode,
            IntPtr lpInBuffer,
            uint nInBufferSize,
            out DiskPerformance lpOutBuffer,
            uint nOutBufferSize,
            out uint lpBytesReturned,
            IntPtr lpOverlapped);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        internal static extern bool GetDevicePowerState(SafeFileHandle hDevice, [MarshalAs(UnmanagedType.Bool)] out bool pfOn);
    }
}

/// <summary>What the passive sources say about a drive at the start of a storage round.</summary>
/// <param name="PoweredOff">Windows reports the disk off. A failed call counts as on, unless Windows last reported the disk off: then it is still off.</param>
/// <param name="Readable">Its counters could be read and belong to a drive with a <see cref="DriveKey"/>.</param>
/// <param name="Recent">Its counters grew since the baseline taken shortly before the round, or Windows turned it on since the round before.</param>
/// <param name="First">It is watched for the first time; said once, whatever happens to the round.</param>
internal readonly record struct DriveActivity(bool PoweredOff, bool Readable, bool Recent, bool First);

/// <summary>The rule of recent activity (design M6b §4.4). Pure.</summary>
internal static class DiskActivity
{
    /// <summary>How long before a round the baseline is taken.</summary>
    internal static readonly TimeSpan Window = TimeSpan.FromSeconds(10);

    /// <summary>
    /// How much later than <see cref="Window"/> the round's sample may still come: the round is
    /// started by a timer and lists the drives first, so it is never exactly on time.
    /// </summary>
    internal static readonly TimeSpan Tolerance = TimeSpan.FromSeconds(2);

    /// <summary>True only when both samples exist, no counter went back and at least one grew.</summary>
    internal static bool Between(DiskCounters? earlier, DiskCounters? later) =>
        earlier is { } before
        && later is { } after
        && after.ReadCount >= before.ReadCount
        && after.WriteCount >= before.WriteCount
        && (after.ReadCount > before.ReadCount || after.WriteCount > before.WriteCount);

    /// <summary>
    /// A round's check of a drive whose power mode matters and whose SMART is on, in this order:
    /// Windows reports it off, so it is in standby and nothing is sent; recent activity, so it is
    /// asked (it is working: resetting its idle timer changes nothing); otherwise idle, and
    /// nothing is sent, since the question alone would keep Windows from ever turning it off.
    /// A disk that is on is also asked the one time it is <see cref="DriveActivity.First"/>
    /// watched: a spinning but quiet disk would otherwise never show a value.
    /// </summary>
    internal static DriveCheck Check(DriveFacts drive, DriveActivity seen, Func<DriveFacts, bool?> isSpunDown)
    {
        var unasked = new DriveCheck(drive, Asked: false, SpunDown: null);
        if (seen.PoweredOff)
        {
            return unasked with { PoweredOff = true };
        }

        return seen.Recent || seen.First ? unasked with { Asked = true, SpunDown = isSpunDown(drive) } : unasked with { Idle = true };
    }
}

/// <summary>
/// The storage worker's samples of the passive sources (one thread, no lock).
/// <para>
/// <b>Baseline.</b> The counters are read <see cref="DiskActivity.Window"/> before a round and
/// compared with a second sample at the round's start, before any command is sent. A baseline
/// serves one round, and only for the drive it was read from: a drive with a
/// <see cref="DriveKey"/>, in a drive list that has not changed since the round before (a new
/// identity at its number, or a hot-plug, which renumbers drives), and no more than the window
/// plus <see cref="DiskActivity.Tolerance"/> old (a suspension, a late round). Anything else is
/// no growth.
/// </para>
/// <para>
/// <b>Memory of the watched drives</b>, by drive number, model and serial. A drive that was not
/// watched in the sample before is <see cref="DriveActivity.First"/>: after <see cref="Clear"/>
/// (storage switched off, the hub idle) every drive is, and so is one that is newly listed,
/// has a new identity, or whose SMART was just switched on. The memory is replaced before
/// <see cref="Sample"/> returns, that is before any question is sent, so a round that fails
/// afterwards cannot make a drive first again.
/// </para>
/// <para>
/// <b>Memory of the drives Windows reported off</b>, which outlives <see cref="Clear"/> for as
/// long as the drive is listed. Only a real "on" ends it: a failed call after an "off" leaves
/// the drive off (nothing may be sent to it), and the first real "on" after an "off" counts as
/// recent activity, since Windows powers a disk up for I/O.
/// </para>
/// </summary>
internal sealed class ActivityWatch(IDiskActivityProbe probe, TimeProvider time)
{
    private readonly Dictionary<int, DiskCounters?> _baseline = [];
    private readonly HashSet<(int Drive, string? Model, string? Serial)> _off = [];
    private HashSet<(int Drive, string? Model, string? Serial)> _met = [];
    private IReadOnlyList<DriveFacts> _listed = [];
    private int[] _watched = [];
    private long _takenAt;
    private bool _taken;

    /// <summary>Whether the next round has drives to watch and no baseline yet.</summary>
    internal bool BaselineDue => !_taken && _watched.Length > 0;

    /// <summary>Reads the counters of the drives the round before watched.</summary>
    internal void TakeBaseline()
    {
        _baseline.Clear();
        foreach (int drive in _watched)
        {
            _baseline[drive] = probe.Read(drive);
        }

        _takenAt = time.GetTimestamp();
        _taken = true;
    }

    /// <summary>
    /// The round's sample of every drive <paramref name="watched"/> selects, by drive number.
    /// The counters of a drive without a <see cref="DriveKey"/> are not read: nothing proves
    /// whose they are. Consumes the baseline and each drive's "first" at once.
    /// </summary>
    internal IReadOnlyDictionary<int, DriveActivity> Sample(IReadOnlyList<DriveFacts> drives, Func<DriveFacts, bool> watched)
    {
        bool usable = _taken
            && time.GetElapsedTime(_takenAt) <= DiskActivity.Window + DiskActivity.Tolerance
            && drives.SequenceEqual(_listed);
        var seen = new Dictionary<int, DriveActivity>();
        var met = new HashSet<(int, string?, string?)>();
        var next = new List<int>();
        foreach (DriveFacts drive in drives)
        {
            if (!watched(drive))
            {
                continue;
            }

            bool keyed = DriveKey.Compute(drive.Model, drive.Serial) is not null;
            DiskSample now = probe.Sample(drive.DriveNumber, withCounters: keyed);
            (int, string?, string?) identity = (drive.DriveNumber, drive.Model, drive.Serial);
            bool wasOff = _off.Contains(identity);
            bool off = now.PoweredOn is { } on ? !on : wasOff; // an unknown state after "off" is still off
            bool first = !_met.Contains(identity);
            bool grew = usable && _baseline.TryGetValue(drive.DriveNumber, out DiskCounters? earlier) && DiskActivity.Between(earlier, now.Counters);
            seen[drive.DriveNumber] = new DriveActivity(off, now.Counters is not null, grew || (wasOff && !off), first);
            met.Add(identity);
            if (off)
            {
                _off.Add(identity);
            }
            else
            {
                _off.Remove(identity);
            }

            if (keyed)
            {
                next.Add(drive.DriveNumber);
            }
        }

        _met = met;
        _off.RemoveWhere(identity => !drives.Any(d => (d.DriveNumber, d.Model, d.Serial) == identity)); // gone, or another disk now
        _listed = drives;
        _watched = [.. next];
        _baseline.Clear();
        _taken = false;
        return seen;
    }

    /// <summary>
    /// Storage was switched off or the hub went idle: nothing is watched, no baseline is read,
    /// and every drive is new afterwards. What Windows last said about a drive being off stays.
    /// </summary>
    internal void Clear()
    {
        _met = [];
        _listed = [];
        _watched = [];
        _baseline.Clear();
        _taken = false;
    }
}
