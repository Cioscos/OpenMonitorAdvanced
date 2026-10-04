using System.Collections.Concurrent;
using System.Diagnostics;
using LibreHardwareMonitor.Hardware;
using Microsoft.Extensions.Logging;
using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Sensors;

public sealed partial class SensorHub
{
    /// <summary>
    /// One storage round: the D6 gate or, once it is open, the drive list with one power check
    /// per drive; then for every disk LHM exposes its identity (first touch only) and its update;
    /// then one new <see cref="StorageRound"/>.
    /// </summary>
    internal void StorageOnce()
    {
        SyncStorageConfig();
        if (!_opened || IsDisposed)
        {
            return;
        }

        EffectiveConfig config = _storageApplied;
        if (!config.Enabled.HasFlag(ServiceModules.Storage))
        {
            return; // switched off softly (P10): no gate, no enumeration, no description, no power check, no update
        }

        long roundStart = _time.GetTimestamp();
        StorageRound before = _round;
        bool keepable = IsCurrent(before, roundStart); // an expired round has nothing to carry over
        RearmAfterAnExpiredRound(late: !keepable && before.Timestamp != long.MinValue, roundStart);
        IReadOnlyList<DriveCheck>? checks = _storageEnabled ? CheckPowerStates(config) : TryEnableStorage(config, roundStart);
        if (checks is null)
        {
            PublishApplied(); // no round to carry it: the request is taken all the same
            return;
        }

        // The round's one answer per drive: it decides the update here and the state in the list.
        var answers = new Dictionary<int, DriveCheck>(checks.Count);
        foreach (DriveCheck check in checks)
        {
            answers[check.Drive.DriveNumber] = check;
        }

        IReadOnlyDictionary<string, DiskResolution> previous = before.Resolved;
        var resolved = new Dictionary<string, DiskResolution>(StringComparer.Ordinal);
        var cache = new Dictionary<string, double?>(StringComparer.Ordinal);
        var held = new HashSet<string>(StringComparer.Ordinal);
        IReadOnlyList<HardwareNode> roots = _tree.Roots;
        var identifierCounts = new Dictionary<string, int>(StringComparer.Ordinal);
        foreach (HardwareNode root in roots)
        {
            if (root.Type == HardwareType.Storage)
            {
                identifierCounts[root.Identifier] = identifierCounts.GetValueOrDefault(root.Identifier) + 1;
            }
        }

        foreach (HardwareNode root in roots)
        {
            if (root.Type != HardwareType.Storage)
            {
                continue;
            }

            if (_stop.IsCancellationRequested)
            {
                return;
            }

            if (identifierCounts[root.Identifier] > 1)
            {
                // Its identifier (and so its sensor identifiers, and every identifier-keyed map
                // here) is shared with another disk: never described, updated or published.
                LogNotUnique(root.Identifier);
                continue;
            }

            DiskResolution? resolution = Resolve(root, previous);
            if (resolution is null)
            {
                continue; // identity unknown: not published, not touched, retried next round
            }

            resolved[root.Identifier] = resolution;
            if (DriveStates.IsSmartOff(resolution.Facts, resolution.Key, config))
            {
                continue; // SMART off for this disk (P7, or off by default): no update, not in the schema
            }

            StorageInfo info = resolution.Info;
            if (resolution.Facts.Availability == DriveAvailability.NoMedia)
            {
                // LHM enumerated this disk, so "not ready / no media" cannot mean "no platter to
                // wake" (a USB bridge may answer so while its disk sleeps): it cannot be
                // confirmed active, so it is not updated.
                LogDiskStateChange(root.Identifier, info.DriveNumber, spunDown: null);
                continue;
            }

            if (info.Rotational)
            {
                // A drive the round did not check (gone from the enumeration, say) is unknown.
                answers.TryGetValue(info.DriveNumber, out DriveCheck? answer);
                bool idle = answer is { Idle: true };
                bool? spunDown = answer switch
                {
                    { PoweredOff: true } => true,
                    { Asked: true } => answer.SpunDown,
                    _ => null,
                };
                if (!idle)
                {
                    LogDiskStateChange(root.Identifier, info.DriveNumber, spunDown); // idle comes and goes with the disk's work: not logged
                }

                if (idle || spunDown != false)
                {
                    // Left alone, asleep or unknown: no SMART read. A disk that rests (Windows
                    // turned it off, it is idle, or it confirmed its standby) keeps the values
                    // of the round before; an unknown state leaves them absent.
                    if (answer is { Rests: true } && keepable)
                    {
                        KeepValues(root, resolution, answer, before, cache, held);
                    }

                    continue;
                }
            }

            try
            {
                _tree.Update(root);
            }
            catch (Exception e)
            {
                LogRateLimited(root.Identifier, e, "Updating {Root} failed; its values are absent until the next storage round", root.Identifier);
                continue;
            }

            // Re-fetched: the update may have activated sensors (a new node for this root).
            HardwareNode fresh = FindRoot(root.Identifier) ?? root;
            CollectValues(fresh, cache);
            resolved[root.Identifier] = CollectCriticalWarning(fresh, resolution, cache);
        }

        TakeReference();

        // Never mutated after publication: the sampler only looks them up.
        PublishRound(roundStart, DriveStates.ToWire(checks, config, gateOpen: true), resolved, cache, held, config, keptFrom: before);
    }

