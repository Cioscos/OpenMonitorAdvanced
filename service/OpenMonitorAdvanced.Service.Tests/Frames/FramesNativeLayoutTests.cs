using System.Runtime.InteropServices;
using OpenMonitorAdvanced.Service.Frames;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Frames;

/// <summary>Marshalled sizes of the FFI structs in <see cref="FramesNative"/> (x64 values).</summary>
public sealed class FramesNativeLayoutTests
{
    [Fact]
    public void EventTracePropertiesIsOneHundredTwentyBytes() =>
        Assert.Equal(120, Marshal.SizeOf<FramesNative.EventTraceProperties>());

    [Fact]
    public void JobExtendedLimitInformationIsOneHundredFortyFourBytes() =>
        Assert.Equal(144, Marshal.SizeOf<FramesNative.JobObjectExtendedLimitInformation>());
}
