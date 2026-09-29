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

        Assert.Equal(new EffectiveConfig(ServiceModules.All, new HashSet<string>()), config);
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
    public void NoSubscribersKeepsTheLastConfiguration() =>
        Assert.Null(EffectiveConfig.Compute([]));

    [Fact]
    public void ConfigurationsCompareByContent()
    {
        var a = new EffectiveConfig(ServiceModules.All, new HashSet<string>(StringComparer.Ordinal) { DriveA, DriveB });
        var b = new EffectiveConfig(ServiceModules.All, new HashSet<string>(StringComparer.Ordinal) { DriveB, DriveA });

        Assert.Equal(a, b);
        Assert.Equal(a.GetHashCode(), b.GetHashCode());
        Assert.NotEqual(a, b with { Enabled = ServiceModules.Cpu });
        Assert.NotEqual(a, b with { SmartDisabledDrives = new HashSet<string>() });
    }

    [Fact]
    public void ASubscribeBecomesARequest()
    {
        var subscribe = new SubscribeMessage(60000, ["memory", "psu"], [DriveA]);

        FeedRequest request = FeedRequest.From(subscribe, intervalMs: 5000);

        Assert.Equal(5000u, request.IntervalMs);
        Assert.Equal(ServiceModules.Memory | ServiceModules.Psu, request.Disabled);
        Assert.Equal([DriveA], request.SmartDisabledDrives);
    }

    [Fact]
    public void ModuleNamesFollowTheWireOrder()
    {
        Assert.Equal(ProtocolConstants.Modules, ServiceModuleNames.ToWire(ServiceModules.All));
        Assert.Equal(["cpu", "storage", "psu"], ServiceModuleNames.ToWire(ServiceModules.Psu | ServiceModules.Cpu | ServiceModules.Storage));
        Assert.Equal(ServiceModules.All, ServiceModuleNames.Parse(ProtocolConstants.Modules));
    }
}