    /// <summary>
    /// The end of a round's disk work, on every path that completes it (a round that asked
    /// nothing and a round of the closed gate too): the counters the next round compares its
    /// sample with, read after this round's last question and last SMART read, so that the
    /// service's own traffic is never taken for the disk's activity, and before the round is
    /// published. Access 0, no command, no lock. A round that fails before this point leaves
    /// no reference, and the next one sees no activity.
    /// </summary>
    private void TakeReference()
    {
        try
        {
            _watch.TakeBaseline();
        }
        catch (Exception e)
        {
            LogRateLimited("storage-reference", e, "Reading the disk counters failed; no disk activity is seen in the next storage round");
        }
    }

    /// <summary>
    /// A round whose predecessor has expired (a suspension and resume, a stalled worker: see
    /// <see cref="IsCurrent"/>) keeps nothing for a resting disk, so a spinning but quiet disk
    /// would show no SMART value until it next works. Such a round starts a storage episode:
    /// every watched drive is first watched again (<see cref="ActivityWatch.Rearm"/>), before
    /// the drives are listed and so before any question, and each one Windows reports on is
    /// asked once, as after the hub was idle. One question per drive and expiry: of rounds that
    /// are late one after the other (a worker that never keeps up with
    /// <see cref="StorageInterval"/>) only the first re-arms, then one every
    /// <see cref="GateEpisode.BlindRetry"/>, since a question at every round would keep Windows
    /// from ever turning the disk off. A round with no round before it (the first one, storage
    /// switched on, a client after an idle hub) is an episode start already and not a late
    /// round. With the D6 gate closed the gate's own rule decides.
    /// </summary>
    private void RearmAfterAnExpiredRound(bool late, long roundStart)
    {
        bool consecutive = _lateRound;
        _lateRound = late;
        if (!late || !_storageEnabled)
        {
            return;
        }

        if (consecutive && _time.GetElapsedTime(_rearmedAt, roundStart) < GateEpisode.BlindRetry)
        {
            return;
        }

        _rearmedAt = roundStart;
        _watch.Rearm();
    }

