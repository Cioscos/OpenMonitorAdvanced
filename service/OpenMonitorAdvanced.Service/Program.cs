// Entry point of oma-service.exe:
//   (no arguments)       the Windows service host, started by the SCM;
//   run                  the same host in a console, for manual checks from an elevated prompt
//                        (same pipe security as the service; stop it with Ctrl+C);
//   install | uninstall  the installer helper verbs (Task 4), used by the NSIS installer.

using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using OpenMonitorAdvanced.Service;
using OpenMonitorAdvanced.Service.Logging;
using OpenMonitorAdvanced.Service.Pipe;
using OpenMonitorAdvanced.Service.Setup;

return args switch
{
    [] => RunHost(console: false),
    ["run"] => RunHost(console: true),
    ["install"] => ServiceInstaller.Install(Environment.ProcessPath!, Console.Out),
    ["uninstall"] => ServiceInstaller.Uninstall(Console.Out),
    _ => Usage(),
};

static int RunHost(bool console)
{
    var fileLogs = new FileLoggerProvider(ServiceHost.LogDirectory);
    ILogger log = fileLogs.CreateLogger("OpenMonitorAdvanced.Service.Program");

    // Anything unhandled on any thread: log it and exit with code 1, so the SCM applies the
    // failure actions (restart after 60 s) instead of seeing a clean stop.
    AppDomain.CurrentDomain.UnhandledException += (_, e) =>
    {
        log.LogCritical(e.ExceptionObject as Exception, "Unhandled exception; exiting with code 1");
        if (console)
        {
            Console.Error.WriteLine($"Unhandled exception: {e.ExceptionObject}");
        }

        Environment.Exit(1);
    };
    TaskScheduler.UnobservedTaskException += (_, e) =>
    {
        log.LogWarning(e.Exception, "Unobserved task exception");
        e.SetObserved();
    };

    try
    {
        log.LogInformation(
            "oma-service {Version} starting ({Mode}, pid {Pid})",
            typeof(ServiceHost).Assembly.GetName().Version?.ToString(3),
            console ? "console" : "Windows service",
            Environment.ProcessId);
        using IHost host = ServiceHost.Build(console, fileLogs, new PipeListenerOptions(), ServiceHost.IdleAfter, ServiceHost.CreateSensorHub);
        int exitCode = ServiceHost.Run(host, log);
        log.LogInformation("oma-service stopped with exit code {ExitCode}", exitCode);
        return exitCode;
    }
    catch (Exception e)
    {
        log.LogCritical(e, "The service host could not start");
        if (console)
        {
            Console.Error.WriteLine($"The service host could not start: {e}");
        }

        return 1;
    }
}

static int Usage()
{
    Console.Error.WriteLine("Usage: oma-service.exe [run|install|uninstall]");
    return 2;
}
