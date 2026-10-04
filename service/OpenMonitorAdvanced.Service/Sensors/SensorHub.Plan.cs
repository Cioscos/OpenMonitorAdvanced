using System.Collections.Concurrent;
using System.Diagnostics;
using LibreHardwareMonitor.Hardware;
using Microsoft.Extensions.Logging;
using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Sensors;

public sealed partial class SensorHub
{
    private Plan OpenTree()
    {
        long started = _time.GetTimestamp();

        // Only the requested groups are ever built; storage waits for the D6 gate.
        ServiceModules groups = (_servedDesired?.Config.Enabled ?? ServiceModules.All) & HardwareModules.TreeGroups;
        _tree.Open(groups);
        _applier.Open(groups);
        ApplyDesired();
        _pawnIo = _pawnIoAvailable();
        _log.LogInformation("PawnIO available: {PawnIo}", _pawnIo);

        // Changes raised during Open() are covered by the Roots read right after.
        Interlocked.Exchange(ref _structureDirty, 0);
        Plan plan = BuildPlan(_tree.Roots, current: null);
        _plan = plan;
        _log.LogInformation(
            "Hardware tree opened in {Elapsed} ms with {Groups}: {Roots} roots, {Devices} devices, {Sensors} sensors (storage deferred until every rotational disk is active)",
            (long)_time.GetElapsedTime(started).TotalMilliseconds,
            groups,
            plan.UpdateRoots.Length,
            plan.Built.Schema.Devices.Count,
            plan.Built.Schema.Sensors.Count);

        // Open() leaves tens of MB of garbage (s1-lhm.md §9.6): compact and decommit once.
        GC.Collect(2, GCCollectionMode.Aggressive, blocking: true, compacting: true);

        lock (_subLock)
        {
            _nextStorageDue = _time.GetTimestamp();
            _opened = true;
        }

        SetQuietly(_storageWake);
        return plan;
    }

    /// <summary>
    /// <see cref="RebuildPlan"/>, or on failure the current plan with the current service block
    /// (<c>failed</c> when the request's schema could not be built), so a waiting client is
    /// answered; the rebuild is retried on the next tick.
    /// </summary>
    private Plan TryRebuildPlan(Plan current)
    {
        if (_rebuildFailing)
        {
            _rebuildFailing = false;
            UpdateServiceState();
        }

        try
        {
            return RebuildPlan(current);
        }
        catch (Exception e)
        {
            Interlocked.Exchange(ref _structureDirty, 1);
            _rebuildFailing = true;
            UpdateServiceState();
            LogRateLimited("schema-rebuild", e, "Rebuilding the schema failed; the current devices stay in use and the rebuild is retried on the next tick");
            if (SchemaComparer.SameServiceState(current.Built.Schema.Service, _serviceState))
            {
                return current;
            }

            BuiltSchema stamped = current.Built with { Schema = current.Built.Schema with { Service = _serviceState } };
            _plan = new Plan(current.Roots, stamped, current.Revision + 1, current.Filter, current.Resolved);
            return _plan;
        }
    }

    private Plan RebuildPlan(Plan current)
    {
        Plan plan = BuildPlan(_tree.Roots, current);
        if (plan.Revision != current.Revision)
        {
            _log.LogInformation(
                "Hardware changed: schema revision {Revision}, {Devices} devices, {Sensors} sensors",
                plan.Revision,
                plan.Built.Schema.Devices.Count,
                plan.Built.Schema.Sensors.Count);
        }

        _plan = plan;
        return plan;
    }

