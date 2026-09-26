using System.Globalization;
using System.Runtime.InteropServices;

namespace OpenMonitorAdvanced.Service.Setup;

/// <summary>
/// Install/uninstall helper for the <c>oma-service</c> Windows service, used by
/// <c>oma-service.exe install|uninstall</c> (invoked by the NSIS installer, spec §8/§9).
/// Every SCM call is P/Invoke against advapi32.dll (see <see cref="NativeMethods"/>); none of
/// this is unit-tested, because it needs administrator privileges (exercised live in Task 15).
/// Every failure is logged with its Win32 error code and turned into a return code, never an
/// exception, so <c>Program.Main</c> can report a clean exit code.
/// </summary>
public static class ServiceInstaller
{
    public const string ServiceName = "oma-service";
    public const string DisplayName = "OpenMonitor Advanced Sensors";

    private const string Description =
        "Provides CPU, motherboard, memory and disk sensors to OpenMonitor Advanced over a local named pipe. Read-only.";

    private const uint FailureActionResetPeriodSeconds = 86400;
    private const uint FailureActionRestartDelayMs = 60000;

    private const int MarkedForDeleteMaxAttempts = 10;
    private const int MarkedForDeleteRetryDelayMs = 500;
    private const int StopWaitTimeoutSeconds = 30;
    private const int StopPollDelayMs = 250;

    /// <summary>Installs (or reconfigures, if already installed) the service. 0 = ok, 1 = failed.</summary>
    public static int Install(string exePath, TextWriter log)
    {
        var scm = NativeMethods.OpenSCManagerW(
            null, null, NativeMethods.SC_MANAGER_CONNECT | NativeMethods.SC_MANAGER_CREATE_SERVICE);
        if (scm == IntPtr.Zero)
        {
            LogWin32(log, "OpenSCManagerW");
            return 1;
        }

        var svc = IntPtr.Zero;
        try
        {
            var binaryPath = $"\"{exePath}\"";
            svc = CreateOrOpenExisting(scm, binaryPath, log);
            if (svc == IntPtr.Zero)
            {
                return 1;
            }

            if (!SetDescription(svc, log))
            {
                return 1;
            }

            if (!SetFailureActions(svc, log))
            {
                return 1;
            }

            if (!GrantInteractiveStartStop(svc, log))
            {
                return 1;
            }

            return 0;
        }
        finally
        {
            if (svc != IntPtr.Zero)
            {
                NativeMethods.CloseServiceHandle(svc);
            }

            NativeMethods.CloseServiceHandle(scm);
        }
    }

    /// <summary>Stops and removes the service. 0 also when it was not installed.</summary>
    public static int Uninstall(TextWriter log)
    {
        var scm = NativeMethods.OpenSCManagerW(null, null, NativeMethods.SC_MANAGER_CONNECT);
        if (scm == IntPtr.Zero)
        {
            LogWin32(log, "OpenSCManagerW");
            return 1;
        }

        try
        {
            var svc = NativeMethods.OpenServiceW(scm, ServiceName, NativeMethods.SERVICE_ALL_ACCESS);
            if (svc == IntPtr.Zero)
            {
                var error = Marshal.GetLastWin32Error();
                if (error == NativeMethods.ERROR_SERVICE_DOES_NOT_EXIST)
                {
                    return 0;
                }

                LogWin32(log, "OpenServiceW", error);
                return 1;
            }

            try
            {
                var status = default(NativeMethods.SERVICE_STATUS);
                if (!NativeMethods.ControlService(svc, NativeMethods.SERVICE_CONTROL_STOP, ref status))
                {
                    // ERROR_SERVICE_NOT_ACTIVE (1062) just means it was already stopped; anything
                    // else is worth logging, but we still try to delete the service afterwards.
                    LogWin32(log, "ControlService(STOP)");
                }

                WaitUntilStopped(svc, log);

                if (!NativeMethods.DeleteService(svc))
                {
                    LogWin32(log, "DeleteService");
                    return 1;
                }

                return 0;
            }
            finally
            {
                NativeMethods.CloseServiceHandle(svc);
            }
        }
        finally
        {
            NativeMethods.CloseServiceHandle(scm);
        }
    }

