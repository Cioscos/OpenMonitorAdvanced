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
/// <see cref="Roots"/> may be read from both threads. <see cref="Open"/> and
/// <see cref="SetModules"/> run on the sampler, the latter only while the storage worker is
/// parked (spec M5 §2.8: no group is closed while another thread updates hardware);
/// <see cref="EnableStorage"/> runs on the storage worker.
/// </remarks>
public interface IHardwareTree : IDisposable
{
    /// <summary>
    /// Opens the tree with the <paramref name="enabled"/> groups of <see cref="HardwareModules.TreeGroups"/>
    /// only, so a group switched off before the first tick is never built. Storage stays disabled
    /// whatever <paramref name="enabled"/> says (decision D6), so opening never enumerates or
    /// identifies a disk. Returns the roots, like <see cref="Roots"/>.
    /// </summary>
    IReadOnlyList<HardwareNode> Open(ServiceModules enabled);

    /// <summary>
    /// Adds and removes the groups of <see cref="HardwareModules.TreeGroups"/> so that exactly the
    /// <paramref name="enabled"/> ones are open; storage is never touched (P10). The membership is
    /// reconciled before returning, so <see cref="Roots"/> never lists hardware of a closed group.
    /// A group that fails to load throws, and is attempted again by the next call.
    /// </summary>
    void SetModules(ServiceModules enabled);

    /// <summary>The current roots (after hardware added/removed or sensors activated/deactivated).</summary>
    IReadOnlyList<HardwareNode> Roots { get; }

    /// <summary>Updates that hardware and its sub-hardware. May throw.</summary>
    void Update(HardwareNode root);

    /// <summary>The current raw LHM value of that sensor; <see langword="null"/> when unknown or null.</summary>
    double? Read(string sensorIdentifier);

    /// <summary>
    /// The NVMe Critical Warning byte (log page 02h, byte 0) of that storage hardware, from
    /// DiskInfoToolkit's SMART attributes as the last <see cref="Update"/> left them: no command
    /// is sent. <see langword="null"/> when the hardware is unknown or has no such attribute.
    /// Called from the storage worker only, right after the storage root's update.
    /// </summary>
    byte? ReadNvmeCriticalWarning(string storageIdentifier);

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

/// <summary>
/// Disk power state checks for decision D6, without LHM and without waking a disk. The power
/// command is not free, though: every one resets Windows' idle timer of that disk, and it powers
/// up a disk Windows turned off (design M6b §2), so the hub sends it only by the rules of
/// <see cref="GateEpisode"/> and <see cref="DiskActivity.Check"/>.
/// </summary>
public interface IDiskPowerProbe
{
    /// <summary>
    /// ATA CHECK POWER MODE on <c>\\.\PhysicalDriveN</c>: <see langword="true"/> in standby,
    /// <see langword="null"/> when unknown. <paramref name="model"/> and <paramref name="serial"/>
    /// are the drive's descriptor identity: they tell the implementation when another disk has
    /// taken the drive number, so nothing it remembers about how to ask is carried over.
    /// </summary>
    bool? IsSpunDown(int driveNumber, string? model, string? serial);

    /// <summary>
    /// Every <c>PhysicalDriveN</c> as <see cref="Describe"/> sees it (access 0): no power command,
    /// no SMART, nothing that could wake a disk.
    /// </summary>
    IReadOnlyList<DriveFacts> Enumerate();

    /// <summary>
    /// Model, serial, bus type and seek penalty of <c>\\.\PhysicalDriveN</c>, read with access 0
    /// (never wakes a disk); <see langword="null"/> when there is no such drive. May block on a
    /// slow device: the hub calls it only from its storage worker.
    /// </summary>
    DriveFacts? Describe(int driveNumber);
}

/// <summary>
/// The two passive sources about a <c>PhysicalDriveN</c> (design M6b §4.4), both on a handle
/// opened with access 0: neither wakes a disk, powers one up or resets Windows' disk idle timer.
/// </summary>
public interface IDiskActivityProbe
{
    /// <summary>The driver's read and write counters (<c>IOCTL_DISK_PERFORMANCE</c>); <see langword="null"/> when they cannot be read.</summary>
    DiskCounters? Read(int driveNumber);

    /// <summary>
    /// Whether Windows has the disk powered (<c>GetDevicePowerState</c>) and, when
    /// <paramref name="withCounters"/>, its counters as <see cref="Read"/> gives them, from one
    /// open of the drive.
    /// </summary>
    DiskSample Sample(int driveNumber, bool withCounters);
}

/// <summary>How many reads and writes a disk's driver has completed.</summary>
public readonly record struct DiskCounters(long ReadCount, long WriteCount);

/// <summary>One look at a disk through the passive sources; each part is <see langword="null"/> when its call fails (or was not asked for).</summary>
public readonly record struct DiskSample(bool? PoweredOn, DiskCounters? Counters);

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
    /// <summary><c>STORAGE_BUS_TYPE.BusTypeUsb</c>.</summary>
    public const uint BusTypeUsb = 0x07;

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

    /// <summary>
    /// A USB disk: its SMART stays off until a client asks for it (a bridge may hide its disk's
    /// standby, so periodic commands could keep it awake).
    /// </summary>
    public bool SmartOffByDefault => BusType == BusTypeUsb;
}

/// <summary>
/// A drive as a storage round saw it: whether it was <paramref name="Asked"/> for its power mode
/// and what it answered (<see langword="true"/> in standby, <see langword="null"/> unknown or not asked).
/// </summary>
public sealed record DriveCheck(DriveFacts Drive, bool Asked, bool? SpunDown)
{
    /// <summary>Windows reports the disk off: it is in standby, and nothing is sent to it.</summary>
    public bool PoweredOff { get; init; }

    /// <summary>Its power mode matters, Windows reports it on and nothing was sent to it, for lack of recent activity.</summary>
    public bool Idle { get; init; }

    /// <summary>Whether it keeps the gate closed: its power mode matters and it is not known to be active.</summary>
    public bool Blocks => PoweredOff || Idle || (Asked && SpunDown != false);

    /// <summary>Whether its values of the round before stay, as held: left alone (off or idle) or in confirmed standby.</summary>
    public bool Rests => PoweredOff || Idle || (Asked && SpunDown == true);

    /// <summary>Its <see cref="DriveKey"/> from the descriptor model and serial; <see langword="null"/> when either is missing.</summary>
    public string? Key => DriveKey.Compute(Drive.Model, Drive.Serial);
}
