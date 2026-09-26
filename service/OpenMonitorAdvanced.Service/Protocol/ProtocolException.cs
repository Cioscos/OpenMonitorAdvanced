namespace OpenMonitorAdvanced.Service.Protocol;

/// <summary>
/// Raised for any malformed or hostile input while encoding, decoding or
/// framing a sensor IPC message. Every error <see cref="MessageCodec"/> and
/// <see cref="FrameReader"/> can produce is fatal for the connection — there
/// is no attempt to resynchronize with the stream after one is thrown.
/// </summary>
public sealed class ProtocolException : Exception
{
    public ProtocolException(string message)
        : base(message)
    {
    }

    public ProtocolException(string message, Exception innerException)
        : base(message, innerException)
    {
    }
}
