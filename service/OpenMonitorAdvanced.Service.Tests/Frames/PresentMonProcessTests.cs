using OpenMonitorAdvanced.Service.Frames;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Frames;

public sealed class PresentMonProcessTests
{
    private static readonly string Cmd = Path.Combine(Environment.SystemDirectory, "cmd.exe");
    private static readonly TimeSpan Limit = TimeSpan.FromSeconds(10);

    [Fact]
    public async Task LinesFromStdoutArrive()
    {
        using IPresentMonRun run = new PresentMonProcess(Cmd).Start(["/c", "echo a& echo b"]);
        PresentMonExit exit = await run.Exited.WaitAsync(Limit, TestContext.Current.CancellationToken);
        var lines = new List<string>();
        await foreach (string l in run.StdoutLines.ReadAllAsync(TestContext.Current.CancellationToken))
        {
            lines.Add(l);
        }

        Assert.Equal(0, exit.ExitCode);
        Assert.Equal(["a", "b"], lines);
    }

    [Fact]
    public async Task ExitCodeAndStderrAreReported()
    {
        using IPresentMonRun run = new PresentMonProcess(Cmd).Start(["/c", "echo boom 1>&2& exit 6"]);
        PresentMonExit exit = await run.Exited.WaitAsync(Limit, TestContext.Current.CancellationToken);

        Assert.Equal(6, exit.ExitCode);
        Assert.Contains(exit.StderrTail, l => l.Contains("boom"));
    }

    [Fact]
    public async Task DisposeKillsTheChildThroughTheJob()
    {
        IPresentMonRun run = new PresentMonProcess(Cmd).Start(["/c", "ping -n 30 127.0.0.1 >nul"]);
        run.Dispose();

        // Dispose waited up to 5 s for the pump: the child (cmd and its ping) must be gone by now.
        Assert.True(run.Exited.IsCompleted);
        await run.Exited; // would take ~30 s if the child had survived the job closing
        run.Dispose(); // idempotent
    }
}
