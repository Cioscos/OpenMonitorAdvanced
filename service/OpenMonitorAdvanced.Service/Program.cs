// Minimal, compilable entry point for this task. Replaced by the setup verbs
// (install/uninstall/…) in Task 4 and by the sampling/named-pipe host in
// Task 7.

using Microsoft.Extensions.Hosting;

var builder = Host.CreateApplicationBuilder(args);
using var host = builder.Build();
host.Run();
