using System.Collections.Concurrent;
using System.ComponentModel;
using System.IO.Pipes;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Microsoft.Win32.SafeHandles;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;

namespace OpenMonitorAdvanced.Service.Pipe;

public sealed class PipeListenerOptions
{
    public string PipeName { get; init; } = ProtocolConstants.PipeName;

    public string? SecurityDescriptorSddl { get; init; } = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x0012019b;;;IU)"; // null = default DACL (tests)

    public int MaxClients { get; init; } = 8;

    public TimeSpan SubscribeTimeout { get; init; } = TimeSpan.FromSeconds(5);

    /// <summary>A write not completing within this closes the session (client stuck). Internal: tests shorten it.</summary>
    internal TimeSpan WriteTimeout { get; init; } = TimeSpan.FromSeconds(5);
}

/// <summary>
/// The sensor pipe's server (spec §6, spike S3 §1): accepts up to <see cref="PipeListenerOptions.MaxClients"/>
/// clients and runs one <see cref="ClientSession"/> per connection.
/// </summary>
/// <remarks>
/// <para>
/// <b>Instances.</b> Every instance comes from <c>CreateNamedPipeW</c> (byte mode, overlapped,
/// <c>PIPE_REJECT_REMOTE_CLIENTS</c>, the configured descriptor); the first one also claims the
/// name with <c>FILE_FLAG_FIRST_PIPE_INSTANCE</c>, and failing to create it is fatal (the name is
/// taken, or the pipe cannot exist at all): <see cref="BackgroundService.ExecuteTask"/> faults and the
/// host exits with code 1. Exactly one instance listens at a time.
/// </para>
/// <para>
/// <b>Never zero instances.</b> When a client connects while fewer than <c>MaxClients</c> are
/// busy, the next listening instance is created <em>before</em> the new session starts, so a
/// quick client that leaves at once never takes the last instance (and the name) with it. The
/// Windows limit counts the listening instance too, so the client that makes <c>MaxClients</c>
/// busy starts its session immediately and no instance listens until one of them ends; the
/// connected ones keep the name alive meanwhile, and further clients see <c>ERROR_PIPE_BUSY</c>.
/// </para>
/// <para>
/// <b>Accounting.</b> For each connection the listener calls <see cref="IdleShutdown.ClientConnected"/>
/// once and, when its session ends for whatever reason, disposes the instance, calls
/// <see cref="IdleShutdown.ClientDisconnected"/> and frees the slot, each exactly once.
/// </para>
/// </remarks>
public sealed class PipeListener(ISensorFeed feed, PipeListenerOptions options, IdleShutdown idle, PawnIoState pawnIo, ILogger<PipeListener> log) : BackgroundService
{
    private static readonly TimeSpan CreateRetryDelay = TimeSpan.FromSeconds(1);

    private readonly SemaphoreSlim _slotFreed = new(0);
    private readonly ConcurrentDictionary<int, Task> _sessions = new();
    private int _busy;
    private int _nextClientId;
    private bool _createFailed;
    private int? _loggedCreateError;

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        NamedPipeServerStream? listening;
        try
        {
            listening = Wrap(PipeNative.CreateInstance(options.PipeName, first: true, options.MaxClients, options.SecurityDescriptorSddl));
        }
        catch (Win32Exception e)
        {
            if (e.NativeErrorCode == PipeNative.ErrorAccessDenied)
            {
                log.LogCritical(
                    "The pipe {Pipe} already exists or cannot be claimed (Win32 error {Error}): another process holds the name; stopping",
                    options.PipeName,
                    e.NativeErrorCode);
            }
            else
            {
                log.LogCritical(e, "Creating the pipe {Pipe} failed (Win32 error {Error}); stopping", options.PipeName, e.NativeErrorCode);
            }

            throw;
        }