    /// <summary>
    /// A resting disk's values of the round before, carried into this one as held (the caller
    /// has checked that round is still current, <see cref="IsCurrent"/>). Only those
    /// measured on this very disk: the earlier round's disk, the resolved one and the drive this
    /// round enumerated at its number must share one <see cref="DriveKey"/> (the same LHM
    /// identifier or drive number alone proves nothing). Absent and non-finite values are not kept.
    /// </summary>
    private static void KeepValues(
        HardwareNode root,
        DiskResolution resolution,
        DriveCheck answer,
        StorageRound before,
        Dictionary<string, double?> cache,
        HashSet<string> held)
    {
        if (resolution.Key is not { } key
            || answer.Key != key
            || !before.Resolved.TryGetValue(root.Identifier, out DiskResolution? measured)
            || measured.Key != key
            || measured.Info.DriveNumber != resolution.Info.DriveNumber)
        {
            return;
        }

        Keep(root.Identifier); // the critical warning flag is keyed by the disk itself
        KeepSensors(root);

        void KeepSensors(HardwareNode node)
        {
            foreach (SensorNode sensor in node.Sensors)
            {
                Keep(sensor.Identifier);
            }

            foreach (HardwareNode child in node.Children)
            {
                KeepSensors(child);
            }
        }

        void Keep(string identifier)
        {
            if (before.Values.TryGetValue(identifier, out double? value) && value is double v && double.IsFinite(v))
            {
                cache[identifier] = v;
                held.Add(identifier);
            }
        }
    }

    /// <summary>
    /// The one place a <see cref="StorageRound"/> is published: a single reference swap, with no
    /// lock. <see langword="null"/> keeps the drives or the resolved disks as they are; unchanged
    /// ones keep their instance, which is how the sampler tells that nothing has to be rebuilt.
    /// The storage worker publishes at the end of a round (or when storage is switched off); only
    /// the values are dropped from another thread, when the last client leaves. Every publication
    /// is a new <see cref="StorageRound.Generation"/>, also when it measured the same values again.
    /// <paramref name="applied"/> is the storage part the round was made with
    /// (<see langword="null"/> keeps it), so it reaches the sampler together with the round.
    /// <paramref name="keptFrom"/> is the round the <paramref name="held"/> values were taken
    /// from: if it is no longer the published one (the values were dropped meanwhile, when the
    /// last client left), they are not published.
    /// </summary>
    private void PublishRound(
        long timestamp,
        IReadOnlyList<WireDrive>? drives,
        IReadOnlyDictionary<string, DiskResolution>? resolved,
        IReadOnlyDictionary<string, double?> values,
        IReadOnlySet<string> held,
        EffectiveConfig? applied,
        StorageRound? keptFrom = null)
    {
        StorageRound current;
        StorageRound next;
        do
        {
            current = _round;
            if (held.Count > 0 && !ReferenceEquals(current, keptFrom))
            {
                values = values.Where(pair => !held.Contains(pair.Key)).ToDictionary(StringComparer.Ordinal);
                held = StorageRound.Empty.Held;
            }

            next = new StorageRound(
                current.Generation + 1,
                timestamp,
                drives is null || drives.SequenceEqual(current.Drives) ? current.Drives : drives,
                resolved is null || SameResolution(current.Resolved, resolved) ? current.Resolved : resolved,
                values,
                held,
                applied ?? current.Applied);
        }
        while (Interlocked.CompareExchange(ref _round, next, current) != current);
    }

    /// <summary>
    /// Storage worker, when it took a request but no round can carry it (the tree is not open
    /// yet, or the drives could not be listed): the current round with the applied storage part,
    /// and nothing else, replaced. Not a new generation: nothing was measured.
    /// </summary>
    private void PublishApplied()
    {
        EffectiveConfig applied = _storageApplied;
        StorageRound current;
        do
        {
            current = _round;
            if (current.Applied.Equals(applied))
            {
                return;
            }
        }
        while (Interlocked.CompareExchange(ref _round, current with { Applied = applied }, current) != current);
    }

