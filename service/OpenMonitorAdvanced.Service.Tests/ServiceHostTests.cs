using System.Collections.Concurrent;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using OpenMonitorAdvanced.Service.Pipe;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Tests.Pipe;
using OpenMonitorAdvanced.Service.Tests.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests;

/// <summary>
/// The composed host (not under the SCM here: the console lifetime) with a <see cref="FakeFeed"/>
/// and a test pipe with the default DACL.
/// </summary>
public sealed class ServiceHostTests
{
    private static readonly TimeSpan RunTimeout = TimeSpan.FromSeconds(15);

    private readonly FakeFeed _feed = new();
    private readonly ListLoggerProvider _logs = new();

    private static CancellationToken Ct => TestContext.Current.CancellationToken;

    [Fact]
    public async Task ServesThePipeAndExitsWithZeroWhenStopped()
    {
        string pipeName = ListenerHarness.NewPipeName();
        using IHost host = Build(pipeName, TimeSpan.FromMinutes(2));
        bool disposedBeforeStopped = false;
        host.Services.GetRequiredService<IHostApplicationLifetime>().ApplicationStopped.Register(() => disposedBeforeStopped = _feed.DisposeCount > 0);
        Task<int> run = RunInBackground(host);

        using var client = await TestClient.ConnectAsync(pipeName, Ct);
        await client.ReadAsync<HelloMessage>(Ct);
        await client.SendAsync(new SubscribeMessage(1000, [], [], []), Ct);
        await PipeAssert.EventuallyAsync(() => _feed.All.Count == 1, "the subscription", Ct);

        host.Services.GetRequiredService<IHostApplicationLifetime>().StopApplication();

        Assert.Equal(0, await run.WaitAsync(RunTimeout, Ct));
        Assert.Null(await client.ReadAsync(Ct));

        // The pipe sessions ended (and unsubscribed) before the feed itself was disposed, and
        // that happened while the host was stopping, not after it reported itself stopped (the
        // container disposes it once more at the end; the hub's Dispose is idempotent).
        Assert.All(_feed.All, s => Assert.Equal(1, s.DisposeCount));
        Assert.Equal(0, _feed.ActiveWhenDisposed);
        Assert.True(disposedBeforeStopped);
    }

    [Fact]
    public async Task IdleShutdownExitsWithZero()
    {
        using IHost host = Build(ListenerHarness.NewPipeName(), TimeSpan.FromMilliseconds(300));

        int exitCode = await RunInBackground(host).WaitAsync(RunTimeout, Ct);

        Assert.Equal(0, exitCode);
        Assert.Contains(_logs.Entries, e => e.Message.Contains("No pipe client", StringComparison.Ordinal));
        Assert.True(_feed.DisposeCount > 0);
    }

    [Fact]
    public async Task TakenPipeNameExitsWithOne()
    {
        await using var owner = await ListenerHarness.StartAsync(Ct);
        using (var client = await TestClient.ConnectAsync(owner.PipeName, Ct))
        {
            await client.ReadAsync<HelloMessage>(Ct); // the other listener owns the name
        }

        using IHost host = Build(owner.PipeName, TimeSpan.FromMinutes(2));

        int exitCode = await RunInBackground(host).WaitAsync(RunTimeout, Ct);

        Assert.Equal(1, exitCode);
        Assert.Contains(_logs.Entries, e => e.Level == LogLevel.Critical && e.Message.Contains(owner.PipeName, StringComparison.Ordinal));
    }

    private IHost Build(string pipeName, TimeSpan idleAfter) => ServiceHost.Build(
        console: false,
        _logs,
        new PipeListenerOptions { PipeName = pipeName, SecurityDescriptorSddl = null },
        idleAfter,
        _ => _feed);

    private Task<int> RunInBackground(IHost host) =>
        Task.Run(() => ServiceHost.Run(host, _logs.CreateLogger("test")), Ct);
}

/// <summary>An <see cref="ILoggerProvider"/> that keeps every entry of every category.</summary>
internal sealed class ListLoggerProvider : ILoggerProvider
{
    private readonly ConcurrentQueue<LogEntry> _entries = new();

    public IReadOnlyList<LogEntry> Entries => _entries.ToArray();

    public ILogger CreateLogger(string categoryName) => new EntryLogger(_entries);

    public void Dispose()
    {
    }

    private sealed class EntryLogger(ConcurrentQueue<LogEntry> entries) : ILogger
    {
        public IDisposable? BeginScope<TState>(TState state)
            where TState : notnull => null;

        public bool IsEnabled(LogLevel logLevel) => true;

        public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter) =>
            entries.Enqueue(new LogEntry(logLevel, formatter(state, exception), exception));
    }
}