    private static IntPtr CreateOrOpenExisting(IntPtr scm, string binaryPath, TextWriter log)
    {
        for (var attempt = 0; attempt < MarkedForDeleteMaxAttempts; attempt++)
        {
            var svc = NativeMethods.CreateServiceW(
                scm,
                ServiceName,
                DisplayName,
                NativeMethods.SERVICE_ALL_ACCESS,
                NativeMethods.SERVICE_WIN32_OWN_PROCESS,
                NativeMethods.SERVICE_DEMAND_START,
                NativeMethods.SERVICE_ERROR_NORMAL,
                binaryPath,
                null,
                IntPtr.Zero,
                null,
                null,
                null);

            if (svc != IntPtr.Zero)
            {
                return svc;
            }

            var error = Marshal.GetLastWin32Error();
            if (error == NativeMethods.ERROR_SERVICE_EXISTS)
            {
                return ReconfigureExisting(scm, binaryPath, log);
            }

            if (error == NativeMethods.ERROR_SERVICE_MARKED_FOR_DELETE && attempt < MarkedForDeleteMaxAttempts - 1)
            {
                Thread.Sleep(MarkedForDeleteRetryDelayMs);
                continue;
            }

            LogWin32(log, "CreateServiceW", error);
            return IntPtr.Zero;
        }

        log.WriteLine("CreateServiceW: service stayed marked for deletion after 10 attempts.");
        return IntPtr.Zero;
    }

    private static IntPtr ReconfigureExisting(IntPtr scm, string binaryPath, TextWriter log)
    {
        var svc = NativeMethods.OpenServiceW(scm, ServiceName, NativeMethods.SERVICE_ALL_ACCESS);
        if (svc == IntPtr.Zero)
        {
            LogWin32(log, "OpenServiceW");
            return IntPtr.Zero;
        }

        if (!NativeMethods.ChangeServiceConfigW(
                svc,
                NativeMethods.SERVICE_WIN32_OWN_PROCESS,
                NativeMethods.SERVICE_DEMAND_START,
                NativeMethods.SERVICE_ERROR_NORMAL,
                binaryPath,
                null,
                IntPtr.Zero,
                null,
                null,
                null,
                null))
        {
            LogWin32(log, "ChangeServiceConfigW");
            NativeMethods.CloseServiceHandle(svc);
            return IntPtr.Zero;
        }

        return svc;
    }

    private static bool SetDescription(IntPtr svc, TextWriter log)
    {
        var descriptor = new NativeMethods.SERVICE_DESCRIPTION { lpDescription = Description };
        var ptr = Marshal.AllocHGlobal(Marshal.SizeOf<NativeMethods.SERVICE_DESCRIPTION>());
        try
        {
            Marshal.StructureToPtr(descriptor, ptr, false);
            if (!NativeMethods.ChangeServiceConfig2W(svc, NativeMethods.SERVICE_CONFIG_DESCRIPTION, ptr))
            {
                LogWin32(log, "ChangeServiceConfig2W(SERVICE_CONFIG_DESCRIPTION)");
                return false;
            }

            return true;
        }
        finally
        {
            Marshal.DestroyStructure<NativeMethods.SERVICE_DESCRIPTION>(ptr);
            Marshal.FreeHGlobal(ptr);
        }
    }

    private static bool SetFailureActions(IntPtr svc, TextWriter log)
    {
        var actionSize = Marshal.SizeOf<NativeMethods.SC_ACTION>();
        var actions = new[]
        {
            new NativeMethods.SC_ACTION { Type = NativeMethods.SC_ACTION_RESTART, Delay = FailureActionRestartDelayMs },
            new NativeMethods.SC_ACTION { Type = NativeMethods.SC_ACTION_RESTART, Delay = FailureActionRestartDelayMs },
            new NativeMethods.SC_ACTION { Type = NativeMethods.SC_ACTION_NONE, Delay = 0 },
        };

        var actionsPtr = Marshal.AllocHGlobal(actionSize * actions.Length);
        var infoPtr = IntPtr.Zero;
        try
        {
            for (var i = 0; i < actions.Length; i++)
            {
                Marshal.StructureToPtr(actions[i], actionsPtr + (i * actionSize), false);
            }

            var info = new NativeMethods.SERVICE_FAILURE_ACTIONS
            {
                dwResetPeriod = FailureActionResetPeriodSeconds,
                lpRebootMsg = IntPtr.Zero,
                lpCommand = IntPtr.Zero,
                cActions = (uint)actions.Length,
                lpsaActions = actionsPtr,
            };

            infoPtr = Marshal.AllocHGlobal(Marshal.SizeOf<NativeMethods.SERVICE_FAILURE_ACTIONS>());
            Marshal.StructureToPtr(info, infoPtr, false);

            if (!NativeMethods.ChangeServiceConfig2W(svc, NativeMethods.SERVICE_CONFIG_FAILURE_ACTIONS, infoPtr))
            {
                LogWin32(log, "ChangeServiceConfig2W(SERVICE_CONFIG_FAILURE_ACTIONS)");
                return false;
            }

            return true;
        }
        finally
        {
            for (var i = 0; i < actions.Length; i++)
            {
                Marshal.DestroyStructure<NativeMethods.SC_ACTION>(actionsPtr + (i * actionSize));
            }

            Marshal.FreeHGlobal(actionsPtr);
            if (infoPtr != IntPtr.Zero)
            {
                Marshal.DestroyStructure<NativeMethods.SERVICE_FAILURE_ACTIONS>(infoPtr);
                Marshal.FreeHGlobal(infoPtr);
            }
        }
    }