    /// <summary>
    /// One wake of the storage loop: a storage round when due (every 30 s, only with a subscriber,
    /// only once open). Returns the delay until the next wake.
    /// </summary>
    internal TimeSpan RunStorageDue()
    {
        if (IsDisposed)
        {
            return Timeout.InfiniteTimeSpan;
        }

        // The boundary of the loop: the storage part of a request is taken here, and here only the
        // worker parks for the sampler's setters, doing no I/O until released (the loop's wait
        // ends on the release's wake or on Dispose).
        SyncStorageConfig();
        bool parked = _park.Park(out bool acknowledged);
        if (acknowledged)
        {
            SetQuietly(_samplerWake);
        }

        if (parked)
        {
            return Timeout.InfiniteTimeSpan;
        }

        long now = _time.GetTimestamp();
        long due;
        lock (_subLock)
        {
            if (!_opened || _subscribers.Count == 0)
            {
                return Timeout.InfiniteTimeSpan;
            }

            due = _nextStorageDue;
        }

        if (now >= due)
        {
            StorageOnce();
            lock (_subLock)
            {
                due = _nextStorageDue = now + SecondsToTicks(StorageInterval);
            }
        }

        return TicksUntil(due);
    }

    private HardwareNode? FindRoot(string identifier)
    {
        foreach (HardwareNode root in _tree.Roots)
        {
            if (root.Identifier == identifier)
            {
                return root;
            }
        }

        return null;
    }

    /// <summary>Sampler: the values of the tick's storage round, unless they are too old to be current.</summary>
    private IReadOnlyDictionary<string, double?> FreshStorageValues(long now) =>
        IsCurrent(_view, now) ? _view.Values : StorageRound.Empty.Values;

    /// <summary>
    /// Whether <paramref name="round"/> has values that still count at <paramref name="now"/>:
    /// it started at most two rounds ago. The one limit for what the sampler publishes and for
    /// what the next round may keep for a sleeping disk.
    /// </summary>
    private bool IsCurrent(StorageRound round, long now) =>
        round.Timestamp != long.MinValue && now - round.Timestamp <= 2 * SecondsToTicks(StorageInterval);

    /// <summary>
    /// The D6 gate, on every storage round until LHM's storage group is enabled: one
    /// <see cref="GateEpisode"/> round over a fresh enumeration. Returns the gate's checks once
    /// it is open: the round goes on with those answers, asking no drive twice. Otherwise
    /// <see langword="null"/>, after publishing the drive list the gate saw.
    /// </summary>
    private IReadOnlyList<DriveCheck>? TryEnableStorage(EffectiveConfig config, long roundStart)
    {
        if (ListDrives(drive => drive.RequiresPowerCheck) is not { } listed)
        {
            return null;
        }

        _gate ??= new GateEpisode(_time, _log);
        IReadOnlyList<DriveCheck> checks = _gate.Round(listed.Drives, listed.Activity, AskPowerMode);
        if (!_gate.Opens)
        {
            // A drive blocks, or an earlier attempt to enable the group failed and is not due again.
            TakeReference();
            PublishRound(roundStart, DriveStates.ToWire(checks, config, gateOpen: false), resolved: null, StorageRound.Empty.Values, StorageRound.Empty.Held, config);
            if (!_storageGateLogged && checks.Any(c => c.Blocks))
            {
                _storageGateLogged = true;
                _log.LogInformation("Storage stays disabled: a rotational disk is in standby or its state is unknown (asked again when it shows activity)");
            }

            return null;
        }

        try
        {
            _tree.EnableStorage();
        }
        catch (Exception e)
        {
            LogRateLimited("storage-enable", e, "Enabling storage failed; tried again in {Minutes} minutes", GateEpisode.BlindRetry.TotalMinutes);
            _gate.EnableFailed();

            // No drive blocks: the list says so. The gate keeps its answers until the next attempt.
            TakeReference();
            PublishRound(roundStart, DriveStates.ToWire(checks, config, gateOpen: false), resolved: null, StorageRound.Empty.Values, StorageRound.Empty.Held, config);
            return null;
        }

        _storageEnabled = true;
        _gate = null;
        _log.LogInformation("Every rotational disk is active: storage enabled");
        return checks;
    }