        log.LogInformation("Listening on pipe {Pipe} (at most {Max} clients)", options.PipeName, options.MaxClients);
        idle.Start();
        try
        {
            while (true)
            {
                while (listening is null)
                {
                    // At capacity: wait for a session to end. After a failed creation: retry soon.
                    TimeSpan wait = _createFailed ? CreateRetryDelay : Timeout.InfiniteTimeSpan;
                    await _slotFreed.WaitAsync(wait, stoppingToken).ConfigureAwait(false);
                    if (Volatile.Read(ref _busy) < options.MaxClients)
                    {
                        listening = TryCreateNext();
                    }
                }

                try
                {
                    await listening.WaitForConnectionAsync(stoppingToken).ConfigureAwait(false);
                }
                catch (IOException e)
                {
                    // A client connected and left before the connection completed (e.g. Win32 232).
                    log.LogDebug(e, "A pipe connection failed before it was established");
                    listening = Reset(listening);
                    continue;
                }

                NamedPipeServerStream connected = listening;
                listening = null;
                int busy = Interlocked.Increment(ref _busy);
                idle.ClientConnected();

                // Next listening instance first, then the session (see remarks).
                if (busy < options.MaxClients)
                {
                    listening = TryCreateNext();
                }

                StartSession(connected, stoppingToken);
            }
        }
        catch (OperationCanceledException) when (stoppingToken.IsCancellationRequested)
        {
            // Host stopping.
        }
        finally
        {
            listening?.Dispose();
            await Task.WhenAll(_sessions.Values).ConfigureAwait(false);
            log.LogInformation("Pipe {Pipe} closed", options.PipeName);
        }
    }

    private void StartSession(NamedPipeServerStream pipe, CancellationToken stoppingToken)
    {
        int id = Interlocked.Increment(ref _nextClientId);
        log.LogDebug("Pipe client {Client} connected", id);
        var session = new ClientSession(pipe, feed, pawnIo, options, log, id);

        // Registered before the session runs, so a session that ends at once is still removed.
        var ended = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        _sessions[id] = ended.Task;
        _ = Task.Run(
            async () =>
            {
                try
                {
                    await session.RunAsync(stoppingToken).ConfigureAwait(false);
                }
                finally
                {
                    try
                    {
                        // The instance goes first, so the accept loop can create a new one in its place.
                        pipe.Dispose();
                        Interlocked.Decrement(ref _busy);
                        idle.ClientDisconnected();
                        log.LogDebug("Pipe client {Client} closed", id);
                    }
                    catch (Exception e)
                    {
                        log.LogError(e, "Releasing pipe client {Client} failed", id);
                    }
                    finally
                    {
                        _sessions.TryRemove(id, out _);
                        _slotFreed.Release();
                        ended.SetResult();
                    }
                }
            },
            CancellationToken.None);
    }

    /// <summary>A further listening instance, or <see langword="null"/> (retried after <see cref="CreateRetryDelay"/>) if it cannot be created now.</summary>
    private NamedPipeServerStream? TryCreateNext()
    {
        try
        {
            NamedPipeServerStream next = Wrap(PipeNative.CreateInstance(options.PipeName, first: false, options.MaxClients, options.SecurityDescriptorSddl));
            _createFailed = false;
            if (_loggedCreateError is not null)
            {
                log.LogInformation("Creating instances of pipe {Pipe} works again", options.PipeName);
                _loggedCreateError = null;
            }

            return next;
        }
        catch (Win32Exception e)
        {
            // Busy (231) is not expected below the limit, but is retried like anything else rather
            // than waiting for a session to end. Logged once per error code.
            _createFailed = true;
            if (e.NativeErrorCode != _loggedCreateError)
            {
                log.LogWarning(
                    "Creating a further instance of pipe {Pipe} failed (Win32 error {Error}); retrying",
                    options.PipeName,
                    e.NativeErrorCode);
                _loggedCreateError = e.NativeErrorCode;
            }

            return null;
        }
    }

    /// <summary>Disconnects a failed listening instance to reuse it; replaces it (or leaves the retry to the loop) if that fails.</summary>
    private NamedPipeServerStream? Reset(NamedPipeServerStream failed)
    {
        try
        {
            failed.Disconnect();
            return failed;
        }
        catch (Exception e) when (e is IOException or InvalidOperationException)
        {
            log.LogDebug(e, "Resetting a pipe instance failed; replacing it");
            failed.Dispose();
            return TryCreateNext();
        }
    }

    private static NamedPipeServerStream Wrap(SafePipeHandle handle)
    {
        try
        {
            return new NamedPipeServerStream(PipeDirection.InOut, isAsync: true, isConnected: false, handle);
        }
        catch
        {
            handle.Dispose();
            throw;
        }
    }
}
