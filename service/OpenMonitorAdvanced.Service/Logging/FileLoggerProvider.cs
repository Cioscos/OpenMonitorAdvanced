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
/// <param name="directory">The log directory.</param>
/// <param name="prepareDirectory">
/// Runs before the first write and again before the first write of each new day: creates the
/// directory if needed and returns <see langword="null"/> when it may be written, otherwise the
/// reason (<see cref="LogDirectoryGuard.Prepare"/> in production). A refusal, or an exception,
/// disables file logging for the rest of the process: the service runs as SYSTEM and must never
/// follow a path another user could have redirected (final review C1).
/// </param>
/// <param name="maxFiles">How many daily files to keep.</param>
/// <param name="time">The clock (tests use a fake one).</param>
/// <param name="onDisabled">Told once, with the reason, when file logging is disabled.</param>
public sealed class FileLoggerProvider(
    string directory,
    Func<string, string?> prepareDirectory,
    int maxFiles = 7,
    TimeProvider? time = null,
    Action<string>? onDisabled = null) : ILoggerProvider
{
    private readonly TimeProvider _time = time ?? TimeProvider.System;
    private readonly Lock _writeLock = new();

    /// <summary>
    /// Date stamp (yyyyMMdd) of the last check and prune, or <c>null</c> before the first write.
    /// Pruning re-enumerates the whole log directory, so it only needs to run once per calendar
    /// day (the day is also the only thing that can add a new file), not on every log line.
    /// </summary>
    private string? _lastPrunedDateStamp;

    private bool _disabled;

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
        string? disabledReason = null;
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
                if (_disabled)
                {
                    return;
                }

                string path = Path.Combine(directory, fileName);
                if (_lastPrunedDateStamp != dateStamp)
                {
                    disabledReason = Verify(path);
                    if (disabledReason is not null)
                    {
                        _disabled = true;
                        return;
                    }
                }

                File.AppendAllText(path, line);

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
        finally
        {
            if (disabledReason is not null)
            {
                Report($"File logging to {directory} is disabled: {disabledReason}");
            }
        }
    }

    /// <summary>The directory check, then the day's file: it must not be a link planted in advance.</summary>
    private string? Verify(string dailyFile)
    {
        string? reason;
        try
        {
            reason = prepareDirectory(directory);
        }
        catch (Exception e)
        {
            reason = $"the directory check failed ({e.GetType().Name}: {e.Message})";
        }

        if (reason is not null)
        {
            return reason;
        }

        FileAttributes attributes;
        try
        {
            attributes = File.GetAttributes(dailyFile); // the entry itself, never a link's target
        }
        catch (Exception e) when (e is FileNotFoundException or DirectoryNotFoundException)
        {
            return null; // created by this write
        }

        if (LogDirectoryGuard.IsRegularFile(attributes))
        {
            return null;
        }

        return attributes.HasFlag(FileAttributes.ReparsePoint)
            ? $"{dailyFile} is a junction or link"
            : $"{dailyFile} is not a regular file";
    }

    private void Report(string message)
    {
        try
        {
            onDisabled?.Invoke(message);
        }
        catch
        {
            // Nowhere left to report to.
        }
    }

    /// <summary>
    /// Deletes all but the newest <c>maxFiles</c> daily files. Only regular files directly inside
    /// the (verified) directory: never a link, a junction or a subfolder.
    /// </summary>
    private void PruneOldFiles()
    {
        PruneInvocationCountForTests++;

        FileInfo[] files = new DirectoryInfo(directory)
            .GetFiles("oma-service-*.log", SearchOption.TopDirectoryOnly)
            .Where(f => LogDirectoryGuard.IsRegularFile(f.Attributes))
            .ToArray();
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
