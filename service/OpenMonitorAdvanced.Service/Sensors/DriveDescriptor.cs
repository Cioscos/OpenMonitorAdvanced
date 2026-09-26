using System.Buffers.Binary;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// Reads and parses a Windows <c>STORAGE_DEVICE_DESCRIPTOR</c> (winioctl.h) off
/// <c>\\.\PhysicalDriveN</c>, opened with access 0 so this never wakes a disk and never
/// needs administrator rights — the same read the core does, and the same trimming rules
/// it applies (<c>crates/oma-win/src/storage_identity.rs</c>, <c>descriptor_text</c> /
/// <c>descriptor_serial</c>).
/// </summary>
public static class DriveDescriptor
{
    private const int ProductIdOffsetField = 16;
    private const int SerialNumberOffsetField = 24;
    private const int BusTypeField = 28;

    /// <summary>
    /// Parses the model (<c>ProductIdOffset</c> field) and serial
    /// (<c>SerialNumberOffset</c> field) out of a raw <c>STORAGE_DEVICE_DESCRIPTOR</c>
    /// buffer. An offset field that is missing, 0, or points outside the buffer means the
    /// field is absent (<see langword="null"/>). The text itself is ASCII, NUL-terminated,
    /// trimmed of leading/trailing whitespace; a missing terminator, non-UTF-8 bytes, or an
    /// empty result after trimming are all <see langword="null"/>.
    /// </summary>
    public static (string? Model, string? Serial) Parse(ReadOnlySpan<byte> storageDeviceDescriptor) =>
        (ReadField(storageDeviceDescriptor, ProductIdOffsetField), ReadField(storageDeviceDescriptor, SerialNumberOffsetField));

    private static string? ReadField(ReadOnlySpan<byte> bytes, int offsetField)
    {
        if (offsetField + sizeof(uint) > bytes.Length)
        {
            return null;
        }

        uint offset = BinaryPrimitives.ReadUInt32LittleEndian(bytes.Slice(offsetField, sizeof(uint)));
        if (offset == 0 || offset >= (uint)bytes.Length)
        {
            return null;
        }

        ReadOnlySpan<byte> tail = bytes[(int)offset..];
        int terminator = tail.IndexOf((byte)0);
        if (terminator < 0)
        {
            return null;
        }

        ReadOnlySpan<byte> raw = tail[..terminator];
        string text;
        try
        {
            var strict = new UTF8Encoding(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true);
            text = strict.GetString(raw);
        }
        catch (DecoderFallbackException)
        {
            return null;
        }

        text = text.Trim();
        return text.Length == 0 ? null : text;
    }

    /// <summary>
    /// Opens <c>\\.\PhysicalDrive&lt;driveNumber&gt;</c> with access 0 (metadata only,
    /// never reads or writes disk data, never wakes the disk) and issues
    /// <c>IOCTL_STORAGE_QUERY_PROPERTY</c> for <c>StorageDeviceProperty</c>. A negative
    /// drive number, a missing drive, or a failed query all yield <c>(null, null)</c>. Not
    /// unit-tested: this needs a real disk; see <see cref="Parse"/> for the tested logic.
    /// </summary>
    public static (string? Model, string? Serial) Read(int driveNumber)
    {
        if (driveNumber < 0)
        {
            return (null, null);
        }

        // SAFETY: access 0 (no GENERIC_READ/GENERIC_WRITE) opens the device handle for
        // metadata queries only; the OS never touches the disk's platters for this.
        using SafeFileHandle handle = NativeMethods.CreateFileW(
            $@"\\.\PhysicalDrive{driveNumber.ToString(System.Globalization.CultureInfo.InvariantCulture)}",
            0,
            NativeMethods.FileShareReadWrite,
            IntPtr.Zero,
            NativeMethods.OpenExisting,
            NativeMethods.FileAttributeNormal,
            IntPtr.Zero);

        if (handle.IsInvalid)
        {
            return (null, null);
        }

        byte[]? descriptor = Query(handle, out _);
        return descriptor is null ? (null, null) : Parse(descriptor);
    }

    /// <summary>
    /// <c>STORAGE_DEVICE_DESCRIPTOR.BusType</c> (<c>STORAGE_BUS_TYPE</c>, a little-endian u32 at
    /// offset 28), or <see langword="null"/> when the buffer is too short.
    /// </summary>
    public static uint? ParseBusType(ReadOnlySpan<byte> storageDeviceDescriptor) =>
        storageDeviceDescriptor.Length >= BusTypeField + sizeof(uint)
            ? BinaryPrimitives.ReadUInt32LittleEndian(storageDeviceDescriptor.Slice(BusTypeField, sizeof(uint)))
            : null;

    /// <summary>
    /// <c>IOCTL_STORAGE_QUERY_PROPERTY</c> / <c>StorageDeviceProperty</c> on an already open
    /// handle (access 0 is enough): the raw descriptor, or <see langword="null"/> with the Win32
    /// error in <paramref name="win32Error"/> (0 for an empty reply).
    /// </summary>
    internal static byte[]? Query(SafeFileHandle handle, out int win32Error)
    {
        byte[] query = new byte[12]; // STORAGE_PROPERTY_QUERY: PropertyId=StorageDeviceProperty(0), QueryType=PropertyStandardQuery(0), both zero already.
        byte[] output = new byte[8192];

        // SAFETY: `handle` is open and valid; `query`/`output` are pinned by the runtime
        // marshaller for the duration of the call and sized as declared below.
        bool ok = NativeMethods.DeviceIoControl(
            handle,
            NativeMethods.IoctlStorageQueryProperty,
            query,
            (uint)query.Length,
            output,
            (uint)output.Length,
            out uint returned,
            IntPtr.Zero);

        if (!ok)
        {
            win32Error = Marshal.GetLastPInvokeError();
            return null;
        }

        win32Error = 0;
        return returned == 0 ? null : output[..(int)returned];
    }

    private static class NativeMethods
    {
        internal const uint FileShareReadWrite = 0x00000001 | 0x00000002;
        internal const uint OpenExisting = 3;
        internal const uint FileAttributeNormal = 0x80;

        /// <c>CTL_CODE(IOCTL_STORAGE_BASE, 0x0500, METHOD_BUFFERED, FILE_ANY_ACCESS)</c> (winioctl.h).
        internal const uint IoctlStorageQueryProperty = 0x2D1400;

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
            byte[] lpInBuffer,
            uint nInBufferSize,
            byte[] lpOutBuffer,
            uint nOutBufferSize,
            out uint lpBytesReturned,
            IntPtr lpOverlapped);
    }
}
