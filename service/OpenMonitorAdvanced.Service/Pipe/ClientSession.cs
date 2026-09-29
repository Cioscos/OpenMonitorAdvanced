using System.Threading.Channels;
using Microsoft.Extensions.Logging;
using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;

namespace OpenMonitorAdvanced.Service.Pipe;

/// <summary>
/// One connected pipe client (spec §6): sends <see cref="HelloMessage"/>, waits for a
/// <see cref="SubscribeMessage"/>, then forwards every <see cref="FeedUpdate"/> as an optional
/// <see cref="SchemaMessage"/> followed by its <see cref="SnapshotMessage"/>.
/// </summary>
/// <remarks>
/// <para>
/// <b>Two loops, one writer.</b> The reader loop only reads client frames and reacts to them; the
/// writer loop is the only code that writes to the pipe, in the order things were queued (Hello,
/// updates, a final Error). A Schema and its Snapshot go out as one write, so nothing can land
/// between them. The reader stays cancellable while the writer is blocked in a write.
/// </para>
/// <para>
/// <b>The feed callback does no I/O.</b> It runs on the hub's sampler thread and only puts the
/// update in the outbox. At most <see cref="MaxQueuedUpdates"/> updates may wait there (the one
/// being written is not counted); one more means the client is not keeping up, and only this
/// session closes. Updates are queued whole, so a Schema is never dropped while its Snapshot
/// stays queued. A write that does not complete within the write timeout also closes the session.
/// </para>
/// <para>
/// <b>Cleanup.</b> <see cref="RunAsync"/> never throws and, on every exit path, disposes the feed
/// subscription it holds exactly once. The pipe handle and the idle client count belong to the
/// listener, which releases them once when <see cref="RunAsync"/> returns.
/// </para>
/// </remarks>
internal sealed class ClientSession
{
    internal const int MaxQueuedUpdates = 2;

    private static readonly string ServiceVersion =
        typeof(ClientSession).Assembly.GetName().Version?.ToString(3) ?? "0.0.0";

    private readonly Stream _pipe;
    private readonly ISensorFeed _feed;
    private readonly PawnIoState _pawnIo;
    private readonly PipeListenerOptions _options;
    private readonly ILogger _log;
    private readonly int _id;

    private readonly Channel<Outgoing> _outbox = Channel.CreateUnbounded<Outgoing>(
        new UnboundedChannelOptions { SingleReader = true, SingleWriter = false });

    /// <summary>
    /// Cancelled by the feed callback on overflow, to abort a write blocked on a client that
    /// stopped reading. Never disposed: a late callback may still cancel it.
    /// </summary>
    private readonly CancellationTokenSource _abort = new();

    private readonly Lock _subscriptionGate = new();
    private IDisposable? _subscription;
    private bool _closed;

    /// <summary>Current subscription generation; callbacks of a replaced subscription are ignored.</summary>
    private int _generation;

    private int _queuedUpdates;
    private int _overflowed;

    public ClientSession(Stream pipe, ISensorFeed feed, PawnIoState pawnIo, PipeListenerOptions options, ILogger log, int id)
    {
        _pipe = pipe;
        _feed = feed;
        _pawnIo = pawnIo;
        _options = options;
        _log = log;
        _id = id;
    }

