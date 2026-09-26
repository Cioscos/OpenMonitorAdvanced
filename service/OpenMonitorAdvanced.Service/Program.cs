// Entry point for the `oma-service.exe install|uninstall` helper verbs (Task 4), used by the
// NSIS installer. The service host itself (sampling/named-pipe) arrives in Task 7.

using OpenMonitorAdvanced.Service.Setup;

return args switch
{
    ["install"] => ServiceInstaller.Install(Environment.ProcessPath!, Console.Out),
    ["uninstall"] => ServiceInstaller.Uninstall(Console.Out),
    _ => Usage(),
};

static int Usage()
{
    Console.Error.WriteLine("Usage: oma-service.exe install|uninstall");
    return 2;
}
