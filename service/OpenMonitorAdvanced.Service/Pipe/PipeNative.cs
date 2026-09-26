using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace OpenMonitorAdvanced.Service.Pipe;

/// <summary>
/// kernel32/advapi32 P/Invoke for the sensor pipe's server instances (spec §6, spike S3 §1):
/// <c>CreateNamedPipeW</c> instead of <c>NamedPipeServerStreamAcl.Create</c>, because only the
/// native call can set <c>PIPE_REJECT_REMOTE_CLIENTS</c>. The handle is then wrapped in a
/// <see cref="System.IO.Pipes.NamedPipeServerStream"/> by the caller.
/// </summary>
internal static class PipeNative
{
    internal const int ErrorAccessDenied = 5;

    private const uint PipeAccessDuplex = 0x00000003;
    private const uint FileFlagOverlapped = 0x40000000;
    private const uint FileFlagFirstPipeInstance = 0x00080000;
    private const uint PipeTypeByte = 0x00000000;
    private const uint PipeReadModeByte = 0x00000000;
    private const uint PipeWait = 0x00000000;
    private const uint PipeRejectRemoteClients = 0x00000008;
    private const uint SddlRevision1 = 1;

    /// <summary>Outbound buffer: a typical snapshot fits; a client that stops reading fills it and blocks the write.</summary>
    private const uint OutBufferSize = 64 * 1024;

    /// <summary>Inbound buffer: the client only ever sends small <c>Subscribe</c> frames.</summary>
    private const uint InBufferSize = 4 * 1024;

    /// <summary>Byte stream in both directions, blocking mode (overlapped I/O comes from the open mode), local clients only.</summary>
    internal const uint PipeMode = PipeTypeByte | PipeReadModeByte | PipeWait | PipeRejectRemoteClients;

    /// <summary>Duplex and overlapped; the first instance also claims the name (fails if it already exists).</summary>
    internal static uint OpenMode(bool first) =>
        PipeAccessDuplex | FileFlagOverlapped | (first ? FileFlagFirstPipeInstance : 0);

    /// <summary>
    /// Creates one server instance of <paramref name="pipeName"/>. <paramref name="sddl"/> is the
    /// security descriptor (<see langword="null"/> = the default DACL, for tests).
    /// </summary>
    /// <exception cref="Win32Exception">The instance could not be created; <see cref="Win32Exception.NativeErrorCode"/> has the Win32 error.</exception>
    internal static SafePipeHandle CreateInstance(string pipeName, bool first, int maxInstances, string? sddl)
    {
        IntPtr descriptor = IntPtr.Zero;
        try
        {
            if (sddl is not null)
            {
                // SAFETY: sddl is a valid string for the call; on success the descriptor is a
                // LocalAlloc'd block owned by us and freed in the finally below.
                if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, SddlRevision1, out descriptor, IntPtr.Zero))
                {
                    throw new Win32Exception(Marshal.GetLastPInvokeError(), "ConvertStringSecurityDescriptorToSecurityDescriptorW failed");
                }
            }

            // A null lpSecurityDescriptor means the default DACL (creator and SYSTEM: full access).
            var attributes = new SECURITY_ATTRIBUTES
            {
                nLength = Marshal.SizeOf<SECURITY_ATTRIBUTES>(),
                lpSecurityDescriptor = descriptor,
                bInheritHandle = 0,
            };

            // SAFETY: attributes (and the descriptor it points to) stay alive for the whole call; the
            // returned handle is owned by the SafePipeHandle, which closes it exactly once.
            SafePipeHandle handle = CreateNamedPipeW(
                $@"\\.\pipe\{pipeName}",
                OpenMode(first),
                PipeMode,
                (uint)maxInstances,
                OutBufferSize,
                InBufferSize,
                0,
                ref attributes);
            if (handle.IsInvalid)
            {
                int error = Marshal.GetLastPInvokeError();
                handle.Dispose();
                throw new Win32Exception(error);
            }

            return handle;
        }
        finally
        {
            if (descriptor != IntPtr.Zero)
            {
                // SAFETY: descriptor came from ConvertStringSecurityDescriptorToSecurityDescriptorW
                // and is freed once, after CreateNamedPipeW copied it into the pipe object.
                _ = LocalFree(descriptor);
            }
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct SECURITY_ATTRIBUTES
    {
        public int nLength;
        public IntPtr lpSecurityDescriptor;
        public int bInheritHandle;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafePipeHandle CreateNamedPipeW(
        string lpName,
        uint dwOpenMode,
        uint dwPipeMode,
        uint nMaxInstances,
        uint nOutBufferSize,
        uint nInBufferSize,
        uint nDefaultTimeOut,
        ref SECURITY_ATTRIBUTES lpSecurityAttributes);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool ConvertStringSecurityDescriptorToSecurityDescriptorW(
        string StringSecurityDescriptor,
        uint StringSDRevision,
        out IntPtr SecurityDescriptor,
        IntPtr SecurityDescriptorSize);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern IntPtr LocalFree(IntPtr hMem);
}
