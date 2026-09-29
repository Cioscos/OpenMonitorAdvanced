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
    /// <see cref="FeedRequest.IntervalMs"/> ms, and counts <paramref name="request"/> in the
    /// service's effective configuration. Disposing the result unsubscribes.
    /// </summary>
    IFeedSubscription Subscribe(FeedRequest request, Action<FeedUpdate> onUpdate);
}

/// <summary>A live subscription; disposing it unsubscribes.</summary>
public interface IFeedSubscription : IDisposable
{
    /// <summary>
    /// Replaces this subscriber's request atomically: the feed never sees it without one (no
    /// passage through "no subscribers"), and the next update to it carries the schema. A no-op
    /// once disposed.
    /// </summary>
    void Update(FeedRequest request);
}

/// <summary>
/// One delivery: <paramref name="Schema"/> is present on a subscriber's first update and on
/// the first one after every revision change, and always describes <paramref name="Snapshot"/>'s
/// value order.
/// </summary>
public sealed record FeedUpdate(SchemaMessage? Schema, SnapshotMessage Snapshot);
