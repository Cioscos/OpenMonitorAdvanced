using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Time.Testing;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests;

public sealed class IdleShutdownTests
{
    private static readonly TimeSpan IdleAfter = TimeSpan.FromMinutes(2);

    private readonly FakeTimeProvider _time = new();
    private readonly FakeLifetime _lifetime = new();

    private IdleShutdown NewIdle() => new(_lifetime, _time, IdleAfter);

    [Fact]
    public void StopsAfterTwoMinutesWithoutClients()
    {
        var idle = NewIdle();
        idle.Start();

        _time.Advance(IdleAfter - TimeSpan.FromSeconds(1));
        Assert.Equal(0, _lifetime.StopCount);

        _time.Advance(TimeSpan.FromSeconds(1));
        Assert.Equal(1, _lifetime.StopCount);

        // Nothing re-arms after the stop request.
        _time.Advance(TimeSpan.FromMinutes(10));
        Assert.Equal(1, _lifetime.StopCount);
    }

    [Fact]
    public void DoesNotCountDownBeforeStart()
    {
        _ = NewIdle();

        _time.Advance(TimeSpan.FromMinutes(10));

        Assert.Equal(0, _lifetime.StopCount);
    }

    [Fact]
    public void AClientCancelsTheCountdown()
    {
        var idle = NewIdle();
        idle.Start();

        _time.Advance(TimeSpan.FromSeconds(119));
        idle.ClientConnected();
        _time.Advance(TimeSpan.FromMinutes(10));

        Assert.Equal(0, _lifetime.StopCount);
        Assert.Equal(1, idle.ClientCount);
    }

    [Fact]
    public void CountdownRestartsWhenTheLastClientLeaves()
    {
        var idle = NewIdle();
        idle.Start();
        idle.ClientConnected();
        idle.ClientConnected();

        idle.ClientDisconnected();
        _time.Advance(TimeSpan.FromMinutes(3));
        Assert.Equal(0, _lifetime.StopCount);

        idle.ClientDisconnected();
        Assert.Equal(0, idle.ClientCount);
        _time.Advance(IdleAfter - TimeSpan.FromSeconds(1));
        Assert.Equal(0, _lifetime.StopCount);

        _time.Advance(TimeSpan.FromSeconds(1));
        Assert.Equal(1, _lifetime.StopCount);
    }

    [Fact]
    public void AClientArrivingAfterALeaveRestartsTheFullCountdown()
    {
        var idle = NewIdle();
        idle.Start();
        idle.ClientConnected();
        idle.ClientDisconnected();
        _time.Advance(TimeSpan.FromSeconds(100));

        idle.ClientConnected();
        idle.ClientDisconnected();
        _time.Advance(TimeSpan.FromSeconds(100));
        Assert.Equal(0, _lifetime.StopCount);

        _time.Advance(TimeSpan.FromSeconds(20));
        Assert.Equal(1, _lifetime.StopCount);
    }

    [Fact]
    public void MoreDisconnectsThanConnectsIsABug()
    {
        var idle = NewIdle();
        idle.Start();

        Assert.Throws<InvalidOperationException>(idle.ClientDisconnected);
    }
}

/// <summary>Records <see cref="StopApplication"/> calls instead of stopping anything.</summary>
internal sealed class FakeLifetime : IHostApplicationLifetime
{
    private int _stopCount;

    public int StopCount => Volatile.Read(ref _stopCount);

    public CancellationToken ApplicationStarted => CancellationToken.None;

    public CancellationToken ApplicationStopping => CancellationToken.None;

    public CancellationToken ApplicationStopped => CancellationToken.None;

    public void StopApplication() => Interlocked.Increment(ref _stopCount);
}
