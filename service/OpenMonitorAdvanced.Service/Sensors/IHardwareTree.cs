namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// The hardware tree the <see cref="SensorHub"/> samples: LibreHardwareMonitor in production
/// (<see cref="LhmTree"/>, the only type that touches <c>LibreHardwareMonitor.Hardware.Computer</c>),
/// a scripted fake in tests.
/// </summary>
/// <remarks>
/// Threading contract: the hub calls <see cref="Update"/> and <see cref="Read"/> for a given root
/// from one thread only (the sampler for every non-storage root, the storage worker for storage
/// roots), and never calls <see cref="IDisposable.Dispose"/> while either is running.
/// <see cref="Roots"/> may be read from both threads.
/// </remarks>
public interface IHardwareTree : IDisposable
{
    /// <summary>
    /// Opens the tree with safe discovery only: storage stays disabled (decision D6), so opening
    /// never enumerates or identifies a disk. Returns the roots, like <see cref="Roots"/>.
    /// </summary>
    IReadOnlyList<HardwareNode> Open();

    /// <summary>The current roots (after hardware added/removed or sensors activated/deactivated).</summary>
    IReadOnlyList<HardwareNode> Roots { get; }

    /// <summary>Updates that hardware and its sub-hardware. May throw.</summary>
    void Update(HardwareNode root);

    /// <summary>The current raw LHM value of that sensor; <see langword="null"/> when unknown or null.</summary>
    double? Read(string sensorIdentifier);

    /// <summary>
    /// Raised when hardware is added or removed, or when a sensor is activated or deactivated.
    /// Handlers must only record the change (they may run inside an LHM callback or an update).
    /// </summary>
    event Action? HardwareChanged;

    /// <summary>
    /// Enables the LHM storage group once (the D6 gate: only when every rotational disk is known
    /// to be active, since creating it identifies every disk and can wake a sleeping one).
    /// </summary>
    void EnableStorage();
}

/// <summary>Disk power state checks for decision D6, without LHM and without waking a disk.</summary>
public interface IDiskPowerProbe
{
    /// <summary>ATA CHECK POWER MODE on <c>\\.\PhysicalDriveN</c>: <see langword="true"/> in standby, <see langword="null"/> when unknown.</summary>
    bool? IsSpunDown(int driveNumber);

    /// <summary>Every <c>PhysicalDriveN</c> with a seek penalty (or an unknown one) answers <see cref="IsSpunDown"/> == <see langword="false"/>.</summary>
    bool AllRotationalDisksActive();
}