    /// <summary>
    /// Storage worker, once the gate is open: every drive of a fresh enumeration (access 0),
    /// whether LHM exposes it or not. A drive whose power mode matters and whose SMART is on is
    /// asked only by the rule of <see cref="DiskActivity.Check"/>: not when Windows turned it
    /// off, and not without recent activity, except the one time it is first watched
    /// (<see cref="ActivityWatch"/>). A drive whose SMART is off is not even sampled: nothing
    /// is sent to it periodically. <see langword="null"/> when the drives cannot be listed: no
    /// disk is updated blind.
    /// </summary>
    private IReadOnlyList<DriveCheck>? CheckPowerStates(EffectiveConfig config)
    {
        if (ListDrives(drive => drive.RequiresPowerCheck && !DriveStates.IsSmartOff(drive, DriveKey.Compute(drive.Model, drive.Serial), config)) is not { } listed)
        {
            return null;
        }

        var checks = new List<DriveCheck>(listed.Drives.Count);
        foreach (DriveFacts drive in listed.Drives)
        {
            if (_stop.IsCancellationRequested)
            {
                return null;
            }

            checks.Add(listed.Activity.TryGetValue(drive.DriveNumber, out DriveActivity seen)
                ? DiskActivity.Check(drive, seen, AskPowerMode)
                : new DriveCheck(drive, Asked: false, SpunDown: null));
        }

        return checks;
    }

    /// <summary>
    /// A fresh enumeration (access 0) with what the passive sources say about each drive
    /// <paramref name="watched"/> selects, sampled before any command is sent and compared
    /// with the reference the round before took at its end (<see cref="TakeReference"/>).
    /// <see langword="null"/> when the drives cannot be listed.
    /// </summary>
    private (IReadOnlyList<DriveFacts> Drives, IReadOnlyDictionary<int, DriveActivity> Activity)? ListDrives(Func<DriveFacts, bool> watched)
    {
        try
        {
            if (Interlocked.Exchange(ref _wentIdle, 0) == 1)
            {
                _watch.Clear(); // a storage episode starts: every drive is watched anew
            }

            IReadOnlyList<DriveFacts> drives = _disks.Enumerate();
            return (drives, _watch.Sample(drives, watched));
        }
        catch (Exception e)
        {
            LogRateLimited("storage-enumerate", e, "Listing the drives failed; no disk is touched in this storage round");
            return null;
        }
    }

    /// <summary>CHECK POWER MODE for that drive; a failure is an unknown state, for that drive only.</summary>
    private bool? AskPowerMode(DriveFacts drive)
    {
        try
        {
            return _disks.IsSpunDown(drive.DriveNumber, drive.Model, drive.Serial);
        }
        catch (Exception e)
        {
            LogRateLimited("power:" + drive.DriveNumber, e, "Checking the power mode of PhysicalDrive{Drive} failed", drive.DriveNumber);
            return null;
        }
    }

    /// <summary>
    /// Storage worker: takes the storage part of a new request. Switched off (P10), the values
    /// (kept ones too) and the resolved disks are dropped at once and the last drive list stays,
    /// every entry <c>smartOff</c>, with no I/O (no counter read either) and the LHM group left open; switched on, or with
    /// another SMART selection, a round runs at once and publishes the request as applied.
    /// </summary>
    private void SyncStorageConfig()
    {
        DesiredConfig? desired = _desired;
        if (desired is null || desired.Version == _storageSyncedVersion)
        {
            return;
        }

        _storageSyncedVersion = desired.Version;
        EffectiveConfig part = desired.Config.StoragePart;
        if (part.Equals(_storageApplied))
        {
            return;
        }

        _storageApplied = part;
        if (!part.Enabled.HasFlag(ServiceModules.Storage))
        {
            _watch.Clear();
            PublishRound(long.MinValue, DriveStates.AllOff(_round.Drives), StorageRound.Empty.Resolved, StorageRound.Empty.Values, StorageRound.Empty.Held, part);
        }
        else
        {
            // Reported as applied by the round that is made with it, together with its drive
            // list; before the tree is open there is no round to wait for.
            bool opened;
            lock (_subLock)
            {
                _nextStorageDue = _time.GetTimestamp();
                opened = _opened;
            }

            if (!opened)
            {
                PublishApplied();
            }
        }

        SetQuietly(_samplerWake); // the sampler reports what was published here; a round's part goes out with the next sample
    }

