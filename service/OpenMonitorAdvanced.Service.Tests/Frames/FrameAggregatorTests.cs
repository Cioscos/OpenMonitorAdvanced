using OpenMonitorAdvanced.Service.Frames;
using OpenMonitorAdvanced.Service.Protocol;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Frames;

public sealed class FrameAggregatorTests
{
    private const long Freq = 10_000_000;

    private static PresentMonRow Row(
        uint pid, ulong qpc, double? ms = 10.0, string mode = "Hardware: Independent Flip", ulong chain = 1, string name = "game.exe") =>
        new(pid, name, mode, new WireFrame(qpc, chain, "app", ms is not null, 10.0, ms, null, null, null, null, null));

    private static FrameAggregator NewAggregator(int batch = 512, int processes = 32) => new(batch, processes, Freq);

    [Fact]
    public void OnlyTargetRowsBecomeFrames()
    {
        var agg = NewAggregator();
        agg.SetTargets([10u]);

        agg.Add(Row(10, 100), 0);
        agg.Add(Row(20, 110), 0);
        agg.Add(Row(10, 120), 0);

        var batch = agg.TakeBatch(10);
        Assert.NotNull(batch);
        Assert.Equal([100UL, 120UL], batch.Frames.Select(f => f.Qpc));
        Assert.Equal(0u, batch.Dropped);
        Assert.Null(agg.TakeBatch(20));
        Assert.Null(agg.TakeBatch(10));
    }

    [Fact]
    public void BatchCapsAtFiveHundredTwelveAndCountsDropped()
    {
        var agg = NewAggregator();
        agg.SetTargets([10u]);
        for (ulong i = 0; i < 600; i++)
        {
            agg.Add(Row(10, 1000 + i), 0);
        }

        var first = agg.TakeBatch(10);
        Assert.NotNull(first);
        Assert.Equal(512, first.Frames.Count);
        Assert.Equal(1000UL, first.Frames[0].Qpc);
        Assert.Equal(88u, first.Dropped);

        agg.Add(Row(10, 5000), 0);
        var second = agg.TakeBatch(10);
        Assert.NotNull(second);
        Assert.Single(second.Frames);
        Assert.Equal(0u, second.Dropped);
    }

    [Fact]
    public void SummaryKeepsProcessesSeenInTheLastSecond()
    {
        var agg = NewAggregator();
        agg.Add(Row(1, 1 * Freq), 0);
        agg.Add(Row(2, 5 * Freq), 0);
        agg.Add(Row(3, 5 * Freq + Freq / 2, ms: null), 0);

        var summary = agg.TakeSummary(5 * Freq);

        var only = Assert.Single(summary.Processes);
        Assert.Equal(2u, only.Pid);
        Assert.Equal("game.exe", only.Name);
        Assert.Equal(5UL * Freq, summary.AtQpc);
    }

    [Fact]
    public void SummaryIsCappedAtThirtyTwoSortedByFps()
    {
        var agg = NewAggregator();
        for (uint pid = 1; pid <= 40; pid++)
        {
            agg.Add(Row(pid, 100 + pid, ms: 1000.0 / pid), 0); // pid N shows N fps
        }

        var summary = agg.TakeSummary(200);

        Assert.Equal(32, summary.Processes.Count);
        Assert.Equal(40u, summary.Processes[0].Pid);
        Assert.Equal(9u, summary.Processes[^1].Pid);
        Assert.True(summary.Processes.Zip(summary.Processes.Skip(1)).All(p => p.First.DisplayedFps >= p.Second.DisplayedFps));
    }

    [Fact]
    public void SummaryUsesTheMostFrequentPresentModeAndCountsSwapchains()
    {
        var agg = NewAggregator();
        agg.Add(Row(1, 100, mode: "Composed: Flip", chain: 1), 0);
        agg.Add(Row(1, 200, mode: "Hardware: Independent Flip", chain: 2), 0);
        agg.Add(Row(1, 300, mode: "Hardware: Independent Flip", chain: 2), 0);

        var p = Assert.Single(agg.TakeSummary(300).Processes);

        Assert.Equal("Hardware: Independent Flip", p.PresentMode);
        Assert.Equal(2u, p.Swapchains);
    }

    [Fact]
    public void SummaryMatchesTheFixtureFps()
    {
        var lines = File.ReadAllLines(Path.Combine(AppContext.BaseDirectory, "Fixtures", "PresentMon", "nofg.csv"));
        var csv = new PresentMonCsv();
        Assert.True(csv.TryReadHeader(lines[0], out _));
        var rows = lines.Skip(1).Select(csv.ParseRow).Select(r => r!).ToList();
        var agg = NewAggregator();
        foreach (var r in rows)
        {
            agg.Add(r, 0);
        }

        var last = rows[^1].Frame.Qpc;
        var inLastSecond = rows.Where(r => r.Frame.Qpc > last - Freq).ToList();
        var expected = 1000.0 * inLastSecond.Count / inLastSecond.Sum(r => r.Frame.MsBetweenDisplayChange!.Value);

        var p = Assert.Single(agg.TakeSummary(last).Processes);

        Assert.Equal(expected, p.DisplayedFps, 0.5);
        Assert.InRange(p.DisplayedFps, 74.2 - 0.5, 74.2 + 0.5);
    }

    [Fact]
    public void CountsArrivalStallsOverOneSecond()
    {
        var agg = NewAggregator();

        agg.Add(Row(1, 1), 0);
        agg.Add(Row(1, 2), Freq / 2);     // 0.5 s: no stall
        agg.Add(Row(1, 3), Freq / 2 + Freq + 1); // just over 1 s: stall
        agg.Add(Row(1, 4), Freq / 2 + Freq + 2);
        agg.Add(Row(1, 5), 5 * Freq);     // another stall

        Assert.Equal(2, agg.Stalls);
    }

    [Fact]
    public void ClearForgetsProcessesAndPendingFramesButKeepsTargetsAndStalls()
    {
        var agg = NewAggregator();
        agg.SetTargets([10u]);
        agg.Add(Row(10, 1 * Freq), 0);
        agg.Add(Row(10, 1 * Freq + 1), 2 * Freq); // one stall

        agg.Clear();

        Assert.Empty(agg.TakeSummary(1 * Freq).Processes);
        Assert.Null(agg.TakeBatch(10));
        Assert.Equal(1, agg.Stalls);

        // The silence across a restart is not a stall, and the target still gathers frames.
        agg.Add(Row(10, 9 * Freq), 9 * Freq);
        Assert.Equal(1, agg.Stalls);
        Assert.Single(agg.TakeBatch(10)!.Frames);
    }

    [Fact]
    public void AddAndTakeBatchFromTwoThreadsLoseNothing()
    {
        const int total = 5000;
        var agg = NewAggregator(batch: 64);
        agg.SetTargets([7u]);
        var taken = 0L;
        var dropped = 0L;
        var done = false;

        var consumer = new Thread(() =>
        {
            while (true)
            {
                var finished = Volatile.Read(ref done);
                var b = agg.TakeBatch(7);
                if (b is not null)
                {
                    taken += b.Frames.Count;
                    dropped += b.Dropped;
                }
                else if (finished)
                {
                    break;
                }
            }
        });
        consumer.Start();

        for (var i = 0; i < total; i++)
        {
            agg.Add(Row(7, (ulong)(i + 1)), i);
        }

        Volatile.Write(ref done, true);
        consumer.Join();

        Assert.Equal(total, taken + dropped);
    }
}
