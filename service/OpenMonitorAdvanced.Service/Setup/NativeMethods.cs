using System.Runtime.InteropServices;

namespace OpenMonitorAdvanced.Service.Setup;

/// <summary>
/// advapi32.dll P/Invoke declarations for <see cref="ServiceInstaller"/>. Every call sets
/// <c>SetLastError = true</c> so callers can log the exact Win32 error code (spec §2.2/§8/§9;
/// verified against the non-admin SCM spike, <c>docs/superpowers/references/m4/s3-pipe-scm.md</c>).
/// This class only declares the native surface; it does not run against the real SCM in tests
/// (that needs elevation — exercised by the Task 15 live check).
/// </summary>
internal static class NativeMethods
{
    // --- SCM/service access rights (only the ones this installer needs). ---
    internal const uint SC_MANAGER_CONNECT = 0x0001;
    internal const uint SC_MANAGER_CREATE_SERVICE = 0x0002;
    internal const uint SERVICE_ALL_ACCESS = 0xF01FF;

    // --- Service type / start type / error control (CreateServiceW). ---
    internal const uint SERVICE_WIN32_OWN_PROCESS = 0x00000010;
    internal const uint SERVICE_DEMAND_START = 0x00000003;
    internal const uint SERVICE_ERROR_NORMAL = 0x00000001;

    // --- ChangeServiceConfig2W info levels. ---
    internal const uint SERVICE_CONFIG_DESCRIPTION = 1;
    internal const uint SERVICE_CONFIG_FAILURE_ACTIONS = 2;

    // --- SC_ACTION types. ---
    internal const int SC_ACTION_NONE = 0;
    internal const int SC_ACTION_RESTART = 1;

    // --- Service control / status codes. ---
    internal const uint SERVICE_CONTROL_STOP = 0x00000001;
    internal const uint SERVICE_STOPPED = 0x00000001;
    internal const int SC_STATUS_PROCESS_INFO = 0;

    // --- Security information flags (SetServiceObjectSecurity/QueryServiceObjectSecurity). ---
    internal const uint DACL_SECURITY_INFORMATION = 0x00000004;

    // --- Win32 error codes this installer treats specially. ---
    internal const int ERROR_SERVICE_EXISTS = 1073;
    internal const int ERROR_SERVICE_MARKED_FOR_DELETE = 1072;
    internal const int ERROR_SERVICE_DOES_NOT_EXIST = 1060;
    internal const int ERROR_INSUFFICIENT_BUFFER = 122;

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    internal static extern IntPtr OpenSCManagerW(string? lpMachineName, string? lpDatabaseName, uint dwDesiredAccess);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    internal static extern IntPtr CreateServiceW(
        IntPtr hSCManager,
        string lpServiceName,
        string lpDisplayName,
        uint dwDesiredAccess,
        uint dwServiceType,
        uint dwStartType,
        uint dwErrorControl,
        string lpBinaryPathName,
        string? lpLoadOrderGroup,
        IntPtr lpdwTagId,
        string? lpDependencies,
        string? lpServiceStartName,
        string? lpPassword);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    internal static extern IntPtr OpenServiceW(IntPtr hSCManager, string lpServiceName, uint dwDesiredAccess);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    internal static extern bool ChangeServiceConfigW(
        IntPtr hService,
        uint dwServiceType,
        uint dwStartType,
        uint dwErrorControl,
        string? lpBinaryPathName,
        string? lpLoadOrderGroup,
        IntPtr lpdwTagId,
        string? lpDependencies,
        string? lpServiceStartName,
        string? lpPassword,
        string? lpDisplayName);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    internal static extern bool ChangeServiceConfig2W(IntPtr hService, uint dwInfoLevel, IntPtr lpInfo);

    [DllImport("advapi32.dll", SetLastError = true)]
    internal static extern bool DeleteService(IntPtr hService);

    [DllImport("advapi32.dll", SetLastError = true)]
    internal static extern bool CloseServiceHandle(IntPtr hSCObject);

