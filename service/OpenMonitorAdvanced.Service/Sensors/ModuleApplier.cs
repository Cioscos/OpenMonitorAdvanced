using Microsoft.Extensions.Logging;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>Where a subscribers' request stands (the schema's <c>service.reconfiguration</c>).</summary>
internal enum ReconfigurationStatus
{
    Applied,
    Pending,
    Failed,
}

/// <summary>
/// The handshake that parks the storage worker at the boundary of its loop before the sampler
/// flips an LHM setter (spec M5 §2.8, F2.3): no group is added or closed while the storage worker
/// may be inside LHM. Three counters, each written by one thread only, so plain
/// <see cref="Volatile"/> reads and writes suffice and no lock is ever held across hardware I/O:
/// the sampler raises <c>request</c>, the worker copies it to <c>ack</c> and waits (in its loop's
/// existing wake wait) until the sampler raises <c>release</c> to match.
/// </summary>
internal sealed class StoragePark
{
    private long _request; // sampler
    private long _ack; // storage worker
    private long _release; // sampler

    /// <summary>Whether the worker acknowledged a park that has not been released yet.</summary>
    public bool IsParked => Volatile.Read(ref _ack) > Volatile.Read(ref _release);

    /// <summary>Sampler: asks for a park unless one is outstanding. Returns whether a new one was asked (wake the worker).</summary>
    public bool Request()
    {
        long request = Volatile.Read(ref _request);
        if (request != Volatile.Read(ref _release))
        {
            return false;
        }

        Volatile.Write(ref _request, request + 1);
        return true;
    }

    /// <summary>Sampler: whether the worker acknowledged the outstanding request (it is parked until <see cref="Release"/>).</summary>
    public bool Acknowledged
    {
        get
        {
            long request = Volatile.Read(ref _request);
            return request != Volatile.Read(ref _release) && Volatile.Read(ref _ack) == request;
        }
    }

    /// <summary>Sampler: releases the outstanding request, acknowledged or not. Returns whether there was one (wake the worker).</summary>
    public bool Release()
    {
        long request = Volatile.Read(ref _request);
        if (request == Volatile.Read(ref _release))
        {
            return false;
        }

        Volatile.Write(ref _release, request);
        return true;
    }

    /// <summary>
    /// Storage worker, at the boundary of its loop: acknowledges a new request
    /// (<paramref name="acknowledged"/>: wake the sampler) and returns whether it must stay parked.
    /// </summary>
    public bool Park(out bool acknowledged)
    {
        long request = Volatile.Read(ref _request);
        acknowledged = request > Volatile.Read(ref _ack);
        if (acknowledged)
        {
            Volatile.Write(ref _ack, request);
        }

        return Volatile.Read(ref _release) < Volatile.Read(ref _ack);
    }
}

/// <summary>
/// The sampler's side of applying a request to the LHM groups other than storage (spec M5 §2.8,
/// F2.3): it asks the storage worker to park without waiting for it, and once the park is
/// acknowledged flips the setters (<see cref="IHardwareTree.SetModules"/>) and releases the worker.
/// A request that is not done within <see cref="Timeout"/> (the storage worker stuck in a driver
/// call) or whose setters throw is <see cref="ReconfigurationStatus.Failed"/>, with one warning per
/// request, and never forced. A park that is not acknowledged is asked again on every step (it is
/// cheap); setters that threw are tried again, with a new park, only after
/// <see cref="SensorHub.FailureRetryDelay"/> for the same request, since a throwing setter can cost
/// seconds of sampler time per call. A new request is tried at once. Sampler-owned: every member
/// runs on <c>oma-sampler</c>.
/// </summary>
internal sealed class ModuleApplier(IHardwareTree tree, StoragePark park, TimeProvider time, ILogger log, Action wakeStorage)
{
    private long _failedVersion; // the request already reported failed (and warned about)
    private long _appliedVersion;
    private long _threwVersion; // the request whose setters threw last, at _threwAt
    private long _threwAt;

    /// <summary>How long a request may stay pending before it is <see cref="ReconfigurationStatus.Failed"/>.</summary>
    public TimeSpan Timeout { get; set; } = TimeSpan.FromSeconds(15);

    /// <summary>Whether the tree is open (before that, <see cref="Open"/> applies the request by construction).</summary>
    public bool IsOpen { get; private set; }

    /// <summary>The groups of <see cref="HardwareModules.TreeGroups"/> open in the tree.</summary>
    public ServiceModules Groups { get; private set; }

    /// <summary>The tree was opened with <paramref name="groups"/>.</summary>
    public void Open(ServiceModules groups)
    {
        Groups = groups & HardwareModules.TreeGroups;
        IsOpen = true;
    }

    /// <summary>
    /// One step for <paramref name="desired"/>; <paramref name="storageApplied"/> says whether the
    /// storage worker already took its storage part. Returns the request's status.
    /// </summary>
    public ReconfigurationStatus Step(SensorHub.DesiredConfig desired, bool storageApplied)
    {
        bool groupsDone = !IsOpen || StepGroups(desired);
        if (groupsDone && storageApplied)
        {
            if (_appliedVersion != desired.Version)
            {
                _appliedVersion = desired.Version;
                log.LogInformation("Service reconfiguration {Version} applied: {Modules}", desired.Version, desired.Config.Enabled);
            }

            return ReconfigurationStatus.Applied;
        }

        if (_failedVersion == desired.Version)
        {
            return ReconfigurationStatus.Failed;
        }

        if (time.GetElapsedTime(desired.RequestedAt) > Timeout)
        {
            Fail(desired, e: null);
            return ReconfigurationStatus.Failed;
        }

        return ReconfigurationStatus.Pending;
    }

    /// <summary>Whether the tree's groups match the request after this step.</summary>
    private bool StepGroups(SensorHub.DesiredConfig desired)
    {
        ServiceModules wanted = desired.Config.Enabled & HardwareModules.TreeGroups;
        if (wanted == Groups)
        {
            // Nothing to flip (maybe no longer: the request changed back): a park asked for an
            // older request is not needed any more.
            if (park.Release())
            {
                wakeStorage();
            }

            return true;
        }

        if (_threwVersion == desired.Version && time.GetElapsedTime(_threwAt) < SensorHub.FailureRetryDelay)
        {
            return false; // stays failed until the delay is over
        }

        if (!park.Acknowledged)
        {
            if (park.Request())
            {
                wakeStorage();
            }

            return false; // sampling goes on; the worker wakes the sampler when it parks
        }

        try
        {
            tree.SetModules(wanted);
            Groups = wanted;
        }
        catch (Exception e)
        {
            // Retried with a new park after the delay; the setters that did run are kept by LHM.
            _threwVersion = desired.Version;
            _threwAt = time.GetTimestamp();
            Fail(desired, e);
        }
        finally
        {
            park.Release();
            wakeStorage();
        }

        return Groups == wanted;
    }

    private void Fail(SensorHub.DesiredConfig desired, Exception? e)
    {
        if (_failedVersion == desired.Version)
        {
            return;
        }

        _failedVersion = desired.Version;
        if (e is null)
        {
            log.LogWarning(
                "Service reconfiguration {Version} did not complete within {Seconds} s (the storage worker has not reached a safe point); it stays failed and is retried on every tick",
                desired.Version,
                Timeout.TotalSeconds);
        }
        else
        {
            log.LogWarning(
                e,
                "Service reconfiguration {Version} failed while switching LibreHardwareMonitor groups; it stays failed and is retried every {Seconds} s",
                desired.Version,
                SensorHub.FailureRetryDelay.TotalSeconds);
        }
    }
}
