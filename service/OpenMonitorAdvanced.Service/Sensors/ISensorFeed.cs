using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// A stream of sensor snapshots at a per-subscriber interval. The named-pipe server (Task 7)
/// subscribes one session per client; the callback runs on the hub's sampler thread and must
/// only enqueue.
/// </summary>
public interface ISensorFeed
{
    /// <summary>
    /// Starts delivering <see cref="FeedUpdate"/>s no more often than every
    /// <paramref name="intervalMs"/> ms. Disposing the result unsubscribes.
    /// </summary>
    IDisposable Subscribe(uint intervalMs, Action<FeedUpdate> onUpdate);
}

/// <summary>
/// One delivery: <paramref name="Schema"/> is present on a subscriber's first update and on
/// the first one after every revision change, and always describes <paramref name="Snapshot"/>'s
/// value order.
/// </summary>
public sealed record FeedUpdate(SchemaMessage? Schema, SnapshotMessage Snapshot);
