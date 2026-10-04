using System.Runtime.InteropServices;

namespace OpenMonitorAdvanced.Service.Frames;

/// <summary>Stops or flushes a named ETW session. Returns the Win32 status (0 = success).</summary>
internal interface IEtwSession
{
    uint Stop(string name);

    uint Flush(string name);
}

/// <summary>
/// <see cref="IEtwSession"/> over <c>ControlTraceW</c> by session name. Stopping a missing session
/// returns 4201 (<c>ERROR_WMI_INSTANCE_NOT_FOUND</c>), even without administrator rights (M7b spike).
/// </summary>
internal sealed class EtwSessionControl : IEtwSession
{
    private const int NameChars = 1024;
    private static readonly int StructSize = Marshal.SizeOf<FramesNative.EventTraceProperties>();

    public uint Stop(string name) => Control(name, FramesNative.EventTraceControlStop);

    public uint Flush(string name) => Control(name, FramesNative.EventTraceControlFlush);

    private static uint Control(string name, uint code)
    {
        // The properties block is the struct followed by room for the logger name and the log file
        // name (2 x 1024 WCHARs), which ControlTraceW fills in.
        int total = StructSize + (2 * NameChars * sizeof(char));
        IntPtr buffer = Marshal.AllocHGlobal(total);
        try
        {
            for (int i = 0; i < total; i += sizeof(long))
            {
                Marshal.WriteInt64(buffer, i, 0);
            }

            var props = new FramesNative.EventTraceProperties
            {
                Wnode = new FramesNative.WnodeHeader { BufferSize = (uint)total },
                LoggerNameOffset = (uint)StructSize,
                LogFileNameOffset = (uint)(StructSize + (NameChars * sizeof(char))),
            };
            Marshal.StructureToPtr(props, buffer, fDeleteOld: false);

            // SAFETY: `buffer` is a live, zeroed allocation of `total` bytes that holds the properties
            // struct and the two name areas at the offsets stored in it; it outlives the call and is
            // freed in `finally`. `name` is a managed string marshalled as a NUL-terminated WCHAR*.
            return FramesNative.ControlTraceW(0, name, buffer, code);
        }
        finally
        {
            Marshal.FreeHGlobal(buffer);
        }
    }
}
