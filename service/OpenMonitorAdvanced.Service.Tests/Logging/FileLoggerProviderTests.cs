using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Time.Testing;

using OpenMonitorAdvanced.Service.Logging;

using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Logging;

public sealed class FileLoggerProviderTests : IDisposable
{
    private readonly string _directory;

    public FileLoggerProviderTests()
    {
        _directory = Path.Combine(Path.GetTempPath(), "oma-service-log-tests-" + Guid.NewGuid().ToString("N"));
    }

    /// <summary>The directory check of a temp directory: create it, trust it (the guard has its own tests).</summary>
    private static string? TrustAndCreate(string directory)
    {
        Directory.CreateDirectory(directory);
        return null;
    }

    public void Dispose()
    {
        LogDirectoryGuardTests.DeleteTree(_directory);
    }

    [Fact]
    public void WritesToTheDailyFile()
    {
        var time = new FakeTimeProvider(new DateTimeOffset(2026, 9, 26, 12, 0, 0, TimeSpan.Zero));
        using var provider = new FileLoggerProvider(_directory, TrustAndCreate, time: time);
        var logger = provider.CreateLogger("OpenMonitorAdvanced.Service.SomeCategory");

        logger.LogInformation("hello from the test");

        var path = Path.Combine(_directory, "oma-service-20260926.log");
        Assert.True(File.Exists(path));
        var content = File.ReadAllText(path);
        Assert.Contains("Information", content);
        Assert.Contains("OpenMonitorAdvanced.Service.SomeCategory", content);
        Assert.Contains("hello from the test", content);
    }

    [Fact]
    public void KeepsAtMostSevenFiles()
    {
        var start = new DateTimeOffset(2026, 9, 1, 0, 0, 0, TimeSpan.Zero);
        var time = new FakeTimeProvider(start);
        using var provider = new FileLoggerProvider(_directory, TrustAndCreate, maxFiles: 7, time: time);
        var logger = provider.CreateLogger("Cat");

        for (var day = 0; day < 9; day++)
        {
            logger.LogInformation("day {Day}", day);
            time.Advance(TimeSpan.FromDays(1));
        }

        var files = Directory.GetFiles(_directory, "oma-service-*.log")
            .Select(Path.GetFileName)
            .OrderBy(name => name, StringComparer.Ordinal)
            .ToArray();

        Assert.Equal(7, files.Length);
        Assert.Equal("oma-service-20260903.log", files[0]);
        Assert.Equal("oma-service-20260909.log", files[^1]);
    }

    [Fact]
    public void PrunesOnlyOnceWhenLoggingMultipleLinesOnTheSameDay()
    {
        var time = new FakeTimeProvider(new DateTimeOffset(2026, 9, 26, 0, 0, 0, TimeSpan.Zero));
        using var provider = new FileLoggerProvider(_directory, TrustAndCreate, time: time);
        var logger = provider.CreateLogger("Cat");

        for (var i = 0; i < 5; i++)
        {
            logger.LogInformation("line {I}", i);
        }

        // One prune for the first write of the day; the other 4 lines on the same day must not
        // each re-enumerate the log directory.
        Assert.Equal(1, provider.PruneInvocationCountForTests);
    }

    [Fact]
    public void PrunesOnceMorePerNewDay()
    {
        var time = new FakeTimeProvider(new DateTimeOffset(2026, 9, 26, 0, 0, 0, TimeSpan.Zero));
        using var provider = new FileLoggerProvider(_directory, TrustAndCreate, time: time);
        var logger = provider.CreateLogger("Cat");

        logger.LogInformation("day one, line one");
        logger.LogInformation("day one, line two");
        time.Advance(TimeSpan.FromDays(1));
        logger.LogInformation("day two, line one");

        Assert.Equal(2, provider.PruneInvocationCountForTests);
    }

    [Fact]
    public void DoesNotThrowWhenTheDirectoryIsNotWritable()
    {
        // A path with an embedded NUL character is never a valid directory on Windows,
        // which is the simplest reliable way to force every filesystem call to fail
        // without depending on ACL manipulation.
        var unwritable = Path.Combine(_directory, "sub\0dir");
        using var provider = new FileLoggerProvider(unwritable, TrustAndCreate);
        var logger = provider.CreateLogger("Cat");

        var exception = Record.Exception(() => logger.LogInformation("this must not throw"));

        Assert.Null(exception);
    }

    [Fact]
    public void ARefusedDirectoryDisablesFileLoggingAndIsReportedOnce()
    {
        var time = new FakeTimeProvider(new DateTimeOffset(2026, 9, 26, 12, 0, 0, TimeSpan.Zero));
        var reports = new List<string>();
        int checks = 0;
        using var provider = new FileLoggerProvider(
            _directory,
            _ =>
            {
                checks++;
                return "owner S-1-5-21-1 is not SYSTEM or Administrators";
            },
            time: time,
            onDisabled: reports.Add);
        var logger = provider.CreateLogger("Cat");

        logger.LogInformation("first");
        logger.LogInformation("second");
        time.Advance(TimeSpan.FromDays(1));
        logger.LogInformation("next day");

        Assert.False(Directory.Exists(_directory));
        Assert.Equal(1, checks);
        string report = Assert.Single(reports);
        Assert.Contains("owner S-1-5-21-1 is not SYSTEM or Administrators", report);
        Assert.Contains(_directory, report);
    }

    [Fact]
    public void AThrowingCheckDisablesFileLoggingToo()
    {
        var reports = new List<string>();
        using var provider = new FileLoggerProvider(_directory, _ => throw new UnauthorizedAccessException("denied"), onDisabled: reports.Add);

        var exception = Record.Exception(() => provider.CreateLogger("Cat").LogInformation("x"));

        Assert.Null(exception);
        Assert.Contains("denied", Assert.Single(reports));
    }

    [Fact]
    public void TheDirectoryIsCheckedBeforeTheFirstWriteAndAgainWhenTheDayRolls()
    {
        var time = new FakeTimeProvider(new DateTimeOffset(2026, 9, 26, 12, 0, 0, TimeSpan.Zero));
        var checkedBefore = new List<bool>();
        using var provider = new FileLoggerProvider(
            _directory,
            d =>
            {
                checkedBefore.Add(Directory.Exists(d) && Directory.EnumerateFiles(d).Any());
                return TrustAndCreate(d);
            },
            time: time);
        var logger = provider.CreateLogger("Cat");

        logger.LogInformation("one");
        logger.LogInformation("two");
        time.Advance(TimeSpan.FromDays(1));
        logger.LogInformation("three");

        Assert.Equal([false, true], checkedBefore); // before any file; then once more on the new day
        Assert.Equal(2, Directory.GetFiles(_directory, "oma-service-*.log").Length);
    }

    [Fact]
    public void ADailyFileThatIsAReparsePointIsNeverFollowed()
    {
        var time = new FakeTimeProvider(new DateTimeOffset(2026, 9, 26, 12, 0, 0, TimeSpan.Zero));
        Directory.CreateDirectory(_directory);
        string target = Path.Combine(_directory, "target");
        Directory.CreateDirectory(target);
        LogDirectoryGuardTests.MakeJunction(Path.Combine(_directory, "oma-service-20260926.log"), target);
        var reports = new List<string>();
        using var provider = new FileLoggerProvider(_directory, TrustAndCreate, time: time, onDisabled: reports.Add);

        provider.CreateLogger("Cat").LogInformation("must not be written through the link");

        Assert.Contains("junction or link", Assert.Single(reports));
        Assert.Empty(Directory.EnumerateFileSystemEntries(target));
    }
}
