using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading.Channels;

namespace OpenMonitorAdvanced.Service.Frames;

/// <summary>How a PresentMon run ended: exit code and the last stderr lines (at most 20, 512 chars each).</summary>
internal sealed record PresentMonExit(int ExitCode, IReadOnlyList<string> StderrTail);

/// <summary>One running capture. Disposing it kills the child (through the job) and waits up to 5 s.</summary>
internal interface IPresentMonRun : IDisposable
{
    ChannelReader<string> StdoutLines { get; }

    Task<PresentMonExit> Exited { get; }
}

/// <summary>Starts capture runs; a seam so the lifecycle can be tested with fakes.</summary>
internal interface IFrameSource
{
    IPresentMonRun Start(IReadOnlyList<string> arguments);
}

/// <summary>
/// <see cref="IFrameSource"/> that runs an executable as a child inside a Job Object with
/// <c>JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE</c>, so the child dies with the service even if it crashes.
/// </summary>
internal sealed class PresentMonProcess(string exePath) : IFrameSource
{
    internal const int ChannelCapacity = 8192;
    internal const int StderrLines = 20;
    internal const int StderrLineChars = 512;

    public IPresentMonRun Start(IReadOnlyList<string> arguments)
    {
        var info = new ProcessStartInfo(exePath)
        {
            UseShellExecute = false,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            CreateNoWindow = true,
        };
        foreach (string a in arguments)
        {
            info.ArgumentList.Add(a);
        }

        FramesNative.JobHandle job = CreateKillOnCloseJob();
        Process? process = null;
        try
        {
            process = Process.Start(info) ?? throw new InvalidOperationException("Process.Start returned null.");

            // Assigned right after the start: in the few microseconds before this call the child is
            // not yet in the job, so a service crash exactly then would leave it running. Starting
            // suspended would close the window but is not worth the CreateProcess plumbing here.
            // SAFETY: `process.Handle` is valid while `process` is alive (it is, until the run disposes it).
            if (!FramesNative.AssignProcessToJobObject(job, process.Handle))
            {
                throw new Win32Exception();
            }

            return new Run(process, job);
        }
        catch
        {
            try
            {
                process?.Kill(entireProcessTree: true);
            }
            catch (Exception ex) when (ex is InvalidOperationException or Win32Exception)
            {
                // Already gone.
            }

            process?.Dispose();
            job.Dispose();
            throw;
        }
    }

    private static FramesNative.JobHandle CreateKillOnCloseJob()
    {
        FramesNative.JobHandle job = FramesNative.CreateJobObjectW(IntPtr.Zero, null);
        if (job.IsInvalid)
        {
            throw new Win32Exception();
        }

        try
        {
            var limits = new FramesNative.JobObjectExtendedLimitInformation();
            limits.BasicLimitInformation.LimitFlags = FramesNative.JobObjectLimitKillOnJobClose;

            // SAFETY: `limits` is a blittable struct passed by reference with its exact size; `job` is a live handle.
            if (!FramesNative.SetInformationJobObject(
                    job,
                    FramesNative.JobObjectExtendedLimitInformationClass,
                    ref limits,
                    (uint)Marshal.SizeOf<FramesNative.JobObjectExtendedLimitInformation>()))
            {
                throw new Win32Exception();
            }

            return job;
        }
        catch
        {
            job.Dispose();
            throw;
        }
    }

    private sealed class Run : IPresentMonRun
    {
        private readonly Process _process;
        private readonly FramesNative.JobHandle _job;
        private readonly Channel<string> _lines = Channel.CreateBounded<string>(
            new BoundedChannelOptions(ChannelCapacity)
            {
                FullMode = BoundedChannelFullMode.DropOldest,
                SingleReader = true,
                SingleWriter = true,
            });

        private readonly Queue<string> _stderr = new();
        private int _disposed;

        public Run(Process process, FramesNative.JobHandle job)
        {
            _process = process;
            _job = job;
            Exited = Pump();
        }

        public ChannelReader<string> StdoutLines => _lines.Reader;

        public Task<PresentMonExit> Exited { get; }

        public void Dispose()
        {
            if (Interlocked.Exchange(ref _disposed, 1) != 0)
            {
                return;
            }

            // Closing the last job handle kills the child; the pipes then reach end of stream and the pump ends.
            _job.Dispose();
            try
            {
                Exited.Wait(TimeSpan.FromSeconds(5));
            }
            catch (AggregateException)
            {
                // A pump failure is already reflected in Exited; nothing more to do here.
            }

            // Only dispose the process once the pump is done with it; on a timeout the finalizer cleans up.
            if (Exited.IsCompleted)
            {
                _process.Dispose();
            }
        }

        private async Task<PresentMonExit> Pump()
        {
            Task stdout = ReadStdout();
            Task stderr = ReadStderr();
            await Task.WhenAll(stdout, stderr).ConfigureAwait(false);
            await _process.WaitForExitAsync().ConfigureAwait(false);
            string[] tail;
            lock (_stderr)
            {
                tail = [.. _stderr];
            }

            return new PresentMonExit(_process.ExitCode, tail);
        }

        private async Task ReadStdout()
        {
            try
            {
                string? line;
                while ((line = await _process.StandardOutput.ReadLineAsync().ConfigureAwait(false)) is not null)
                {
                    _lines.Writer.TryWrite(line);
                }
            }
            finally
            {
                _lines.Writer.TryComplete();
            }
        }

        private async Task ReadStderr()
        {
            string? line;
            while ((line = await _process.StandardError.ReadLineAsync().ConfigureAwait(false)) is not null)
            {
                if (line.Length > StderrLineChars)
                {
                    line = line[..StderrLineChars];
                }

                lock (_stderr)
                {
                    _stderr.Enqueue(line);
                    while (_stderr.Count > StderrLines)
                    {
                        _stderr.Dequeue();
                    }
                }
            }
        }
    }
}
