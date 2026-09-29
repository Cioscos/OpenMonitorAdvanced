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
            tree.Open(ServiceModules.None);
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

    /// <summary>
    /// The seam opens nothing, so LHM's setters only record their flag (no group is built): what
    /// is checked is which groups <see cref="LhmTree"/> asks LHM for.
    /// </summary>
    private static (LhmTree Tree, Func<Computer?> Opened) NotOpening()
    {
        Computer? opened = null;
        var tree = new LhmTree(NullLogger<LhmTree>.Instance, () => new Computer(), computer => opened = computer);
        return (tree, () => opened);
    }

    private static (bool Cpu, bool Motherboard, bool Memory, bool Storage, bool Controller, bool Psu, bool Gpu, bool Network, bool Battery, bool PowerMonitor) Flags(Computer c) =>
        (c.IsCpuEnabled, c.IsMotherboardEnabled, c.IsMemoryEnabled, c.IsStorageEnabled, c.IsControllerEnabled, c.IsPsuEnabled, c.IsGpuEnabled, c.IsNetworkEnabled, c.IsBatteryEnabled, c.IsPowerMonitorEnabled);

    [Fact]
    public void OpenCreatesOnlyTheRequestedGroups()
    {
        (LhmTree tree, Func<Computer?> opened) = NotOpening();
        using (tree)
        {
            // Storage in the request changes nothing: the D6 gate enables it later.
            tree.Open(ServiceModules.Cpu | ServiceModules.Memory | ServiceModules.Psu | ServiceModules.Storage);

            Computer computer = Assert.IsType<Computer>(opened());
            Assert.Equal((true, false, true, false, false, true, false, false, false, false), Flags(computer));
        }
    }

    [Fact]
    public void SetModulesNeverTouchesStorage()
    {
        (LhmTree tree, Func<Computer?> opened) = NotOpening();
        using (tree)
        {
            tree.Open(ServiceModules.All);
            Computer computer = opened()!;

            tree.SetModules(ServiceModules.All);
            Assert.False(computer.IsStorageEnabled);

            tree.EnableStorage();
            tree.SetModules(ServiceModules.None);
            Assert.Equal((false, false, false, true, false, false, false, false, false, false), Flags(computer));

            tree.SetModules(ServiceModules.Motherboard | ServiceModules.Controller);
            Assert.Equal((false, true, false, true, true, false, false, false, false, false), Flags(computer));
        }
    }

    [Fact]
    public void RemovingMemoryRunsItsFinalizersBeforeReturning()
    {
        // RAMSPDToolkit's ~SPDAccessor restores the SPD page over SMBus: after the memory group
        // is closed its finalizers run at once, with the driver loaded, before any re-enable.
        (LhmTree tree, _) = NotOpening();
        using (tree)
        {
            tree.Open(ServiceModules.Cpu | ServiceModules.Memory);
            int gen2 = GC.CollectionCount(2);
            tree.SetModules(ServiceModules.Cpu | ServiceModules.Memory | ServiceModules.Psu);
            Assert.Equal(gen2, GC.CollectionCount(2)); // nothing removed: no collection

            tree.SetModules(ServiceModules.Cpu);
            Assert.True(GC.CollectionCount(2) > gen2);
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
