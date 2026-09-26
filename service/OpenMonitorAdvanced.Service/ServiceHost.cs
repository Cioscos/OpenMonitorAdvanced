using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Hosting.WindowsServices;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;
using OpenMonitorAdvanced.Service.Pipe;
using OpenMonitorAdvanced.Service.Sensors;
using OpenMonitorAdvanced.Service.Setup;

namespace OpenMonitorAdvanced.Service;

/// <summary>
/// Composition of the service host (spec §5.3, §6, §8): the sensor hub as the singleton
/// <see cref="ISensorFeed"/>, the pipe listener, the idle shutdown and the logging. The same host
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

    /// <summary><c>%ProgramData%\OpenMonitorAdvanced\logs</c>.</summary>
    internal static string LogDirectory { get; } = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.CommonApplicationData),
        "OpenMonitorAdvanced",
        "logs");

    internal static IHost Build(
        bool console,
        ILoggerProvider fileLogs,
        PipeListenerOptions pipeOptions,
        TimeSpan idleAfter,
        Func<IServiceProvider, ISensorFeed> feed)
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

        builder.Services.AddSingleton(feed);
        builder.Services.AddSingleton(pipeOptions);
        builder.Services.AddSingleton(sp => new IdleShutdown(
            sp.GetRequiredService<IHostApplicationLifetime>(),
            TimeProvider.System,
            idleAfter,
            sp.GetRequiredService<ILogger<IdleShutdown>>()));
        builder.Services.AddSingleton<PipeListener>();

        // Hosted services stop in reverse order: the pipe (and every session's subscription)
        // first, then the feed, so the hub is disposed with no subscriber left and before the
        // host reports itself stopped.
        builder.Services.AddHostedService<FeedShutdown>();
        builder.Services.AddHostedService(sp => sp.GetRequiredService<PipeListener>());

        return builder.Build();
    }

    /// <summary>The production feed: <see cref="SensorHub"/> over LibreHardwareMonitor (Task 6).</summary>
    internal static ISensorFeed CreateSensorHub(IServiceProvider services)
    {
        ILoggerFactory logs = services.GetRequiredService<ILoggerFactory>();
        var disks = new DiskPowerProbe(logs.CreateLogger<DiskPowerProbe>());
        var pawnIo = new PawnIoProbe(logs.CreateLogger<PawnIoProbe>());

        // Owned by the hub, never registered in DI: the hub decides whether it may be closed.
        var tree = new LhmTree(logs.CreateLogger<LhmTree>());
        return new SensorHub(tree, disks, pawnIo.IsAvailable, TimeProvider.System, logs.CreateLogger<SensorHub>());
    }

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

    /// <summary>Disposes the feed while the host stops (after the pipe listener), within the shutdown timeout.</summary>
    private sealed class FeedShutdown(ISensorFeed feed, ILogger<FeedShutdown> log) : IHostedService
    {
        public Task StartAsync(CancellationToken cancellationToken) => Task.CompletedTask;

        public Task StopAsync(CancellationToken cancellationToken)
        {
            try
            {
                (feed as IDisposable)?.Dispose();
            }
            catch (Exception e)
            {
                log.LogError(e, "Stopping the sensor feed failed");
            }

            return Task.CompletedTask;
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