    /// <summary>Serves the client until it leaves, misbehaves or <paramref name="stopping"/> fires. Never throws.</summary>
    public async Task RunAsync(CancellationToken stopping)
    {
        using var session = CancellationTokenSource.CreateLinkedTokenSource(stopping, _abort.Token);
        try
        {
            _outbox.Writer.TryWrite(new Outgoing(new HelloMessage(ProtocolConstants.Version, ServiceVersion, PawnIoClassifier.ToWire(_pawnIo.Status)), null, Final: false));

            Task writer = WriteLoopAsync(session.Token);
            Task<bool> reader = ReadLoopAsync(session.Token);

            Task first = await Task.WhenAny(writer, reader).ConfigureAwait(false);
            if (first == reader && await reader.ConfigureAwait(false))
            {
                // The reader queued a final Error: let the writer send it (bounded by the write
                // timeout), then close. Everything else ends the session at once.
                await writer.ConfigureAwait(false);
            }

            await session.CancelAsync().ConfigureAwait(false);
            await Task.WhenAll(writer, reader).ConfigureAwait(false);
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            _log.LogWarning(e, "Pipe client {Client}: session failed", _id);
        }
        catch (OperationCanceledException)
        {
            // Shutdown or the session's own cancellation.
        }
        finally
        {
            CloseSubscription();
            _outbox.Writer.TryComplete();
        }

        if (Volatile.Read(ref _overflowed) != 0)
        {
            _log.LogWarning("Pipe client {Client} is not reading: more than {Max} updates queued; closed", _id, MaxQueuedUpdates);
        }
    }

    /// <summary>
    /// Reads client frames. Returns <see langword="true"/> when it queued a final Error for the
    /// writer, <see langword="false"/> when the client left or the session was cancelled.
    /// </summary>
    private async Task<bool> ReadLoopAsync(CancellationToken ct)
    {
        try
        {
            bool subscribed = false;
            while (true)
            {
                IMessage? message;
                using (var firstSubscribe = CancellationTokenSource.CreateLinkedTokenSource(ct))
                {
                    if (!subscribed)
                    {
                        firstSubscribe.CancelAfter(_options.SubscribeTimeout);
                    }

                    try
                    {
                        message = await FrameReader.ReadAsync(_pipe, firstSubscribe.Token).ConfigureAwait(false);
                    }
                    catch (OperationCanceledException) when (!ct.IsCancellationRequested)
                    {
                        return QueueError($"expected subscribe within {_options.SubscribeTimeout.TotalMilliseconds} ms");
                    }
                }

                switch (message)
                {
                    case null:
                        _log.LogDebug("Pipe client {Client} disconnected", _id);
                        return false;
                    case SubscribeMessage subscribe:
                        if (!TrySubscribe(subscribe.IntervalMs))
                        {
                            return false;
                        }

                        subscribed = true;
                        break;
                    default:
                        return QueueError($"unexpected {message.GetType().Name} from the client; only subscribe is accepted");
                }
            }
        }
        catch (ProtocolException e)
        {
            return QueueError(e.Message);
        }
        catch (OperationCanceledException)
        {
            return false;
        }
        catch (IOException e)
        {
            _log.LogDebug(e, "Pipe client {Client}: read failed", _id);
            return false;
        }
    }

    private bool QueueError(string message)
    {
        _log.LogWarning("Pipe client {Client}: bad request ({Reason}); closing", _id, message);
        _outbox.Writer.TryWrite(new Outgoing(new ErrorMessage("bad_request", message), null, Final: true));
        _outbox.Writer.TryComplete();
        return true;
    }

    /// <summary>Subscribes (or resubscribes) at the clamped interval; false if the feed failed.</summary>
    private bool TrySubscribe(uint requestedMs)
    {
        uint intervalMs = Math.Clamp(requestedMs, ProtocolConstants.MinIntervalMs, ProtocolConstants.MaxIntervalMs);
        int generation;
        IDisposable? previous;
        lock (_subscriptionGate)
        {
            if (_closed)
            {
                return false;
            }

            previous = _subscription;
            _subscription = null;
            generation = ++_generation;
        }

        previous?.Dispose();

        IDisposable subscription;
        try
        {
            subscription = _feed.Subscribe(intervalMs, update => OnUpdate(generation, update));
        }
        catch (Exception e)
        {
            _log.LogError(e, "Pipe client {Client}: subscribing to the sensor feed failed; closing", _id);
            return false;
        }

        bool adopted;
        lock (_subscriptionGate)
        {
            adopted = !_closed;
            if (adopted)
            {
                _subscription = subscription;
            }
        }

        if (!adopted)
        {
            // Closed while subscribing: cleanup already ran, so release the new subscription here.
            subscription.Dispose();
            return false;
        }

        _log.LogDebug("Pipe client {Client} subscribed at {Interval} ms", _id, intervalMs);
        return true;
    }

