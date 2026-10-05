using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Frames;

/// <summary>
/// The frame requests of the connected pipe clients (spec M7b §4.1), combined: the capture runs
/// while any session asks for it, with the PC latency and GPU options of all of them in OR, and
/// the targets are the distinct PIDs the sessions follow. A session's request stays valid for
/// <see cref="Grace"/> after it disconnects, so an app that reconnects (a service restart, a
/// reload) does not stop and restart PresentMon. Expiry is evaluated lazily against the injected
/// clock; the owner re-reads <see cref="Effective"/> once <see cref="NextExpiry"/> has passed.
/// Thread safe: one lock guards all state.
/// </summary>
internal sealed class FrameRequests(TimeProvider time)
{
    internal static readonly TimeSpan Grace = TimeSpan.FromSeconds(30);

    private readonly Lock _gate = new();
    private readonly Dictionary<int, Entry> _sessions = [];

    /// <summary><see langword="null"/> when no live session has frames enabled; otherwise the options in OR.</summary>
    public FramesOptions? Effective
    {
        get
        {
            lock (_gate)
            {
                Expire();
                bool any = false, pcl = false, gpu = false;
                foreach (Entry entry in _sessions.Values)
                {
                    if (entry.Config is { Enabled: true } c)
                    {
                        any = true;
                        pcl |= c.TrackPcLatency;
                        gpu |= c.TrackGpu;
                    }
                }

                return any ? new FramesOptions(pcl, gpu) : null;
            }
        }
    }

    /// <summary>The distinct valid PIDs the live sessions follow.</summary>
    public IReadOnlyCollection<uint> Targets
    {
        get
        {
            lock (_gate)
            {
                Expire();
                return _sessions.Values.Where(e => e.Pid is not null).Select(e => e.Pid!.Value).Distinct().ToArray();
            }
        }
    }

    /// <summary>Time until the first disconnected session expires, or <see langword="null"/> if none is pending.</summary>
    public TimeSpan? NextExpiry
    {
        get
        {
            lock (_gate)
            {
                Expire();
                DateTimeOffset now = time.GetUtcNow();
                TimeSpan? next = null;
                foreach (Entry entry in _sessions.Values)
                {
                    if (entry.DisconnectedAt is { } at)
                    {
                        TimeSpan left = at + Grace - now;
                        if (next is null || left < next)
                        {
                            next = left;
                        }
                    }
                }

                return next;
            }
        }
    }

    /// <summary>PIDs 0 (Idle) and 4 (System) never present: they count as no target.</summary>
    public static bool IsValidPid(uint pid) => pid is not 0 and not 4;

    public void Configure(int session, FramesConfigureMessage message)
    {
        lock (_gate)
        {
            EntryOf(session).Config = message;
        }
    }

    /// <summary>Sets the session's target; an invalid PID (see <see cref="IsValidPid"/>) clears it.</summary>
    public void Target(int session, uint? pid)
    {
        lock (_gate)
        {
            EntryOf(session).Pid = pid is { } p && IsValidPid(p) ? p : null;
        }
    }

    /// <summary>The session's target, or <see langword="null"/> (none, or the session expired).</summary>
    public uint? TargetOf(int session)
    {
        lock (_gate)
        {
            Expire();
            return _sessions.TryGetValue(session, out Entry? entry) ? entry.Pid : null;
        }
    }

    /// <summary>Starts the grace of the session's request (nothing to do for a session that never asked).</summary>
    public void Disconnected(int session)
    {
        lock (_gate)
        {
            if (_sessions.TryGetValue(session, out Entry? entry) && entry.DisconnectedAt is null)
            {
                entry.DisconnectedAt = time.GetUtcNow();
            }
        }
    }

    private Entry EntryOf(int session)
    {
        if (!_sessions.TryGetValue(session, out Entry? entry))
        {
            entry = new Entry();
            _sessions[session] = entry;
        }

        return entry;
    }

    private void Expire()
    {
        // Read on every hub tick: removing while enumerating is allowed for Dictionary, so no copy.
        DateTimeOffset now = time.GetUtcNow();
        foreach (var (session, entry) in _sessions)
        {
            if (entry.DisconnectedAt is { } at && now - at >= Grace)
            {
                _sessions.Remove(session);
            }
        }
    }

    private sealed class Entry
    {
        public FramesConfigureMessage? Config;
        public uint? Pid;
        public DateTimeOffset? DisconnectedAt;
    }
}
