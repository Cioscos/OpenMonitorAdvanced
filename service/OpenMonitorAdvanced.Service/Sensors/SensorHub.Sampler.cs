using System.Collections.Concurrent;
using System.Diagnostics;
using LibreHardwareMonitor.Hardware;
using Microsoft.Extensions.Logging;
using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Sensors;

public sealed partial class SensorHub
{
    /// <summary>
    /// One sampling tick: opens the tree the first time, updates every non-storage root, publishes a
    /// snapshot and delivers it to the due subscribers. With <paramref name="keepSchedule"/> (the
    /// extra sample for a new request) the sampling anchor and the next due time stay as they are.
    /// </summary>
    internal void TickOnce(bool keepSchedule = false)
    {
        if (IsDisposed)
        {
            return;
        }

        long tickStart = _time.GetTimestamp();
        ulong tickUnixMs = (ulong)_time.GetUtcNow().ToUnixTimeMilliseconds();

        // One storage round for the whole tick: its drive list goes into the service block, its
        // resolved disks into the schema and its values into the snapshot. A round published
        // while this tick runs is the next tick's, so its states never meet this one's values.
        _view = _round;
        ApplyDesired();
        Plan plan = _plan ?? OpenTree();
        if (IsDisposed)
        {
            return; // disposed from this very thread while it switched groups
        }

        // A request's schema effect (or a new service block, or the disks of a new storage round)
        // is built before the updates, so a switched-off group is not updated from this tick on
        // and this snapshot uses the new bindings.
        bool rebuildFailed = false;
        if (!plan.Filter.Equals(_schemaFilter)
            || !ReferenceEquals(plan.Resolved, _view.Resolved)
            || !SchemaComparer.SameServiceState(plan.Built.Schema.Service, _serviceState))
        {
            Interlocked.Exchange(ref _structureDirty, 0);
            plan = TryRebuildPlan(plan);
            rebuildFailed = _rebuildFailing;
        }

        _failedRoots.Clear();
        foreach (HardwareNode root in plan.UpdateRoots)
        {
            try
            {
                _tree.Update(root);
            }
            catch (Exception e)
            {
                _failedRoots.Add(root.Identifier);
                LogRateLimited(root.Identifier, e, "Updating {Root} failed; its sensors are absent from this snapshot", root.Identifier);
            }
        }

        // Rebuilt after the updates, so sensors LHM activates during an Update() are published
        // (with their value) in this very tick. A failed rebuild keeps the current devices and is
        // retried on the next tick.
        if (!rebuildFailed && Interlocked.Exchange(ref _structureDirty, 0) == 1)
        {
            plan = TryRebuildPlan(plan);
        }

        var values = new double?[plan.LhmIds.Length];
        var held = new bool[values.Length];
        IReadOnlyDictionary<string, double?> storage = FreshStorageValues(tickStart);

        // A new seq is not a new measurement: once a round's values went out, they are repeated.
        bool republished = _view.Generation == _publishedGeneration;
        bool anyFailed = _failedRoots.Count > 0;
        for (int i = 0; i < values.Length; i++)
        {
            double? raw;
            if (plan.FromStorage[i])
            {
                raw = storage.TryGetValue(plan.LhmIds[i], out double? cached) ? cached : null;
            }
            else if (anyFailed && plan.Owner[i] is { } owner && _failedRoots.Contains(owner))
            {
                raw = null;
            }
            else
            {
                raw = _tree.Read(plan.LhmIds[i]);
            }

            values[i] = raw is double r && double.IsFinite(r * plan.Scales[i]) ? r * plan.Scales[i] : null;
            held[i] = plan.FromStorage[i] && values[i] is not null && (republished || _view.Held.Contains(plan.LhmIds[i]));
        }

        _publishedGeneration = _view.Generation;

        long anchor;
        lock (_subLock)
        {
            if (keepSchedule && _sampled)
            {
                // An off-schedule sample for a new request: the phase every subscriber is on stays.
                anchor = _sampleAnchor;
            }
            else
            {
                anchor = ScheduleNextSampleLocked(tickStart);
            }
        }

        // The block the published schema carries reflects the served request only once the plan
        // was rebuilt with it (a failed rebuild keeps the older block, and the older version).
        if (SchemaComparer.SameServiceState(plan.Built.Schema.Service, _serviceState))
        {
            _reflectedVersion = _servedDesired?.Version ?? 0;
        }

        _seq++;
        _published = new Published(plan.Revision, plan.Built.Schema, new SnapshotMessage(_seq, tickUnixMs, values, held), anchor, _reflectedVersion);
        DeliverDue(_time.GetTimestamp());
    }

    /// <summary>
    /// Anchors this sample to the schedule when it is on time (or late by less than an interval);
    /// resynced to the actual start when early (a direct call) or further behind. Returns the anchor.
    /// </summary>
    private long ScheduleNextSampleLocked(long tickStart)
    {
        long scheduled = _nextSampleDue;
        bool onSchedule = scheduled != long.MinValue && scheduled <= tickStart && tickStart - scheduled < _minIntervalTicks;
        long anchor = onSchedule ? scheduled : tickStart;
        _sampleAnchor = anchor;
        _sampled = true;
        _nextSampleDue = anchor + _minIntervalTicks;
        return anchor;
    }

    /// <summary>One wake of the sampler loop: a sampling tick if one is due, otherwise only the due deliveries. Returns the delay until the next wake.</summary>
    internal TimeSpan RunDue()
    {
        if (IsDisposed)
        {
            return Timeout.InfiniteTimeSpan;
        }

        long now = _time.GetTimestamp();
        bool due;
        bool newRequest;
        lock (_subLock)
        {
            if (_subscribers.Count == 0)
            {
                return Timeout.InfiniteTimeSpan;
            }

            due = _plan is null || now >= _nextSampleDue;

            // A new desired configuration changes the schema's service block: sampled at once
            // (off schedule, without moving it), so the requesting client is answered with it.
            newRequest = !ReferenceEquals(_desired, _servedDesired);
        }

        if (due || newRequest)
        {
            TickOnce(keepSchedule: !due); // applies the desired configuration first
        }
        else
        {
            // Woken by the storage worker's park, a delivery deadline or a new subscriber: the
            // applier steps anyway; a new service block goes out with the next sample.
            ApplyDesired();
            DeliverDue(now);
        }

        lock (_subLock)
        {
            if (_subscribers.Count == 0)
            {
                return Timeout.InfiniteTimeSpan;
            }

            long wake = _nextSampleDue;
            foreach (Subscriber s in _subscribers)
            {
                if (s.RequestVersion > _reflectedVersion)
                {
                    continue; // waits for a sample that reflects its request (at the latest the next one)
                }

                wake = Math.Min(wake, s.NextDue);
            }

            return TicksUntil(wake);
        }
    }
}