    /// <summary>
    /// The schema covers every non-storage root of a requested module plus the disks the tick's
    /// storage round has resolved (with their resolved identity) whose SMART is on; unresolved
    /// disks stay out until then. The plan only updates and reads the roots of requested modules.
    /// </summary>
    private Plan BuildPlan(IReadOnlyList<HardwareNode> roots, Plan? current)
    {
        EffectiveConfig filter = _schemaFilter;
        IReadOnlyDictionary<string, DiskResolution> resolved = _view.Resolved;
        var planRoots = new List<HardwareNode>(roots.Count);
        var schemaRoots = new List<HardwareNode>(roots.Count);
        foreach (HardwareNode root in roots)
        {
            if (!HardwareModules.IsOn(root.Type, filter.Enabled))
            {
                continue;
            }

            planRoots.Add(root);
            if (root.Type != HardwareType.Storage)
            {
                schemaRoots.Add(root);
            }
            else if (resolved.TryGetValue(root.Identifier, out DiskResolution? resolution)
                && !DriveStates.IsSmartOff(resolution.Facts, resolution.Key, filter))
            {
                schemaRoots.Add(root with { Storage = resolution.Info });
            }
        }

        // A published id stays pinned while its disk keeps the same complete identity, so an
        // identical disk resolved in a later round never changes it (it gets its own id instead).
        var pins = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach ((string rootId, (DiskResolution pinned, string id)) in _storagePins)
        {
            if (resolved.TryGetValue(rootId, out DiskResolution? resolution) && resolution.Complete && Identity(resolution.Info) == Identity(pinned.Info))
            {
                pins[rootId] = id;
            }
        }

        BuiltSchema built = SchemaBuilder.Build(schemaRoots, _pawnIo, pins, _serviceState);
        foreach (string skipped in built.SkippedRoots)
        {
            LogNotUnique(skipped);
        }

        // A disk the request hides keeps its pin, so it comes back with the id its clients know.
        var hidden = _storagePins.Where(pin => DriveStates.IsSmartOff(pin.Value.Disk.Facts, pin.Value.Disk.Key, filter)).ToList();
        _storagePins.Clear();
        foreach ((string rootId, DiskResolution resolution) in resolved)
        {
            if (resolution.Complete && built.StorageDeviceIds.TryGetValue(rootId, out string? id))
            {
                _storagePins[rootId] = (resolution, id);
            }
        }

        foreach ((string rootId, (DiskResolution Disk, string Id) pin) in hidden)
        {
            _storagePins.TryAdd(rootId, pin);
        }

        if (current is null)
        {
            return new Plan(planRoots, built, revision: 1, filter, resolved);
        }

        return SchemaComparer.SameStructure(current.Built, built)
            ? new Plan(planRoots, current.Built, current.Revision, filter, resolved)
            : new Plan(planRoots, built, current.Revision + 1, filter, resolved);
    }

    /// <summary>The disk identity a pinned id depends on: the NVMe health flags come and go with updates and never change an id.</summary>
    private static StorageInfo Identity(StorageInfo info) => info with { IsNvme = false, HasCriticalWarning = false };

    /// <summary>
    /// The disk's identity with the descriptor model/serial and the rotational flag, from
    /// <see cref="IDiskPowerProbe.Describe"/>. Only a complete description (present, descriptor
    /// read, seek penalty known) is reused on later rounds while the drive number and serial
    /// stay the same; anything else is described again every round, so a transient answer never
    /// sticks. <see langword="null"/> when it cannot be described (logged once per disk).
    /// </summary>
    private DiskResolution? Resolve(HardwareNode root, IReadOnlyDictionary<string, DiskResolution> previous)
    {
        StorageInfo? fromTree = root.Storage;
        if (fromTree is null || fromTree.DriveNumber < 0)
        {
            return null;
        }

        if (previous.TryGetValue(root.Identifier, out DiskResolution? known)
            && known.Complete
            && known.Info.DriveNumber == fromTree.DriveNumber
            && known.Info.DriveSerial == fromTree.DriveSerial)
        {
            return known;
        }

        DriveFacts? facts;
        try
        {
            facts = _disks.Describe(fromTree.DriveNumber);
        }
        catch (Exception e)
        {
            LogRateLimited("describe:" + root.Identifier, e, "Describing {Root} failed", root.Identifier);
            facts = null;
        }

        if (facts is null)
        {
            if (_undescribedLogged.Add(root.Identifier))
            {
                _log.LogWarning(
                    "{Root} (PhysicalDrive{Drive}) cannot be described; it stays out of the schema and is retried every storage round",
                    root.Identifier,
                    fromTree.DriveNumber);
            }

            return null;
        }

        _undescribedLogged.Remove(root.Identifier);
        bool complete = facts.Availability == DriveAvailability.Present && facts.BusType is not null && facts.SeekPenalty is not null;
        StorageInfo info = fromTree with
        {
            DescriptorModel = facts.Model,
            DescriptorSerial = facts.Serial,
            Rotational = facts.Availability == DriveAvailability.NoMedia || facts.RequiresPowerCheck,
        };
        return new DiskResolution(info, facts, complete);
    }

