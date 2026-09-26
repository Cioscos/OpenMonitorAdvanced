using LibreHardwareMonitor.Hardware;
using Microsoft.Extensions.Logging.Abstractions;
using OpenMonitorAdvanced.Service.Sensors;
using RAMSPDToolkit.I2CSMBus.Interop.PawnIO;
using RAMSPDToolkit.Windows.Driver;
using RAMSPDToolkit.Windows.Driver.Interfaces;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// Tests that swap RAMSPDToolkit's process-wide <see cref="DriverManager.Driver"/> or count
/// gen-2 collections: they must not overlap with any other test.
/// </summary>
[CollectionDefinition(nameof(ProcessWideStateCollection), DisableParallelization = true)]
public sealed class ProcessWideStateCollection;

/// <summary>
/// <see cref="LhmTree.Dispose"/> over a real LibreHardwareMonitor <see cref="Computer"/> with no
/// hardware group enabled (so no PawnIO and no administrator rights are needed). The hardware
/// paths themselves are exercised live in Task 15.
/// </summary>
[Collection(nameof(ProcessWideStateCollection))]
public sealed class LhmTreeTests
{
    /// <summary>
    /// Task 15 regression: unloading the SMBus driver on dispose nulled the PawnIO modules that
    /// RAMSPDToolkit's <c>SPDAccessor</c> finalizers still use to restore the SPD page, and the
    /// forced collection that followed ran those finalizers at once. The NullReferenceException
    /// on the finalizer thread killed the stopping service (exit 1, SCM event 7031, restart).
    /// LHM is closed only when the process exits, so the driver must stay loaded (the OS
    /// releases its handles at exit) and no collection may be forced.
    /// </summary>
    [Fact]
    public void DisposeClosesTheComputerWithoutUnloadingTheSmBusDriverOrForcingACollection()
    {
        IDriver? previous = DriverManager.Driver;
        var driver = new RecordingPawnIoDriver();
        DriverManager.Driver = driver;
        try
        {
            var tree = new LhmTree(NullLogger<LhmTree>.Instance, () => new Computer());
            tree.Open();
            int gen2Before = GC.CollectionCount(2);

            tree.Dispose();

            Assert.Equal(0, driver.Unloads);
            Assert.Same(driver, DriverManager.Driver);
            Assert.Equal(gen2Before, GC.CollectionCount(2));
        }
        finally
        {
            DriverManager.Driver = previous;
        }
    }

    private sealed class RecordingPawnIoDriver : IPawnIODriver
    {
        public int Unloads { get; private set; }

        public bool IsOpen => true;

        public bool Load() => true;

        public IPawnIOModule? LoadModule(PawnIOSMBusIdentifier pawnIOSMBusIdentifier) => null;

        public void Unload() => Unloads++;
    }
}
