using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;

namespace OpenMonitorAdvanced.Service;

/// <summary>
/// Stops the host after <paramref name="idleAfter"/> (2 minutes in production) without any
/// connected pipe client, so the demand-started service does not linger once the app is gone
/// (spec §2.2). The countdown is armed by <see cref="Start"/> (the listener calls it once the pipe
/// exists), cancelled by the first client and restarted in full when the last one leaves. The stop
/// goes through <see cref="IHostApplicationLifetime.StopApplication"/>, so the process exits with
/// code 0.
/// </summary>
public sealed class IdleShutdown(IHostApplicationLifetime lifetime, TimeProvider time, TimeSpan idleAfter, ILogger<IdleShutdown>? log = null)
{
    private readonly Lock _gate = new();
    private int _clients;
    private bool _started;
    private bool _stopRequested;
    private ITimer? _timer;

    /// <summary>Incremented on every arm/disarm, so a timer callback that lost a race does nothing.</summary>
    private long _generation;

    /// <summary>Connected clients right now (test introspection).</summary>
    internal int ClientCount
    {
        get
        {
            lock (_gate)
            {
                return _clients;
            }
        }
    }

    /// <summary>Starts counting down if no client is connected yet. Idempotent.</summary>
    public void Start()
    {
        lock (_gate)
        {
            if (_started)
            {
                return;
            }

            _started = true;
            if (_clients == 0)
            {
                ArmLocked();
            }
        }
    }

    /// <summary>A client connected: the countdown stops until the last client leaves.</summary>
    public void ClientConnected()
    {
        lock (_gate)
        {
            _clients++;
            DisarmLocked();
        }
    }

    /// <summary>A client left; with none left the full countdown starts again.</summary>
    /// <exception cref="InvalidOperationException">More disconnections than connections (a release counted twice).</exception>
    public void ClientDisconnected()
    {
        lock (_gate)
        {
            if (_clients == 0)
            {
                throw new InvalidOperationException("ClientDisconnected without a matching ClientConnected");
            }

            _clients--;
            if (_clients == 0 && _started)
            {
                ArmLocked();
            }
        }
    }

    private void ArmLocked()
    {
        DisarmLocked();
        if (_stopRequested)
        {
            return;
        }

        long generation = _generation;
        _timer = time.CreateTimer(_ => OnElapsed(generation), null, idleAfter, Timeout.InfiniteTimeSpan);
    }

    private void DisarmLocked()
    {
        _generation++;
        _timer?.Dispose();
        _timer = null;
    }

    private void OnElapsed(long generation)
    {
        lock (_gate)
        {
            if (generation != _generation || _clients != 0 || _stopRequested)
            {
                return; // a client arrived (or the timer was replaced) after this one fired
            }

            _stopRequested = true;
            DisarmLocked();
        }

        log?.LogInformation("No pipe client for {Minutes} minutes: stopping the service", idleAfter.TotalMinutes);
        lifetime.StopApplication();
    }
}