    /// <summary>Feed callback, on the hub's sampler thread: queue only, never block or write.</summary>
    private void OnUpdate(int generation, FeedUpdate update)
    {
        if (generation != Volatile.Read(ref _generation) || Volatile.Read(ref _overflowed) != 0)
        {
            return;
        }

        if (Interlocked.Increment(ref _queuedUpdates) > MaxQueuedUpdates)
        {
            Interlocked.Decrement(ref _queuedUpdates);
            if (Interlocked.Exchange(ref _overflowed, 1) == 0)
            {
                // End only this session. CancelAsync runs the cancellation callbacks (CancelIoEx on
                // a blocked write) on the thread pool, so this thread still does no I/O.
                _outbox.Writer.TryComplete();
                _ = _abort.CancelAsync();
            }

            return;
        }

        if (!_outbox.Writer.TryWrite(new Outgoing(null, update, Final: false)))
        {
            Interlocked.Decrement(ref _queuedUpdates); // session already closing
        }
    }

    private async Task WriteLoopAsync(CancellationToken ct)
    {
        try
        {
            await foreach (Outgoing item in _outbox.Reader.ReadAllAsync(ct).ConfigureAwait(false))
            {
                if (item.Update is not null)
                {
                    Interlocked.Decrement(ref _queuedUpdates);
                }

                if (Volatile.Read(ref _overflowed) != 0)
                {
                    break;
                }

                byte[] bytes = item.Update is { } update ? Encode(update) : MessageCodec.EncodeFrame(item.Message!);
                using (var write = CancellationTokenSource.CreateLinkedTokenSource(ct))
                {
                    write.CancelAfter(_options.WriteTimeout);
                    try
                    {
                        await _pipe.WriteAsync(bytes, write.Token).ConfigureAwait(false);
                    }
                    catch (OperationCanceledException) when (!ct.IsCancellationRequested)
                    {
                        _log.LogWarning(
                            "Pipe client {Client}: a write did not complete within {Seconds} s; closing",
                            _id,
                            _options.WriteTimeout.TotalSeconds);
                        return;
                    }
                }

                if (item.Final)
                {
                    return;
                }
            }
        }
        catch (OperationCanceledException)
        {
            // Session over.
        }
        catch (IOException e)
        {
            _log.LogDebug(e, "Pipe client {Client}: write failed", _id);
        }
    }

    /// <summary>Encodes a Schema (if any) and its Snapshot into one buffer: they go out as a single write.</summary>
    private static byte[] Encode(FeedUpdate update)
    {
        byte[] snapshot = MessageCodec.EncodeFrame(update.Snapshot);
        if (update.Schema is null)
        {
            return snapshot;
        }

        byte[] schema = MessageCodec.EncodeFrame(update.Schema);
        var both = new byte[schema.Length + snapshot.Length];
        schema.CopyTo(both, 0);
        snapshot.CopyTo(both, schema.Length);
        return both;
    }

    private void CloseSubscription()
    {
        IDisposable? subscription;
        lock (_subscriptionGate)
        {
            _closed = true;
            _generation++;
            subscription = _subscription;
            _subscription = null;
        }

        try
        {
            subscription?.Dispose();
        }
        catch (Exception e)
        {
            _log.LogWarning(e, "Pipe client {Client}: unsubscribing failed", _id);
        }
    }

    /// <summary>One writer work item: a control message, or a feed update.</summary>
    private sealed record Outgoing(IMessage? Message, FeedUpdate? Update, bool Final);
}