    [DllImport("advapi32.dll", SetLastError = true)]
    internal static extern bool ControlService(IntPtr hService, uint dwControl, ref SERVICE_STATUS lpServiceStatus);

    [DllImport("advapi32.dll", SetLastError = true)]
    internal static extern bool QueryServiceStatusEx(
        IntPtr hService,
        int infoLevel,
        byte[] lpBuffer,
        uint cbBufSize,
        out uint pcbBytesNeeded);

    [DllImport("advapi32.dll", SetLastError = true)]
    internal static extern bool QueryServiceObjectSecurity(
        IntPtr hService,
        uint dwSecurityInformation,
        byte[] lpSecurityDescriptor,
        uint cbBufSize,
        out uint pcbBytesNeeded);

    [DllImport("advapi32.dll", SetLastError = true)]
    internal static extern bool SetServiceObjectSecurity(
        IntPtr hService,
        uint dwSecurityInformation,
        byte[] lpSecurityDescriptor);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    internal static extern bool ConvertSecurityDescriptorToStringSecurityDescriptorW(
        byte[] securityDescriptor,
        uint requestedStringSDRevision,
        uint securityInformation,
        out IntPtr stringSecurityDescriptor,
        out uint stringSecurityDescriptorLen);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    internal static extern bool ConvertStringSecurityDescriptorToSecurityDescriptorW(
        string stringSecurityDescriptor,
        uint stringSDRevision,
        out IntPtr securityDescriptor,
        out uint securityDescriptorSize);

    [DllImport("kernel32.dll", SetLastError = true)]
    internal static extern IntPtr LocalFree(IntPtr hMem);

    [StructLayout(LayoutKind.Sequential)]
    internal struct SERVICE_STATUS
    {
        public uint dwServiceType;
        public uint dwCurrentState;
        public uint dwControlsAccepted;
        public uint dwWin32ExitCode;
        public uint dwServiceSpecificExitCode;
        public uint dwCheckPoint;
        public uint dwWaitHint;

        // Compile-time-equivalent size check (FFI struct convention): SERVICE_STATUS is
        // documented as 7 DWORDs = 28 bytes.
        static SERVICE_STATUS()
        {
            if (Marshal.SizeOf<SERVICE_STATUS>() != 28)
            {
                throw new InvalidOperationException(
                    $"SERVICE_STATUS layout drifted: expected 28 bytes, got {Marshal.SizeOf<SERVICE_STATUS>()}.");
            }
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct SERVICE_STATUS_PROCESS
    {
        public uint dwServiceType;
        public uint dwCurrentState;
        public uint dwControlsAccepted;
        public uint dwWin32ExitCode;
        public uint dwServiceSpecificExitCode;
        public uint dwCheckPoint;
        public uint dwWaitHint;
        public uint dwProcessId;
        public uint dwServiceFlags;

        // Verified against the SCM spike: "SERVICE_STATUS_PROCESS is 36 bytes".
        static SERVICE_STATUS_PROCESS()
        {
            if (Marshal.SizeOf<SERVICE_STATUS_PROCESS>() != 36)
            {
                throw new InvalidOperationException(
                    $"SERVICE_STATUS_PROCESS layout drifted: expected 36 bytes, got {Marshal.SizeOf<SERVICE_STATUS_PROCESS>()}.");
            }
        }
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    internal struct SERVICE_DESCRIPTION
    {
        public string? lpDescription;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct SERVICE_FAILURE_ACTIONS
    {
        public uint dwResetPeriod;
        public IntPtr lpRebootMsg;
        public IntPtr lpCommand;
        public uint cActions;
        public IntPtr lpsaActions;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct SC_ACTION
    {
        public int Type;
        public uint Delay;

        // Verified against the Win32 SC_ACTION layout: one int32 + one uint32 = 8 bytes.
        static SC_ACTION()
        {
            if (Marshal.SizeOf<SC_ACTION>() != 8)
            {
                throw new InvalidOperationException(
                    $"SC_ACTION layout drifted: expected 8 bytes, got {Marshal.SizeOf<SC_ACTION>()}.");
            }
        }
    }
}
