using System.Runtime.InteropServices;
using OpenMonitorAdvanced.Service.Pipe;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Pipe;

public sealed class PipeNativeTests
{
    [Fact]
    public void SecurityAttributesIsTwentyFourBytes()
    {
        // DWORD (4) + 4 padding + pointer (8) + BOOL (4) + 4 tail padding on x64 = 24.
        Assert.Equal(24, Marshal.SizeOf<PipeNative.SECURITY_ATTRIBUTES>());
    }

    [Fact]
    public void OnlyTheFirstInstanceAsksForFirstPipeInstance()
    {
        // PIPE_ACCESS_DUPLEX (0x3) | FILE_FLAG_OVERLAPPED (0x40000000), + FILE_FLAG_FIRST_PIPE_INSTANCE (0x80000).
        Assert.Equal(0x40080003u, PipeNative.OpenMode(first: true));
        Assert.Equal(0x40000003u, PipeNative.OpenMode(first: false));
    }

    [Fact]
    public void PipeModeIsByteModeAndRejectsRemoteClients()
    {
        // PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT are all 0; PIPE_REJECT_REMOTE_CLIENTS = 0x8.
        Assert.Equal(0x8u, PipeNative.PipeMode);
    }
}
