using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Hosting.WindowsServices;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using OpenMonitorAdvanced.Service.Frames;
using OpenMonitorAdvanced.Service.Pipe;
using OpenMonitorAdvanced.Service.Sensors;
using OpenMonitorAdvanced.Service.Setup;

namespace OpenMonitorAdvanced.Service;

/// <summary>
/// Composition of the service host (spec §5.3, §6, §8): the sensor hub as the singleton
/// <see cref="ISensorFeed"/>, the frames hub (spec M7b §4.1), the pipe listener, the idle shutdown
/// and the logging. The same host
/// runs under the SCM (<c>oma-service.exe</c> with no arguments) and in a console
/// (<c>oma-service.exe run</c>), with the same pipe security in both.
/// </summary>
internal static class ServiceHost
{
    internal static readonly TimeSpan IdleAfter = TimeSpan.FromMinutes(2);

    /// <summary>
    /// How long the host waits for its services to stop. <see cref="SensorHub.Dispose"/> alone may
    /// take up to about 20 s (two bounded 10 s worker joins), so this leaves room for it.
    /// </summary>
    internal static readonly TimeSpan ShutdownTimeout = TimeSpan.FromSeconds(30);

    /// <summary>
    /// The folder of <c>oma-service.exe</c> (<c>$INSTDIR\service</c> once installed, locked down
    /// by the installer): the root below which <see cref="LogDirectory"/> is checked.
    /// </summary>
    internal static string ServiceDirectory { get; } = Path.TrimEndingDirectorySeparator(AppContext.BaseDirectory);

    /// <summary>
    /// <c>&lt;service folder&gt;\logs</c> (ruling R30: never a folder another user could create
    /// first, unlike <c>%ProgramData%</c>), checked before the service writes to it
    /// (<see cref="Logging.LogDirectoryGuard"/>).
    /// </summary>
    internal static string LogDirectory { get; } = Path.Combine(ServiceDirectory, "logs");

    /// <summary>The pinned PresentMon, installed in <c>$INSTDIR\service\presentmon</c>.</summary>
    internal static string PresentMonPath { get; } = Path.Combine(ServiceDirectory, "presentmon", PresentMonPin.FileName);

    internal static IHost Build(
        bool console,
        ILoggerProvider fileLogs,
        PipeListenerOptions pipeOptions,
        TimeSpan idleAfter,
        Func<IServiceProvider, ISensorFeed> feed,
        Func<IServiceProvider, FrameCapture> frameCapture)
    {
        HostApplicationBuilder builder = Host.CreateApplicationBuilder(new HostApplicationBuilderSettings
        {
            Args = [],
            ApplicationName = ServiceInstaller.ServiceName,
            ContentRootPath = AppContext.BaseDirectory,
        });

        builder.Services.AddWindowsService(o => o.ServiceName = ServiceInstaller.ServiceName);
        if (WindowsServiceHelpers.IsWindowsService())
        {
            // Replaces the WindowsServiceLifetime registered above, to ask the SCM for stop time.
            builder.Services.AddSingleton<IHostLifetime, OmaServiceLifetime>();
        }

        builder.Services.Configure<HostOptions>(o =>
        {
            o.ShutdownTimeout = ShutdownTimeout;
            o.BackgroundServiceExceptionBehavior = BackgroundServiceExceptionBehavior.StopHost;
        });

        builder.Logging.ClearProviders();
        builder.Logging.SetMinimumLevel(LogLevel.Information);
        builder.Logging.AddProvider(fileLogs);
        if (console)
        {
            builder.Logging.AddSimpleConsole(o =>
            {
                o.SingleLine = true;
                o.TimestampFormat = "HH:mm:ss ";
            });
        }

        // Computed once per process, on first use, and shared by the hub and every client's Hello.
        builder.Services.AddSingleton(sp =>
        {
            ILogger<PawnIoProbe> probeLog = sp.GetRequiredService<ILoggerFactory>().CreateLogger<PawnIoProbe>();
            return new PawnIoState(() => ProbePawnIo(new PawnIoProbe(probeLog), probeLog));
        });
        builder.Services.AddSingleton(feed);
        builder.Services.AddSingleton(pipeOptions);
        builder.Services.AddSingleton(sp => new IdleShutdown(
            sp.GetRequiredService<IHostApplicationLifetime>(),
            TimeProvider.System,
            idleAfter,
            sp.GetRequiredService<ILogger<IdleShutdown>>()));

        // The frame engine: nothing runs until a client asks for frames (the capture's constructor
        // only stops a session left over by a crashed service).
        builder.Services.AddSingleton(frameCapture);
        builder.Services.AddSingleton(_ => new FrameAggregator(ticksPerSecond: TimeProvider.System.TimestampFrequency));
        builder.Services.AddSingleton(_ => new FrameRequests(TimeProvider.System));
        builder.Services.AddSingleton(sp => new FramesHub(
            sp.GetRequiredService<FrameCapture>(),
            sp.GetRequiredService<FrameAggregator>(),
            sp.GetRequiredService<FrameRequests>(),
            TimeProvider.System,
            sp.GetRequiredService<ILogger<FramesHub>>()));
        builder.Services.AddSingleton<PipeListener>();

        // Hosted services stop in reverse order: the pipe (and every session's subscription)
        // first, then the feed and the frames hub, so both are disposed with no subscriber left
        // and before the host reports itself stopped.
        builder.Services.AddHostedService<FeedShutdown>();
        builder.Services.AddHostedService(sp => sp.GetRequiredService<PipeListener>());

        return builder.Build();
    }

