using Microsoft.Extensions.Time.Testing;
using OpenMonitorAdvanced.Service.Frames;
using OpenMonitorAdvanced.Service.Protocol;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Frames;

public sealed class FrameRequestsTests
{
    private static readonly FramesConfigureMessage On = new(Enabled: true, TrackPcLatency: false, TrackGpu: false);

    private readonly FakeTimeProvider _time = new();

    [Fact]
    public void NoSessionMeansOff()
    {
        var requests = new FrameRequests(_time);

        Assert.Null(requests.Effective);
        Assert.Empty(requests.Targets);
        Assert.Null(requests.NextExpiry);

        requests.Configure(1, On with { Enabled = false, TrackGpu = true });
        Assert.Null(requests.Effective);
    }

    [Fact]
    public void OptionsAreOredAcrossSessions()
    {
        var requests = new FrameRequests(_time);

        requests.Configure(1, On with { TrackPcLatency = true });
        Assert.Equal(new FramesOptions(TrackPcLatency: true, TrackGpu: false), requests.Effective);

        requests.Configure(2, On with { TrackGpu = true });
        Assert.Equal(new FramesOptions(TrackPcLatency: true, TrackGpu: true), requests.Effective);

        // A disabled session asks for nothing, whatever its flags.
        requests.Configure(1, new FramesConfigureMessage(Enabled: false, TrackPcLatency: true, TrackGpu: true));
        Assert.Equal(new FramesOptions(TrackPcLatency: false, TrackGpu: true), requests.Effective);
    }

    [Fact]
    public void DisconnectKeepsTheRequestForThirtySeconds()
    {
        var requests = new FrameRequests(_time);
        requests.Configure(1, On with { TrackGpu = true });
        requests.Target(1, 4242);

        requests.Disconnected(1);
        Assert.Equal(TimeSpan.FromSeconds(30), requests.NextExpiry);

        _time.Advance(TimeSpan.FromSeconds(29));
        Assert.Equal(new FramesOptions(TrackPcLatency: false, TrackGpu: true), requests.Effective);
        Assert.Equal([4242u], requests.Targets);
        Assert.Equal(TimeSpan.FromSeconds(1), requests.NextExpiry);

        _time.Advance(TimeSpan.FromSeconds(1));
        Assert.Null(requests.Effective);
        Assert.Empty(requests.Targets);
        Assert.Null(requests.NextExpiry);
    }

    [Fact]
    public void ReconnectWithinGraceKeepsCaptureRunning()
    {
        var requests = new FrameRequests(_time);
        requests.Configure(1, On);
        requests.Disconnected(1);

        _time.Advance(TimeSpan.FromSeconds(20));
        requests.Configure(2, On);
        Assert.Equal(new FramesOptions(false, false), requests.Effective);

        _time.Advance(TimeSpan.FromSeconds(20)); // session 1 has expired, session 2 holds the capture
        Assert.Equal(new FramesOptions(false, false), requests.Effective);
        Assert.Null(requests.NextExpiry);
    }

    [Theory]
    [InlineData(0u)]
    [InlineData(4u)]
    public void PidZeroAndFourAreIgnored(uint pid)
    {
        var requests = new FrameRequests(_time);
        requests.Configure(1, On);
        requests.Target(1, 1234);

        requests.Target(1, pid);

        Assert.Empty(requests.Targets);
        Assert.Null(requests.TargetOf(1));
        Assert.False(FrameRequests.IsValidPid(pid));
    }

    [Fact]
    public void TargetsAreTheDistinctValidPids()
    {
        var requests = new FrameRequests(_time);
        requests.Configure(1, On);
        requests.Configure(2, On);
        requests.Configure(3, On);
        requests.Target(1, 100);
        requests.Target(2, 100);
        requests.Target(3, 200);
        requests.Target(4, null);

        Assert.Equal([100u, 200u], requests.Targets.Order());
        Assert.Equal(100u, requests.TargetOf(2));

        requests.Target(3, null);
        Assert.Equal([100u], requests.Targets);
    }
}