    private static bool GrantInteractiveStartStop(IntPtr svc, TextWriter log)
    {
        if (!TryQuerySecurityDescriptor(svc, log, out var sd))
        {
            return false;
        }

        string sddl;
        var sddlPtr = IntPtr.Zero;
        try
        {
            if (!NativeMethods.ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    sd, 1 /* SDDL_REVISION_1 */, NativeMethods.DACL_SECURITY_INFORMATION, out sddlPtr, out _))
            {
                LogWin32(log, "ConvertSecurityDescriptorToStringSecurityDescriptorW");
                return false;
            }

            sddl = Marshal.PtrToStringUni(sddlPtr) ?? string.Empty;
        }
        finally
        {
            if (sddlPtr != IntPtr.Zero)
            {
                NativeMethods.LocalFree(sddlPtr);
            }
        }

        string newSddl;
        try
        {
            newSddl = SddlEditor.GrantStartStopToInteractiveUsers(sddl);
        }
        catch (InvalidOperationException ex)
        {
            log.WriteLine($"GrantStartStopToInteractiveUsers failed on '{sddl}': {ex.Message}");
            return false;
        }

        var newSdPtr = IntPtr.Zero;
        try
        {
            if (!NativeMethods.ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    newSddl, 1 /* SDDL_REVISION_1 */, out newSdPtr, out var newSdSize))
            {
                LogWin32(log, "ConvertStringSecurityDescriptorToSecurityDescriptorW");
                return false;
            }

            var newSd = new byte[newSdSize];
            Marshal.Copy(newSdPtr, newSd, 0, (int)newSdSize);

            if (!NativeMethods.SetServiceObjectSecurity(svc, NativeMethods.DACL_SECURITY_INFORMATION, newSd))
            {
                LogWin32(log, "SetServiceObjectSecurity");
                return false;
            }

            return true;
        }
        finally
        {
            if (newSdPtr != IntPtr.Zero)
            {
                NativeMethods.LocalFree(newSdPtr);
            }
        }
    }

    private static bool TryQuerySecurityDescriptor(IntPtr svc, TextWriter log, out byte[] securityDescriptor)
    {
        var buffer = Array.Empty<byte>();
        if (!NativeMethods.QueryServiceObjectSecurity(
                svc, NativeMethods.DACL_SECURITY_INFORMATION, buffer, 0, out var needed))
        {
            var error = Marshal.GetLastWin32Error();
            if (error != NativeMethods.ERROR_INSUFFICIENT_BUFFER)
            {
                LogWin32(log, "QueryServiceObjectSecurity(sizing)", error);
                securityDescriptor = [];
                return false;
            }
        }

        buffer = new byte[needed];
        if (!NativeMethods.QueryServiceObjectSecurity(
                svc, NativeMethods.DACL_SECURITY_INFORMATION, buffer, needed, out _))
        {
            LogWin32(log, "QueryServiceObjectSecurity");
            securityDescriptor = [];
            return false;
        }

        securityDescriptor = buffer;
        return true;
    }

    private static void WaitUntilStopped(IntPtr svc, TextWriter log)
    {
        var deadline = DateTime.UtcNow.AddSeconds(StopWaitTimeoutSeconds);
        while (DateTime.UtcNow < deadline)
        {
            if (!TryQueryState(svc, log, out var state))
            {
                return;
            }

            if (state == NativeMethods.SERVICE_STOPPED)
            {
                return;
            }

            Thread.Sleep(StopPollDelayMs);
        }

        log.WriteLine($"Uninstall: the service did not reach SERVICE_STOPPED within {StopWaitTimeoutSeconds} s.");
    }

    private static bool TryQueryState(IntPtr svc, TextWriter log, out uint state)
    {
        var size = Marshal.SizeOf<NativeMethods.SERVICE_STATUS_PROCESS>();
        var buffer = new byte[size];
        if (!NativeMethods.QueryServiceStatusEx(
                svc, NativeMethods.SC_STATUS_PROCESS_INFO, buffer, (uint)size, out _))
        {
            LogWin32(log, "QueryServiceStatusEx");
            state = 0;
            return false;
        }

        var handle = GCHandle.Alloc(buffer, GCHandleType.Pinned);
        try
        {
            var status = Marshal.PtrToStructure<NativeMethods.SERVICE_STATUS_PROCESS>(handle.AddrOfPinnedObject());
            state = status.dwCurrentState;
            return true;
        }
        finally
        {
            handle.Free();
        }
    }

    private static void LogWin32(TextWriter log, string apiName)
    {
        LogWin32(log, apiName, Marshal.GetLastWin32Error());
    }

    private static void LogWin32(TextWriter log, string apiName, int win32Error)
    {
        log.WriteLine(string.Create(
            CultureInfo.InvariantCulture,
            $"{apiName} failed with Win32 error {win32Error}."));
    }
}