    /// <summary>
    /// The NVMe critical warning of a just-updated disk as a 0/1 flag under the disk's own
    /// identifier (its binding's key), read from the SMART attributes that same update left
    /// (no extra command). Returns the resolution with the disk's current NVMe flags: the first
    /// update is what makes the attribute exist, and that changes the schema.
    /// </summary>
    private DiskResolution CollectCriticalWarning(HardwareNode fresh, DiskResolution resolution, Dictionary<string, double?> cache)
    {
        StorageInfo? current = fresh.Storage;
        if (current is null)
        {
            return resolution;
        }

        if (current is { IsNvme: true, HasCriticalWarning: true })
        {
            byte? raw;
            try
            {
                raw = _tree.ReadNvmeCriticalWarning(fresh.Identifier);
            }
            catch (Exception e)
            {
                LogRateLimited("critical-warning:" + fresh.Identifier, e, "Reading the critical warning of {Root} failed", fresh.Identifier);
                raw = null;
            }

            cache[fresh.Identifier] = raw is byte b ? ((b & CriticalWarningMask) != 0 ? 1.0 : 0.0) : null;
        }

        StorageInfo info = resolution.Info;
        return info.IsNvme == current.IsNvme && info.HasCriticalWarning == current.HasCriticalWarning
            ? resolution
            : resolution with { Info = info with { IsNvme = current.IsNvme, HasCriticalWarning = current.HasCriticalWarning } };
    }

    private void CollectValues(HardwareNode node, Dictionary<string, double?> cache)
    {
        foreach (SensorNode sensor in node.Sensors)
        {
            cache[sensor.Identifier] = _tree.Read(sensor.Identifier);
        }

        foreach (HardwareNode child in node.Children)
        {
            CollectValues(child, cache);
        }
    }

    private void LogDiskStateChange(string root, int drive, bool? spunDown)
    {
        if (_diskStates.TryGetValue(root, out bool? previous) && previous == spunDown)
        {
            return;
        }

        _diskStates[root] = spunDown;
        string state = spunDown switch
        {
            true => "in standby: SMART skipped",
            false => "active",
            null => "of unknown power state: SMART skipped",
        };
        _log.LogInformation("{Root} (PhysicalDrive{Drive}) is {State}", root, drive, state);
    }

    /// <summary>
    /// A disk's resolved identity, with the <paramref name="Facts"/> it was described by; only a
    /// <paramref name="Complete"/> one is reused (and its device id pinned).
    /// </summary>
    private sealed record DiskResolution(StorageInfo Info, DriveFacts Facts, bool Complete)
    {
        /// <summary>Its <see cref="DriveKey"/> from the descriptor model and serial; <see langword="null"/> without them.</summary>
        public string? Key { get; } = DriveKey.Compute(Info.DescriptorModel, Info.DescriptorSerial);
    }

    /// <summary>
    /// What the storage worker knows after one round, published as a whole: the state of every
    /// drive, the disks resolved for the schema (by LHM identifier) and the raw values (by LHM
    /// sensor identifier) with the round's monotonic start (<see cref="long.MinValue"/>: no
    /// values). <paramref name="Held"/> names the values kept from an earlier round for a
    /// sleeping disk instead of measured in this one; <paramref name="Applied"/> is the storage
    /// part of the request the round was made with. <paramref name="Generation"/> grows by one
    /// per round, which is how the sampler tells a new measurement from one it already published.
    /// Never mutated.
    /// </summary>
    private sealed record StorageRound(
        long Generation,
        long Timestamp,
        IReadOnlyList<WireDrive> Drives,
        IReadOnlyDictionary<string, DiskResolution> Resolved,
        IReadOnlyDictionary<string, double?> Values,
        IReadOnlySet<string> Held,
        EffectiveConfig Applied)
    {
        public static StorageRound Empty { get; } = new(
            0,
            long.MinValue,
            [],
            new Dictionary<string, DiskResolution>(),
            new Dictionary<string, double?>(),
            new HashSet<string>(),
            EffectiveConfig.AllOn.StoragePart);
    }
}
