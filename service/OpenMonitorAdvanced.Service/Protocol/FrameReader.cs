using System.Buffers;
using System.Buffers.Binary;

namespace OpenMonitorAdvanced.Service.Protocol;

/// <summary>
/// Reads one <c>u32</c>-LE-length-prefixed frame at a time from a stream
/// (mirrors the framing half of <c>crates/oma-ipc/src/frame.rs</c>'s
/// <c>FrameDecoder</c>, adapted to a blocking/awaitable stream read instead
/// of the incremental push/pull buffer Rust's overlapped reader needs).
/// </summary>
public static class FrameReader
{
    /// <summary>
    /// Reads and decodes the next frame from <paramref name="stream"/>.
    /// </summary>
    /// <returns><c>null</c> on a clean end of stream at a frame boundary (no bytes read for the next header).</returns>
    /// <exception cref="ProtocolException">
    /// The stream ended with a truncated header or body, the declared frame
    /// length exceeds <see cref="ProtocolConstants.MaxFrameBytes"/> (thrown
    /// after reading exactly the 4-byte header, before touching the body),
    /// or the payload is malformed.
    /// </exception>
    public static async ValueTask<IMessage?> ReadAsync(Stream stream, CancellationToken ct)
    {
        var header = new byte[4];
        var headerRead = await ReadExactAsync(stream, header, ct).ConfigureAwait(false);
        if (headerRead == 0)
        {
            return null;
        }

        if (headerRead < header.Length)
        {
            throw new ProtocolException("stream ended with a truncated frame header");
        }

        var length = BinaryPrimitives.ReadUInt32LittleEndian(header);
        if (length > ProtocolConstants.MaxFrameBytes)
        {
            throw new ProtocolException(
                $"frame of {length} bytes exceeds the maximum of {ProtocolConstants.MaxFrameBytes} bytes");
        }

        var body = new byte[length];
        var bodyRead = await ReadExactAsync(stream, body, ct).ConfigureAwait(false);
        if (bodyRead < body.Length)
        {
            throw new ProtocolException("stream ended with a truncated frame body");
        }

        return MessageCodec.DecodePayload(new ReadOnlySequence<byte>(body));
    }

    /// <summary>
    /// Reads until <paramref name="buffer"/> is full or the stream hits EOF,
    /// returning the number of bytes actually read (which is less than
    /// <paramref name="buffer"/>'s length only at EOF).
    /// </summary>
    private static async ValueTask<int> ReadExactAsync(Stream stream, byte[] buffer, CancellationToken ct)
    {
        var totalRead = 0;
        while (totalRead < buffer.Length)
        {
            var read = await stream.ReadAsync(buffer.AsMemory(totalRead), ct).ConfigureAwait(false);
            if (read == 0)
            {
                break;
            }

            totalRead += read;
        }

        return totalRead;
    }
}
