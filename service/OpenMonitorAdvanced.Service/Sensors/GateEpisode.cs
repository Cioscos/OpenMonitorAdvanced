using System.Globalization;
using Microsoft.Extensions.Logging;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// The D6 gate (controller ruling R17, design M6b §4.4) from its first round until LHM's storage
/// group is enabled: the storage worker's memory of what each drive answered, so that a closed
/// gate does not ask every drive at every round (each question resets Windows' idle timer of
/// that disk, and powers up a disk Windows turned off).
/// <list type="number">
/// <item>The first time it meets a drive whose <see cref="DriveFacts.RequiresPowerCheck"/> holds
/// it asks it once, unless Windows reports it off: that drive is in standby and blocks, with no
/// command.</item>
/// <item>Afterwards only a drive that blocks is asked again, and only after recent activity on
/// it (its counters grew, or Windows turned it on again); one whose counters cannot be read, at most every <see cref="BlindRetry"/>. An "active"
/// answer is kept as it is.</item>
/// <item>When no drive blocks any more, every drive not asked in this very round is asked once
/// more, so that the gate opens on answers of one round; a standby found then keeps it closed.
/// After a failed attempt to enable the group (<see cref="EnableFailed"/>) that check, and the
/// next attempt, wait <see cref="BlindRetry"/>: the answers are kept meanwhile.</item>
/// </list>
/// The gate may be opened after a round when <see cref="Opens"/>.
/// </summary>
internal sealed class GateEpisode(TimeProvider time, ILogger log)
{
    /// <summary>How long a blocker whose counters cannot be read is left alone, and how long a failed enable is not tried again.</summary>
    internal static readonly TimeSpan BlindRetry = TimeSpan.FromMinutes(5);

    private readonly Dictionary<int, Known> _known = [];
    private string? _lastBlockers;
    private long? _enableFailedAt;

    /// <summary>Whether the last round allows enabling the storage group: no drive blocks, on answers of that very round.</summary>
    internal bool Opens { get; private set; }

    /// <summary>Enabling the storage group failed after a round that <see cref="Opens"/>.</summary>
    internal void EnableFailed() => _enableFailedAt = time.GetTimestamp();

    /// <summary>
    /// One gate round over a fresh enumeration: one <see cref="DriveCheck"/> per drive, in the
    /// order of <paramref name="drives"/>. <paramref name="activity"/> is what the passive
    /// sources say about each drive that needs a power check.
    /// </summary>
    internal IReadOnlyList<DriveCheck> Round(
        IReadOnlyList<DriveFacts> drives,
        IReadOnlyDictionary<int, DriveActivity> activity,
        Func<DriveFacts, bool?> isSpunDown)
    {
        long now = time.GetTimestamp();
        var checks = new DriveCheck[drives.Count];
        var askedAt = new long?[drives.Count];
        var askedNow = new bool[drives.Count];
        for (int i = 0; i < drives.Count; i++)
        {
            DriveFacts drive = drives[i];
            checks[i] = new DriveCheck(drive, Asked: false, SpunDown: null);
            if (!drive.RequiresPowerCheck)
            {
                continue;
            }

            // Only for the disk it was learnt from: a drive number is reused by whatever is plugged in next.
            Known? known = _known.TryGetValue(drive.DriveNumber, out Known? met) && met.Check.Drive.Model == drive.Model && met.Check.Drive.Serial == drive.Serial ? met : null;
            askedAt[i] = known?.AskedAt;
            activity.TryGetValue(drive.DriveNumber, out DriveActivity seen);
            if (seen.PoweredOff)
            {
                checks[i] = checks[i] with { PoweredOff = true };
            }
            else if (known is null || (known.Check.Blocks && IsDue(seen, known.AskedAt, now)))
            {
                Ask(i);
            }
            else if (known.Check.Asked)
            {
                checks[i] = known.Check with { Drive = drive };
            }
            else
            {
                checks[i] = checks[i] with { Idle = true }; // on, never asked, and nothing shows that it works
            }
        }

        bool waits = _enableFailedAt is long failedAt && time.GetElapsedTime(failedAt, now) < BlindRetry;
        if (!waits && !checks.Any(c => c.Blocks))
        {
            for (int i = 0; i < drives.Count; i++)
            {
                if (drives[i].RequiresPowerCheck && !askedNow[i])
                {
                    Ask(i);
                }
            }
        }

        _known.Clear();
        for (int i = 0; i < drives.Count; i++)
        {
            if (drives[i].RequiresPowerCheck)
            {
                _known[drives[i].DriveNumber] = new Known(checks[i], askedAt[i]);
            }
        }

        Opens = !waits && !checks.Any(c => c.Blocks);
        LogBlockersOnChange([.. checks.Where(c => c.Blocks)]);
        return checks;

        void Ask(int i)
        {
            checks[i] = new DriveCheck(drives[i], Asked: true, isSpunDown(drives[i]));
            askedAt[i] = now;
            askedNow[i] = true;
        }
    }

    private bool IsDue(DriveActivity seen, long? askedAt, long now) =>
        seen.Recent || (!seen.Readable && (askedAt is not long at || time.GetElapsedTime(at, now) >= BlindRetry));

    private void LogBlockersOnChange(IReadOnlyList<DriveCheck> blockers)
    {
        string key = string.Join(';', blockers.Select(b => $"{b.Drive.DriveNumber}:{State(b)}"));
        if (key == _lastBlockers)
        {
            return;
        }

        _lastBlockers = key;
        if (blockers.Count == 0)
        {
            log.LogInformation("No drive keeps storage disabled any more");
            return;
        }

        foreach (DriveCheck blocker in blockers)
        {
            log.LogInformation(
                "PhysicalDrive{Drive} (bus {Bus}, model {Model}) keeps storage disabled: {State}",
                blocker.Drive.DriveNumber,
                blocker.Drive.BusType is uint bus ? "0x" + bus.ToString("X2", CultureInfo.InvariantCulture) : "unknown",
                DisplayName.Clean(blocker.Drive.Model) is { Length: > 0 } model ? model : "unknown",
                State(blocker));
        }

        static string State(DriveCheck blocker) =>
            blocker.PoweredOff ? "turned off by Windows"
            : blocker.Idle ? "not asked, no recent activity"
            : blocker.SpunDown == true ? "in standby"
            : "power state unknown";
    }

    /// <summary>What a drive last answered, and when (monotonic) it was last asked; <see langword="null"/> if never.</summary>
    private sealed record Known(DriveCheck Check, long? AskedAt);
}
