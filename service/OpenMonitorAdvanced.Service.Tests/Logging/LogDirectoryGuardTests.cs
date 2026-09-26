using System.Diagnostics;
using System.Security.AccessControl;
using System.Security.Principal;

using OpenMonitorAdvanced.Service.Logging;

using Xunit;

namespace OpenMonitorAdvanced.Service.Tests.Logging;

/// <summary>
/// The log directory checks (final review C1): the decision logic runs on in-memory security
/// descriptors written as SDDL, the path checks on a temp directory with junctions
/// (<c>mklink /J</c> needs no administrator rights). Nothing here needs elevation.
/// </summary>
public sealed class LogDirectoryGuardTests : IDisposable
{
    private static readonly string Me = WindowsIdentity.GetCurrent().User!.Value;

    /// <summary>What the installer and <see cref="LogDirectoryGuard.CreateProtected"/> set (0x1200a9 = read and execute).</summary>
    private const string InstallerDacl = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";

    private readonly string _root = Path.Combine(Path.GetTempPath(), "oma-logguard-" + Guid.NewGuid().ToString("N"));

    public LogDirectoryGuardTests()
    {
        Directory.CreateDirectory(_root);
    }

    public void Dispose() => DeleteTree(_root);

    private static string? Check(string sddl) => LogDirectoryGuard.CheckSecurity(new RawSecurityDescriptor(sddl));

    [Fact]
    public void TheInstallerAclIsAcceptedWithASystemOrAdministratorsOwner()
    {
        Assert.Null(Check("O:SY" + InstallerDacl));
        Assert.Null(Check("O:BA" + InstallerDacl));
    }

    [Fact]
    public void AnOwnerOtherThanSystemOrAdministratorsIsRefused()
    {
        Assert.Contains("owner", Check($"O:{Me}" + InstallerDacl));
        Assert.Contains("owner", Check("O:BU" + InstallerDacl));
    }

    [Fact]
    public void ANullDaclIsRefused()
    {
        Assert.Contains("no DACL", Check("O:SYD:NO_ACCESS_CONTROL"));
    }

    public static TheoryData<string> WriteRights() =>
    [
        "0x2", // WriteData / CreateFiles
        "0x4", // AppendData / CreateDirectories
        "0x10", // WriteExtendedAttributes
        "0x40", // DeleteSubdirectoriesAndFiles
        "0x100", // WriteAttributes
        "SD", // Delete
        "WD", // ChangePermissions
        "WO", // TakeOwnership
        "0x1301bf", // Modify
        "GW", // GENERIC_WRITE
        "GA", // GENERIC_ALL
        "FA",
    ];

    [Theory]
    [MemberData(nameof(WriteRights))]
    public void AWriteRightForAnUntrustedPrincipalIsRefused(string rights)
    {
        // Users, Authenticated Users, Everyone, Interactive, the current account.
        foreach (string sid in new[] { "BU", "AU", "WD", "IU", Me })
        {
            string? verdict = Check($"O:SYD:P(A;OICI;FA;;;SY)(A;OICI;{rights};;;{sid})");
            Assert.True(verdict is not null, $"{rights} for {sid} must be refused");
        }
    }

    [Fact]
    public void ProgramDatasInheritedUserRightsAreRefusedEvenInheritOnly()
    {
        // C:\ProgramData: BUILTIN\Users:(CI)(WD,AD,WEA,WA), what a child inherits unless protected.
        Assert.NotNull(Check("O:SYD:(A;OICI;FA;;;SY)(A;CIID;0x116;;;BU)"));

        // Inherit-only still reaches the log files created inside.
        Assert.NotNull(Check("O:SYD:P(A;OICI;FA;;;SY)(A;OICIIO;GW;;;BU)"));
    }

    [Fact]
    public void ReadRightsDenyAcesAndCreatorOwnerAreAccepted()
    {
        Assert.Null(Check("O:BAD:P(D;OICI;0x1301bf;;;BU)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICIIO;GA;;;CO)(A;OICI;0x1200a9;;;WD)"));
    }

    [Fact]
    public void MissingDirectoriesAreCreatedWithAnExplicitProtectedDacl()
    {
        string logs = Path.Combine(_root, "OpenMonitorAdvanced", "logs");

        string? verdict = LogDirectoryGuard.Prepare(_root, logs);

        // Unelevated, we own what we created, so the first folder is refused and nothing is
        // created below it. Elevated, the owner is Administrators and both folders pass.
        string[] created = IsElevated() ? [Path.GetDirectoryName(logs)!, logs] : [Path.GetDirectoryName(logs)!];
        Assert.Equal(IsElevated(), Directory.Exists(logs));
        foreach (string dir in created)
        {
            Assert.True(Directory.Exists(dir), dir);
            var raw = new RawSecurityDescriptor(new DirectoryInfo(dir).GetAccessControl(AccessControlSections.Access).GetSecurityDescriptorBinaryForm(), 0);
            Assert.True(raw.ControlFlags.HasFlag(ControlFlags.DiscretionaryAclProtected), $"{dir} must not inherit");
            var aces = raw.DiscretionaryAcl!.Cast<CommonAce>()
                .Select(a => (Sid: a.SecurityIdentifier.Value, a.AccessMask, a.AceFlags, a.AceQualifier))
                .OrderBy(a => a.Sid, StringComparer.Ordinal)
                .ToArray();
            Assert.Equal(
                new[]
                {
                    ("S-1-5-18", 0x1f01ff, AceFlags.ObjectInherit | AceFlags.ContainerInherit, AceQualifier.AccessAllowed),
                    ("S-1-5-32-544", 0x1f01ff, AceFlags.ObjectInherit | AceFlags.ContainerInherit, AceQualifier.AccessAllowed),
                    ("S-1-5-32-545", 0x1200a9, AceFlags.ObjectInherit | AceFlags.ContainerInherit, AceQualifier.AccessAllowed),
                }.OrderBy(a => a.Item1, StringComparer.Ordinal),
                aces);
        }

        if (IsElevated())
        {
            Assert.Null(verdict);
        }
        else
        {
            Assert.Contains("owner", verdict);
        }
    }

