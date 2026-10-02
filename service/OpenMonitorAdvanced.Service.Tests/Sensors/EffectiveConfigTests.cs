using OpenMonitorAdvanced.Service.Protocol;
using OpenMonitorAdvanced.Service.Sensors;
using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Sensors;

/// <summary>
/// <see cref="EffectiveConfig.Compute"/>, the pure aggregation of every subscriber's request
/// (spec M5 §2.8, F2.3), and the translation of a <see cref="SubscribeMessage"/> into a
/// <see cref="FeedRequest"/>.
/// </summary>
public sealed class EffectiveConfigTests
{
    private const string DriveA = "589488fb5895d8b81b82760dc67568e8c99b40a81fafe4240bd45dd1ee614d83";
    private const string DriveB = "0000000000000000000000000000000000000000000000000000000000000001";

    [Fact]
    public void AModuleStaysOnIfAnySubscriberWantsIt()
    {
        EffectiveConfig? config = EffectiveConfig.Compute(
        [
            Requests.Of(1000, ServiceModules.Cpu | ServiceModules.Memory),
            Requests.Of(1000, ServiceModules.Cpu | ServiceModules.Psu),
        ]);

        Assert.NotNull(config);
        Assert.Equal(ServiceModules.All & ~ServiceModules.Cpu, config.Enabled);
    }

    [Fact]
    public void EveryModuleIsOnWhenNobodyDisablesOne()
    {
        EffectiveConfig? config = EffectiveConfig.Compute([Requests.Of(1000)]);

        Assert.Equal(new EffectiveConfig(ServiceModules.All, new HashSet<string>(), new HashSet<string>()), config);
    }

    [Fact]
    public void SmartOfADriveNeedsASubscriberWithStorageOn()
    {
        // B does not disable A's drive, but B has storage off: B's wish does not count.
        EffectiveConfig? onlyStorageSubscriberDecides = EffectiveConfig.Compute(
        [
            Requests.Of(1000, ServiceModules.None, DriveA),
            Requests.Of(1000, ServiceModules.Storage),
        ]);
        Assert.NotNull(onlyStorageSubscriberDecides);
        Assert.True(onlyStorageSubscriberDecides.Enabled.HasFlag(ServiceModules.Storage));
        Assert.Equal([DriveA], onlyStorageSubscriberDecides.SmartDisabledDrives);

        // Two subscribers with storage on: a drive's SMART stays on if either keeps it on.
        EffectiveConfig? eitherKeepsItOn = EffectiveConfig.Compute(
        [
            Requests.Of(1000, ServiceModules.None, DriveA, DriveB),
            Requests.Of(1000, ServiceModules.None, DriveB),
        ]);
        Assert.NotNull(eitherKeepsItOn);
        Assert.Equal([DriveB], eitherKeepsItOn.SmartDisabledDrives);
    }

    [Fact]
    public void WithStorageOffEverywhereNoDriveIsListed()
    {
        EffectiveConfig? config = EffectiveConfig.Compute([Requests.Of(1000, ServiceModules.Storage, DriveA)]);

        Assert.NotNull(config);
        Assert.False(config.Enabled.HasFlag(ServiceModules.Storage));
        Assert.Empty(config.SmartDisabledDrives);
    }

    [Fact]
    public void ADefaultOffDriveIsEnabledIfAnyStorageSubscriberEnablesIt()
    {
        EffectiveConfig? config = EffectiveConfig.Compute(
        [
            Requests.Of(1000).WithSmartOn(DriveA),
            Requests.Of(1000),
            Requests.Of(1000, ServiceModules.Cpu).WithSmartOn(DriveB),
        ]);

        Assert.NotNull(config);
        Assert.Equal([DriveB, DriveA], config.SmartEnabledDrives.Order(StringComparer.Ordinal));
        Assert.Empty(config.SmartDisabledDrives);
        Assert.Empty(EffectiveConfig.Compute([Requests.Of(1000)])!.SmartEnabledDrives);
    }

    [Fact]
    public void EnabledDrivesOfASubscriberWithStorageOffAreIgnored()
    {
        EffectiveConfig? config = EffectiveConfig.Compute(
        [
            Requests.Of(1000, ServiceModules.Storage).WithSmartOn(DriveA),
            Requests.Of(1000).WithSmartOn(DriveB),
        ]);
        Assert.NotNull(config);
        Assert.Equal([DriveB], config.SmartEnabledDrives);

        // With storage off everywhere no drive is listed at all.
        EffectiveConfig? off = EffectiveConfig.Compute([Requests.Of(1000, ServiceModules.Storage).WithSmartOn(DriveA)]);
        Assert.NotNull(off);
        Assert.Empty(off.SmartEnabledDrives);
    }

