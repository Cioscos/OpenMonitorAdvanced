using System.Runtime.InteropServices;

using OpenMonitorAdvanced.Service.Setup;

using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Setup;

/// <summary>
/// Asserts the marshalled size of every FFI struct in <see cref="NativeMethods"/> (the
/// compile-time-assert convention adapted to C#: a struct's own static constructor only runs on
/// first access of a static *member*, so it is never guaranteed to run just because the struct is
/// used as a P/Invoke parameter or field type — an actual test that calls <see cref="Marshal.SizeOf{T}"/>
/// is the only way to catch a layout drift). Expected sizes are for x64 (this project's only
/// <c>RuntimeIdentifier</c>, <c>win-x64</c>).
/// </summary>
public sealed class NativeMethodsStructLayoutTests
{
    [Fact]
    public void ServiceStatusIsTwentyEightBytes()
    {
        // 7 DWORDs, no padding (all fields are 4 bytes): 7 * 4 = 28.
        Assert.Equal(28, Marshal.SizeOf<NativeMethods.SERVICE_STATUS>());
    }

    [Fact]
    public void ServiceStatusProcessIsThirtySixBytes()
    {
        // 9 DWORDs, no padding: 9 * 4 = 36 (matches the SCM spike's measurement).
        Assert.Equal(36, Marshal.SizeOf<NativeMethods.SERVICE_STATUS_PROCESS>());
    }

    [Fact]
    public void ScActionIsEightBytes()
    {
        // int (4) + uint (4), no padding needed (both 4-byte fields): 8.
        Assert.Equal(8, Marshal.SizeOf<NativeMethods.SC_ACTION>());
    }

    [Fact]
    public void ServiceDescriptionIsOnePointerWide()
    {
        // A single marshalled LPWSTR field: one pointer, 8 bytes on x64.
        Assert.Equal(8, Marshal.SizeOf<NativeMethods.SERVICE_DESCRIPTION>());
    }

    [Fact]
    public void ServiceFailureActionsIsFortyBytes()
    {
        // dwResetPeriod (uint, 4) + 4 bytes padding to 8-align the first IntPtr +
        // lpRebootMsg (IntPtr, 8) + lpCommand (IntPtr, 8) +
        // cActions (uint, 4) + 4 bytes padding to 8-align the next IntPtr +
        // lpsaActions (IntPtr, 8) = 40 bytes on x64.
        Assert.Equal(40, Marshal.SizeOf<NativeMethods.SERVICE_FAILURE_ACTIONS>());
    }
}