    private static PawnIoStatus ProbePawnIo(PawnIoProbe probe, ILogger log)
    {
        try
        {
            return probe.Probe();
        }
        catch (Exception e)
        {
            log.LogError(e, "Probing PawnIO failed; its status is unknown");
            return PawnIoStatus.Unknown;
        }
    }

    /// <summary>The production feed: <see cref="SensorHub"/> over LibreHardwareMonitor (Task 6).</summary>
    internal static ISensorFeed CreateSensorHub(IServiceProvider services)
    {
        ILoggerFactory logs = services.GetRequiredService<ILoggerFactory>();
        var disks = new DiskPowerProbe(logs.CreateLogger<DiskPowerProbe>());
        var activity = new DiskActivityProbe(logs.CreateLogger<DiskActivityProbe>());
        PawnIoState pawnIo = services.GetRequiredService<PawnIoState>();

        // Owned by the hub, never registered in DI: the hub decides whether it may be closed.
        var tree = new LhmTree(logs.CreateLogger<LhmTree>());
        return new SensorHub(tree, disks, activity, () => pawnIo.Status == PawnIoStatus.Ok, TimeProvider.System, logs.CreateLogger<SensorHub>());
    }

    /// <summary>
    /// The production frame capture: the pinned PresentMon and the real ETW session control. Tests
    /// pass fakes instead, so no test stops the product's live ETW session.
    /// </summary>
    internal static FrameCapture CreateFrameCapture(IServiceProvider services) => new(
        new PresentMonProcess(PresentMonPath),
        new EtwSessionControl(),
        () => PresentMonPin.HashOf(PresentMonPath),
        TimeProvider.System,
        services.GetRequiredService<ILogger<FrameCapture>>());

    /// <summary>
    /// Runs <paramref name="host"/> until it stops and returns the process exit code: 0 for a
    /// requested stop (SCM, Ctrl+C, idle shutdown), 1 if the host failed or the pipe listener
    /// faulted (for example, the pipe name is taken).
    /// </summary>
    internal static int Run(IHost host, ILogger log)
    {
        // Resolved first: Run() disposes the host (and its container) on the way out. It does not
        // throw when a BackgroundService faults either, hence the explicit check afterwards.
        PipeListener listener = host.Services.GetRequiredService<PipeListener>();
        try
        {
            host.Run();
        }
        catch (Exception e)
        {
            log.LogCritical(e, "The service host failed");
            return 1;
        }

        if (listener.ExecuteTask is { IsFaulted: true })
        {
            log.LogCritical("The pipe listener failed; exiting with code 1");
            return 1;
        }

        return 0;
    }

    /// <summary>
    /// Disposes the feed and the frames hub while the host stops (after the pipe listener), within
    /// the shutdown timeout. They stop side by side: the feed may take about 20 s and the frame
    /// capture up to 15 s, which in sequence would not fit in <see cref="ShutdownTimeout"/>.
    /// </summary>
    private sealed class FeedShutdown(ISensorFeed feed, FramesHub frames, ILogger<FeedShutdown> log) : IHostedService
    {
        public Task StartAsync(CancellationToken cancellationToken) => Task.CompletedTask;

        public Task StopAsync(CancellationToken cancellationToken) => Task.WhenAll(
            Task.Run(() => Stop(() => (feed as IDisposable)?.Dispose(), "the sensor feed"), CancellationToken.None),
            Task.Run(() => Stop(frames.Dispose, "the frame capture"), CancellationToken.None));

        private void Stop(Action dispose, string what)
        {
            try
            {
                dispose();
            }
            catch (Exception e)
            {
                log.LogError(e, "Stopping {What} failed", what);
            }
        }
    }
}

/// <summary>
/// The Windows service lifetime, plus two things the stock one lacks: it asks the SCM for enough
/// stop time to cover <see cref="ServiceHost.ShutdownTimeout"/>, and it reports exit code 1 to the
/// SCM when the pipe listener faulted. It does not override <c>OnCustomCommand</c>: custom
/// control codes 128–255, which interactive users may send, stay without effect (spec §6).
/// </summary>
internal sealed class OmaServiceLifetime(
    IHostEnvironment environment,
    IHostApplicationLifetime applicationLifetime,
    ILoggerFactory loggerFactory,
    IOptions<HostOptions> optionsAccessor,
    IOptions<WindowsServiceLifetimeOptions> windowsServiceOptionsAccessor,
    PipeListener listener)
    : WindowsServiceLifetime(environment, applicationLifetime, loggerFactory, optionsAccessor, windowsServiceOptionsAccessor)
{
    protected override void OnStop()
    {
        try
        {
            // Wait hint for STOP_PENDING: the host's own timeout plus a margin.
            RequestAdditionalTime((int)(ServiceHost.ShutdownTimeout + TimeSpan.FromSeconds(5)).TotalMilliseconds);
        }
        catch (InvalidOperationException)
        {
            // Not in a pending state: nothing to extend.
        }

        base.OnStop(); // stops the host and waits for it (up to the shutdown timeout)

        if (listener.ExecuteTask is { IsFaulted: true })
        {
            ExitCode = 1;
        }
    }
}