    [Fact]
    public void EnabledSetsParticipateInEqualityAndStoragePart()
    {
        HashSet<string> none = [];
        var a = new EffectiveConfig(ServiceModules.All, none, new HashSet<string>(StringComparer.Ordinal) { DriveA, DriveB });
        var b = new EffectiveConfig(ServiceModules.All, none, new HashSet<string>(StringComparer.Ordinal) { DriveB, DriveA });

        Assert.Equal(a, b);
        Assert.Equal(a.GetHashCode(), b.GetHashCode());
        Assert.NotEqual(a, b with { SmartEnabledDrives = none });
        Assert.NotEqual(a, b with { SmartEnabledDrives = new HashSet<string> { DriveA } });
        Assert.NotEqual(a.GetHashCode(), (b with { SmartEnabledDrives = none }).GetHashCode());

        // The same key enabled is not the same key disabled.
        var disabled = new EffectiveConfig(ServiceModules.All, new HashSet<string> { DriveA }, none);
        var enabled = new EffectiveConfig(ServiceModules.All, none, new HashSet<string> { DriveA });
        Assert.NotEqual(disabled, enabled);
        Assert.NotEqual(disabled.GetHashCode(), enabled.GetHashCode());

        // The storage worker's part carries both sets, so a newly enabled drive is a new part.
        Assert.Equal(new EffectiveConfig(ServiceModules.Storage, none, new HashSet<string> { DriveA, DriveB }), a.StoragePart);
        Assert.NotEqual(a.StoragePart, (a with { SmartEnabledDrives = none }).StoragePart);
        Assert.Empty(EffectiveConfig.AllOn.SmartEnabledDrives); // a USB disk stays off before any request
    }

    [Fact]
    public void NoSubscribersKeepThePreviousConfiguration()
    {
        EffectiveConfig? previous = EffectiveConfig.Compute([Requests.Of(1000, ServiceModules.None, DriveB).WithSmartOn(DriveA)]);
        Assert.NotNull(previous);
        Assert.Equal([DriveA], previous.SmartEnabledDrives);

        // Nothing replaces it: the caller keeps the previous one, enabled drives included.
        Assert.Null(EffectiveConfig.Compute([]));
    }

    [Fact]
    public void ConfigurationsCompareByContent()
    {
        HashSet<string> none = [];
        var a = new EffectiveConfig(ServiceModules.All, new HashSet<string>(StringComparer.Ordinal) { DriveA, DriveB }, none);
        var b = new EffectiveConfig(ServiceModules.All, new HashSet<string>(StringComparer.Ordinal) { DriveB, DriveA }, none);

        Assert.Equal(a, b);
        Assert.Equal(a.GetHashCode(), b.GetHashCode());
        Assert.NotEqual(a, b with { Enabled = ServiceModules.Cpu });
        Assert.NotEqual(a, b with { SmartDisabledDrives = new HashSet<string>() });
    }

    [Fact]
    public void ASubscribeBecomesARequest()
    {
        var subscribe = new SubscribeMessage(60000, ["memory", "psu"], [DriveA], [DriveB]);

        FeedRequest request = FeedRequest.From(subscribe, intervalMs: 5000);

        Assert.Equal(5000u, request.IntervalMs);
        Assert.Equal(ServiceModules.Memory | ServiceModules.Psu, request.Disabled);
        Assert.Equal([DriveA], request.SmartDisabledDrives);
        Assert.Equal([DriveB], request.SmartEnabledDrives);
    }

    [Fact]
    public void ModuleNamesFollowTheWireOrder()
    {
        Assert.Equal(ProtocolConstants.Modules, ServiceModuleNames.ToWire(ServiceModules.All));
        Assert.Equal(["cpu", "storage", "psu"], ServiceModuleNames.ToWire(ServiceModules.Psu | ServiceModules.Cpu | ServiceModules.Storage));
        Assert.Equal(ServiceModules.All, ServiceModuleNames.Parse(ProtocolConstants.Modules));
    }
}
