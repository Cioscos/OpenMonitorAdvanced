using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// <see cref="DriveDescriptor.Parse"/> against hand-built <c>STORAGE_DEVICE_DESCRIPTOR</c>
/// buffers (winioctl.h: <c>ProductIdOffset</c> at 16, <c>SerialNumberOffset</c> at 24, both
/// little-endian <c>u32</c>). <see cref="DriveDescriptor.Read"/> needs a real disk and is not
/// unit-tested here.
/// </summary>
public sealed class DriveDescriptorTests
{
    /// <summary>
    /// Builds a minimal descriptor buffer: a 36-byte header (only the two offset fields
    /// matter) followed by the NUL-terminated model text and then the NUL-terminated (or raw)
    /// serial text, at the offsets the header points to.
    /// </summary>
    private static byte[] BuildDescriptor(byte[]? modelBytes, byte[]? serialBytes, bool terminateModel = true, bool terminateSerial = true)
    {
        const int headerLength = 36;
        int modelOffset = modelBytes is null ? 0 : headerLength;
        int modelLength = modelBytes?.Length + (terminateModel ? 1 : 0) ?? 0;
        int serialOffset = serialBytes is null ? 0 : headerLength + modelLength;
        int serialLength = serialBytes?.Length + (terminateSerial ? 1 : 0) ?? 0;

        var buffer = new byte[headerLength + modelLength + serialLength];
        BitConverter.GetBytes(modelOffset).CopyTo(buffer, 16);
        BitConverter.GetBytes(serialOffset).CopyTo(buffer, 24);

        if (modelBytes is not null)
        {
            modelBytes.CopyTo(buffer, modelOffset);
        }

        if (serialBytes is not null)
        {
            serialBytes.CopyTo(buffer, serialOffset);
        }

        return buffer;
    }

    [Fact]
    public void ParsesModelAndSerialAndTrims()
    {
        byte[] descriptor = BuildDescriptor(" ST2000DM008-2FR102 "u8.ToArray(), "  SERIAL123  "u8.ToArray());

        (string? model, string? serial) = DriveDescriptor.Parse(descriptor);

        Assert.Equal("ST2000DM008-2FR102", model);
        Assert.Equal("SERIAL123", serial);
    }

    [Fact]
    public void ZeroOffsetMeansMissing()
    {
        byte[] descriptor = BuildDescriptor(null, "SERIAL"u8.ToArray());

        (string? model, string? serial) = DriveDescriptor.Parse(descriptor);

        Assert.Null(model);
        Assert.Equal("SERIAL", serial);
    }

    [Fact]
    public void NonUtf8SerialIsNull()
    {
        byte[] descriptor = BuildDescriptor("MODEL"u8.ToArray(), [0xFF, 0xFE, 0x00]);

        (string? model, string? serial) = DriveDescriptor.Parse(descriptor);

        Assert.Equal("MODEL", model);
        Assert.Null(serial);
    }

    [Fact]
    public void OffsetOutsideTheBufferIsNull()
    {
        byte[] descriptor = BuildDescriptor("MODEL"u8.ToArray(), null);
        BitConverter.GetBytes(descriptor.Length + 100).CopyTo(descriptor, 16);

        (string? model, string? serial) = DriveDescriptor.Parse(descriptor);

        Assert.Null(model);
        Assert.Null(serial);
    }

    [Fact]
    public void EmptyAfterTrimIsNull()
    {
        byte[] descriptor = BuildDescriptor("   "u8.ToArray(), "SERIAL"u8.ToArray());

        (string? model, string? serial) = DriveDescriptor.Parse(descriptor);

        Assert.Null(model);
        Assert.Equal("SERIAL", serial);
    }

    [Fact]
    public void MissingTerminatorIsNull()
    {
        byte[] descriptor = BuildDescriptor("MODEL"u8.ToArray(), null, terminateModel: false);

        (string? model, string? _) = DriveDescriptor.Parse(descriptor);

        Assert.Null(model);
    }

    [Fact]
    public void NegativeDriveNumberReadsNothing()
    {
        (string? model, string? serial) = DriveDescriptor.Read(-1);

        Assert.Null(model);
        Assert.Null(serial);
    }
}
