using LibreHardwareMonitor.Hardware;
using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// A read-only view of one node of the LibreHardwareMonitor hardware tree (a
/// <c>Motherboard</c>, <c>Cpu</c>, <c>Memory</c>, <c>Storage</c>, <c>SuperIO</c>,
/// <c>Cooler</c> or <c>Psu</c> hardware), so <see cref="SchemaBuilder"/> is testable
/// without LHM itself or administrator rights. <paramref name="Children"/> holds
/// sub-hardware such as a Super I/O chip under a motherboard.
/// </summary>
public sealed record HardwareNode(
    string Identifier,
    HardwareType Type,
    string Name,
    IReadOnlyList<SensorNode> Sensors,
    IReadOnlyList<HardwareNode> Children,
    StorageInfo? Storage = null);

/// <summary>
/// A read-only view of one LHM sensor. <paramref name="Value"/> is populated only
/// for constants the builder turns into device properties (e.g. the storage
/// "Available Spare Threshold" level).
/// </summary>
public sealed record SensorNode(
    string Identifier,
    SensorType Type,
    string Name,
    int Index,
    float? Value = null);

/// <summary>
/// Storage identity inputs for a <c>Storage</c> hardware node: <paramref name="DriveNumber"/>
/// is the N of <c>\\.\PhysicalDriveN</c>; <paramref name="DescriptorModel"/> and
/// <paramref name="DescriptorSerial"/> come from <see cref="DriveDescriptor"/> reading that
/// same drive (never from DiskInfoToolkit); <paramref name="DriveSerial"/> is LHM's own
/// ATA/NVMe IDENTIFY serial (<c>StorageInfo.DriveSerial</c>, i.e. LHM's <c>SerialNumber</c>);
/// <paramref name="Rotational"/> is informational only for <see cref="SchemaBuilder"/>.
/// </summary>
public sealed record StorageInfo(
    int DriveNumber,
    string? DescriptorModel,
    string? DescriptorSerial,
    string? DriveSerial,
    bool Rotational);

/// <summary>
/// Binds one emitted <see cref="Protocol.WireSensor"/> back to the LHM sensor it was
/// derived from: <c>wire value = LHM value * Scale</c>. <see cref="BuiltSchema.Bindings"/>
/// is index-aligned with <c>Schema.Sensors</c>.
/// </summary>
public sealed record SensorBinding(string LhmIdentifier, double Scale);

/// <summary>The wire schema built from a hardware tree, plus its sensor bindings.</summary>
public sealed record BuiltSchema(SchemaMessage Schema, IReadOnlyList<SensorBinding> Bindings);
