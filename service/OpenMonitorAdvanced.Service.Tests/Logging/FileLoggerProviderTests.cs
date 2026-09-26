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

    public void Dispose()
    {
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }
    }

    [Fact]
    public void WritesToTheDailyFile()
    {
        var time = new FakeTimeProvider(new DateTimeOffset(2026, 9, 26, 12, 0, 0, TimeSpan.Zero));
        using var provider = new FileLoggerProvider(_directory, time: time);
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
        using var provider = new FileLoggerProvider(_directory, maxFiles: 7, time: time);
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
    public void DoesNotThrowWhenTheDirectoryIsNotWritable()
    {
        // A path with an embedded NUL character is never a valid directory on Windows,
        // which is the simplest reliable way to force every filesystem call to fail
        // without depending on ACL manipulation.
        var unwritable = Path.Combine(_directory, "sub\0dir");
        using var provider = new FileLoggerProvider(unwritable);
        var logger = provider.CreateLogger("Cat");

        var exception = Record.Exception(() => logger.LogInformation("this must not throw"));

        Assert.Null(exception);
    }
}
