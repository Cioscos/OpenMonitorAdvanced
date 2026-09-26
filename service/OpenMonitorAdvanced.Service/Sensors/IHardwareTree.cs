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

    /// <summary>
    /// The D6 gate (controller ruling R17): every <c>PhysicalDriveN</c> whose
    /// <see cref="DriveFacts.RequiresPowerCheck"/> is true answers <see cref="IsSpunDown"/> ==
    /// <see langword="false"/>.
    /// </summary>
    bool AllRotationalDisksActive();

    /// <summary>
    /// Model, serial, bus type and seek penalty of <c>\\.\PhysicalDriveN</c>, read with access 0
    /// (never wakes a disk); <see langword="null"/> when there is no such drive. May block on a
    /// slow device: the hub calls it only from its storage worker.
    /// </summary>
    DriveFacts? Describe(int driveNumber);
}

/// <summary>Whether a <c>PhysicalDriveN</c> could be described.</summary>
public enum DriveAvailability
{
    /// <summary>Opened and queried (some facts may still be unknown).</summary>
    Present,

    /// <summary>The open or a query failed with "not ready" / "no media" (an empty card reader).</summary>
    NoMedia,

    /// <summary>The open failed for another reason: nothing is known about the drive.</summary>
    Unreadable,
}

/// <summary>
/// What can be learned about a <c>PhysicalDriveN</c> without waking it (access 0):
/// <c>STORAGE_DEVICE_DESCRIPTOR</c> model/serial/bus type and <c>DEVICE_SEEK_PENALTY_DESCRIPTOR</c>.
/// </summary>
public sealed record DriveFacts(int DriveNumber, DriveAvailability Availability, string? Model, string? Serial, uint? BusType, bool? SeekPenalty)
{
    /// <summary><c>STORAGE_BUS_TYPE.BusTypeVirtual</c>.</summary>
    public const uint BusTypeVirtual = 0x0E;

    /// <summary><c>STORAGE_BUS_TYPE.BusTypeFileBackedVirtual</c>.</summary>
    public const uint BusTypeFileBackedVirtual = 0x0F;

    /// <summary><c>STORAGE_BUS_TYPE.BusTypeSpaces</c>: a Storage Spaces virtual disk (ruling R19).</summary>
    public const uint BusTypeSpaces = 0x10;

    /// <summary><c>STORAGE_BUS_TYPE.BusTypeNvme</c>.</summary>
    public const uint BusTypeNvme = 0x11;

    /// <summary>
    /// Controller ruling R17: whether identifying or reading SMART from this drive could spin up
    /// a platter, so it must be known to be active first. Not for a drive without media, a
    /// virtual disk (including Storage Spaces, ruling R19), NVMe or a drive without seek penalty;
    /// for every other drive (seek penalty true or unknown, or unreadable) yes. This only scopes
    /// the gate: for a disk LHM has enumerated, the hub never treats "no media" as "active".
    /// </summary>
    public bool RequiresPowerCheck =>
        Availability != DriveAvailability.NoMedia
        && BusType is not (BusTypeVirtual or BusTypeFileBackedVirtual or BusTypeSpaces or BusTypeNvme)
        && SeekPenalty != false;
}

/// <summary>A drive that keeps the D6 gate closed, with its power-mode answer (standby or unknown).</summary>
public sealed record DriveBlocker(DriveFacts Drive, bool? SpunDown);