    private static bool SameResolution(IReadOnlyDictionary<string, DiskResolution> a, IReadOnlyDictionary<string, DiskResolution> b) =>
        a.Count == b.Count && a.All(pair => b.TryGetValue(pair.Key, out DiskResolution? other) && other == pair.Value);

    /// <summary>
    /// A schema revision with its per-binding sampling plan (sampler-owned, replaced as a whole),
    /// over the roots of the modules <paramref name="filter"/> keeps on.
    /// </summary>
    private sealed class Plan
    {
        public Plan(IReadOnlyList<HardwareNode> roots, BuiltSchema built, int revision, EffectiveConfig filter, IReadOnlyDictionary<string, DiskResolution> resolved)
        {
            Roots = roots;
            Built = built;
            Revision = revision;
            Filter = filter;
            Resolved = resolved;
            UpdateRoots = roots.Where(r => r.Type != HardwareType.Storage).ToArray();

            var owners = new Dictionary<string, (string Root, bool Storage)>(StringComparer.Ordinal);
            foreach (HardwareNode root in roots)
            {
                AddOwners(root, root.Identifier, root.Type == HardwareType.Storage, owners);
            }

            int count = built.Bindings.Count;
            LhmIds = new string[count];
            Scales = new double[count];
            FromStorage = new bool[count];
            Owner = new string?[count];
            for (int i = 0; i < count; i++)
            {
                SensorBinding binding = built.Bindings[i];
                LhmIds[i] = binding.LhmIdentifier;
                Scales[i] = binding.Scale;
                if (binding.Source == BindingSource.NvmeCriticalWarning)
                {
                    // Keyed by the storage hardware identifier, and only ever cached by the storage worker.
                    FromStorage[i] = true;
                    Owner[i] = binding.LhmIdentifier;
                }
                else if (owners.TryGetValue(binding.LhmIdentifier, out (string Root, bool Storage) owner))
                {
                    FromStorage[i] = owner.Storage;
                    Owner[i] = owner.Root;
                }
            }

            Debug.Assert(built.Schema.Sensors.Count == count, "bindings are index-aligned with the schema sensors");
        }

        public IReadOnlyList<HardwareNode> Roots { get; }

        public BuiltSchema Built { get; }

        public int Revision { get; }

        /// <summary>The request whose schema effect this plan has.</summary>
        public EffectiveConfig Filter { get; }

        /// <summary>The resolved disks (of a <see cref="StorageRound"/>) this plan was built from: another instance means a rebuild.</summary>
        public IReadOnlyDictionary<string, DiskResolution> Resolved { get; }

        public HardwareNode[] UpdateRoots { get; }

        public string[] LhmIds { get; }

        public double[] Scales { get; }

        /// <summary>Read from the storage worker's cache, never from the tree.</summary>
        public bool[] FromStorage { get; }

        /// <summary>Identifier of the root whose failed update blanks this binding.</summary>
        public string?[] Owner { get; }

        private static void AddOwners(HardwareNode node, string root, bool storage, Dictionary<string, (string, bool)> owners)
        {
            foreach (SensorNode sensor in node.Sensors)
            {
                owners[sensor.Identifier] = (root, storage);
            }

            foreach (HardwareNode child in node.Children)
            {
                AddOwners(child, root, storage, owners);
            }
        }
    }
}