    [Fact]
    public void AJunctionAnywhereBelowTheRootIsRefusedAndNeverFollowed()
    {
        string target = Path.Combine(_root, "elsewhere");
        Directory.CreateDirectory(target);
        string oma = Path.Combine(_root, "OpenMonitorAdvanced");
        MakeJunction(oma, target);

        string? verdict = LogDirectoryGuard.Prepare(_root, Path.Combine(oma, "logs"));

        Assert.NotNull(verdict);
        Assert.Contains("junction or link", verdict);
        Assert.Empty(Directory.EnumerateFileSystemEntries(target));
    }

    [Fact]
    public void ARootThatIsAJunctionIsRefused()
    {
        string target = Path.Combine(_root, "real");
        Directory.CreateDirectory(target);
        string root = Path.Combine(_root, "linked-root");
        MakeJunction(root, target);

        string? verdict = LogDirectoryGuard.Prepare(root, Path.Combine(root, "OpenMonitorAdvanced", "logs"));

        Assert.NotNull(verdict);
        Assert.Contains("junction or link", verdict);
        Assert.Empty(Directory.EnumerateFileSystemEntries(target));
    }

    [Fact]
    public void ADirectoryOutsideTheRootIsRefused()
    {
        Assert.NotNull(LogDirectoryGuard.Prepare(Path.Combine(_root, "a"), Path.Combine(_root, "ab", "logs")));
        Assert.NotNull(LogDirectoryGuard.Prepare(Path.Combine(_root, "a"), Path.Combine(_root, "a")));
        Assert.NotNull(LogDirectoryGuard.Prepare(Path.Combine(_root, "a"), Path.Combine(_root, "a", "..", "b")));
        Assert.False(Directory.Exists(Path.Combine(_root, "ab")));
        Assert.False(Directory.Exists(Path.Combine(_root, "b")));
    }

    [Fact]
    public void AFileWhereAFolderShouldBeIsRefused()
    {
        File.WriteAllText(Path.Combine(_root, "OpenMonitorAdvanced"), "not a folder");

        Assert.Contains("not a directory", LogDirectoryGuard.Prepare(_root, Path.Combine(_root, "OpenMonitorAdvanced", "logs")));
    }

    [Fact]
    public void OnlyRegularFilesArePrunable()
    {
        Assert.True(LogDirectoryGuard.IsRegularFile(FileAttributes.Archive));
        Assert.True(LogDirectoryGuard.IsRegularFile(FileAttributes.Normal));
        Assert.False(LogDirectoryGuard.IsRegularFile(FileAttributes.Archive | FileAttributes.ReparsePoint));
        Assert.False(LogDirectoryGuard.IsRegularFile(FileAttributes.Directory));
        Assert.False(LogDirectoryGuard.IsRegularFile(FileAttributes.Directory | FileAttributes.ReparsePoint));
    }

    internal static void MakeJunction(string link, string target)
    {
        using Process mklink = Process.Start(new ProcessStartInfo("cmd.exe", ["/d", "/c", "mklink", "/J", link, target])
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        })!;
        mklink.WaitForExit();
        Assert.True(mklink.ExitCode == 0, $"mklink /J failed: {mklink.StandardError.ReadToEnd()}");
        Assert.True(new DirectoryInfo(link).Attributes.HasFlag(FileAttributes.ReparsePoint));
    }

    /// <summary>
    /// Deletes a test tree: junctions are removed as links first (an unelevated recursive
    /// <see cref="Directory.Delete(string, bool)"/> fails on them), and folders the guard created
    /// with a protected DACL are opened up first (unelevated, we are their owner).
    /// </summary>
    internal static void DeleteTree(string path)
    {
        var info = new DirectoryInfo(path);
        if (!info.Exists)
        {
            return;
        }

        if (info.Attributes.HasFlag(FileAttributes.ReparsePoint))
        {
            Directory.Delete(path);
            return;
        }

        try
        {
            var open = new DirectorySecurity();
            open.SetAccessRuleProtection(isProtected: false, preserveInheritance: false);
            open.AddAccessRule(new FileSystemAccessRule(WindowsIdentity.GetCurrent().User!, FileSystemRights.FullControl, InheritanceFlags.ContainerInherit | InheritanceFlags.ObjectInherit, PropagationFlags.None, AccessControlType.Allow));
            info.SetAccessControl(open);
        }
        catch (UnauthorizedAccessException)
        {
            // Not ours to change: the delete below reports it.
        }

        foreach (string child in Directory.EnumerateDirectories(path))
        {
            DeleteTree(child);
        }

        Directory.Delete(path, recursive: true);
    }

    private static bool IsElevated() => new WindowsPrincipal(WindowsIdentity.GetCurrent()).IsInRole(WindowsBuiltInRole.Administrator);
}
