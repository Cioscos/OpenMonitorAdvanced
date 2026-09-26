using System.Globalization;

using Microsoft.Extensions.Logging;

namespace OpenMonitorAdvanced.Service.Logging;

/// <summary>
/// Writes log lines to a rolling daily file under <paramref name="directory"/>
/// (<c>%ProgramData%\OpenMonitorAdvanced\logs</c> in production), keeping at most
/// <paramref name="maxFiles"/> files. The daily file name uses the local calendar date
/// (<c>TimeProvider.GetLocalNow()</c>), because the service and the logs it writes are
/// read by a person on this machine, not compared across time zones.
/// Safe for concurrent logging from several threads; never throws from <see cref="ILogger"/>
/// calls, even when <paramref name="directory"/> is not writable — a logging failure must
/// never take the service down.
/// </summary>
public sealed class FileLoggerProvider(string directory, int maxFiles = 7, TimeProvider? time = null) : ILoggerProvider
{
    private readonly TimeProvider _time = time ?? TimeProvider.System;
    private readonly Lock _writeLock = new();

    /// <summary>
    /// Date stamp (yyyyMMdd) of the last prune, or <c>null</c> before the first write. Pruning
    /// re-enumerates the whole log directory, so it only needs to run once per calendar day (the
    /// day is also the only thing that can add a new file), not on every log line.
    /// </summary>
    private string? _lastPrunedDateStamp;

    /// <summary>
    /// Test-only introspection: how many times <see cref="PruneOldFiles"/> actually ran. Internal
    /// (visible to OpenMonitorAdvanced.Service.Tests) rather than a public API.
    /// </summary>
    internal int PruneInvocationCountForTests { get; private set; }

    public ILogger CreateLogger(string categoryName) => new FileLogger(this, categoryName);

    public void Dispose()
    {
    }

    internal void Write(string categoryName, LogLevel logLevel, string message, Exception? exception)
    {
        try
        {
            var now = _time.GetLocalNow();
            var dateStamp = now.ToString("yyyyMMdd", CultureInfo.InvariantCulture);
            var fileName = $"oma-service-{dateStamp}.log";
            var line = string.Create(
                CultureInfo.InvariantCulture,
                $"{now:O} [{logLevel}] {categoryName}: {message}");
            if (exception is not null)
            {
                line += Environment.NewLine + exception;
            }

            line += Environment.NewLine;

            lock (_writeLock)
            {
                Directory.CreateDirectory(directory);
                File.AppendAllText(Path.Combine(directory, fileName), line);

                if (_lastPrunedDateStamp != dateStamp)
                {
                    PruneOldFiles();
                    _lastPrunedDateStamp = dateStamp;
                }
            }
        }
        catch
        {
            // Logging must never crash the service: swallow and move on (brief requirement).
        }
    }

    private void PruneOldFiles()
    {
        PruneInvocationCountForTests++;

        var files = new DirectoryInfo(directory).GetFiles("oma-service-*.log");
        if (files.Length <= maxFiles)
        {
            return;
        }

        foreach (var stale in files.OrderByDescending(f => f.Name, StringComparer.Ordinal).Skip(maxFiles))
        {
            stale.Delete();
        }
    }
}

internal sealed class FileLogger(FileLoggerProvider provider, string categoryName) : ILogger
{
    public IDisposable? BeginScope<TState>(TState state)
        where TState : notnull => null;

    public bool IsEnabled(LogLevel logLevel) => logLevel != LogLevel.None;

    public void Log<TState>(
        LogLevel logLevel,
        EventId eventId,
        TState state,
        Exception? exception,
        Func<TState, Exception?, string> formatter)
    {
        if (!IsEnabled(logLevel))
        {
            return;
        }

        string message;
        try
        {
            message = formatter(state, exception);
        }
        catch
        {
            return;
        }

        provider.Write(categoryName, logLevel, message, exception);
    }
}
