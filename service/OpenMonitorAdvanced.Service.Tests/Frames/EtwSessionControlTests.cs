using OpenMonitorAdvanced.Service.Frames;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Frames;

public sealed class EtwSessionControlTests
{
    [Fact]
    public void StoppingAMissingSessionReportsNotFound()
    {
        // Verified without privileges in the M7b spike: ERROR_WMI_INSTANCE_NOT_FOUND.
        Assert.Equal(4201u, new EtwSessionControl().Stop("OpenMonitorAdvanced-Test-Missing"));
    }
}
