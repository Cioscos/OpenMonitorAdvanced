using System.Runtime.InteropServices;
using System.Security.AccessControl;
using System.Security.Principal;
using Microsoft.Win32.SafeHandles;

namespace OpenMonitorAdvanced.Service.Logging;

/// <summary>
/// Defence in depth for the LocalSystem service's log directory
/// (<c>%ProgramData%\OpenMonitorAdvanced\logs</c>, final review C1). Any user can create folders
/// in <c>C:\ProgramData</c>, so a folder planted before the installer ran (owned by that user, or
/// with a junction in it) would let the service's writes and prune deletes land anywhere as
/// SYSTEM. The installer creates and locks both folders down (<c>OmaProtectLogDir</c> in
/// <c>app/src-tauri/nsis/oma.nsh</c>); the service still checks before it writes, and gives up
/// file logging rather than follow a path it does not trust.
/// </summary>
internal static class LogDirectoryGuard
{
    private static readonly SecurityIdentifier LocalSystem = new(WellKnownSidType.LocalSystemSid, null);
    private static readonly SecurityIdentifier Administrators = new(WellKnownSidType.BuiltinAdministratorsSid, null);
    private static readonly SecurityIdentifier Users = new(WellKnownSidType.BuiltinUsersSid, null);

    /// <summary>NT SERVICE\TrustedInstaller.</summary>
    private static readonly SecurityIdentifier TrustedInstaller = new("S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464");

    /// <summary>
    /// Stands for whoever creates a child; only trusted principals can create children in a
    /// directory that passes this check, so it never grants anything to anyone else.
    /// </summary>
    private static readonly SecurityIdentifier CreatorOwner = new(WellKnownSidType.CreatorOwnerSid, null);

    /// <summary>
    /// Every right that changes a directory's content, its entry, its attributes or its security
    /// (on files: write, append, delete, attributes, DACL, owner), plus the generic write/all bits
    /// an inherit-only ACE may carry unmapped.
    /// </summary>
    private const int WriteMask =
        (int)(FileSystemRights.WriteData
            | FileSystemRights.AppendData
            | FileSystemRights.WriteExtendedAttributes
            | FileSystemRights.DeleteSubdirectoriesAndFiles
            | FileSystemRights.WriteAttributes
            | FileSystemRights.Delete
            | FileSystemRights.ChangePermissions
            | FileSystemRights.TakeOwnership)
        | 0x40000000 // GENERIC_WRITE
        | 0x10000000; // GENERIC_ALL

    /// <summary>
    /// Makes sure <paramref name="directory"/> may be written by the service: it must lie strictly
    /// inside <paramref name="root"/> (<c>%ProgramData%</c>); the root and every folder below it
    /// down to <paramref name="directory"/> must be a real directory, never a junction or link;
    /// a missing folder is created with an explicit protected DACL (<see cref="CreateProtected"/>);
    /// and every folder below the root must pass <see cref="CheckSecurity(RawSecurityDescriptor)"/>,
    /// so that no other user can rename, replace or fill any of them afterwards. Each folder is
    /// opened without following a reparse point, and its attributes and security are read from
    /// that same handle. Returns <see langword="null"/> when the directory is safe, otherwise the
    /// reason (nothing below a refused folder is touched).
    /// </summary>
    internal static string? Prepare(string root, string directory)
    {
        string fullRoot = Path.TrimEndingDirectorySeparator(Path.GetFullPath(root));
        string fullDirectory = Path.TrimEndingDirectorySeparator(Path.GetFullPath(directory));
        if (!fullDirectory.StartsWith(fullRoot + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase))
        {
            return $"{fullDirectory} is not inside {fullRoot}";
        }

        string? problem = CheckEntry(fullRoot, checkSecurity: false, create: false);
        string current = fullRoot;
        foreach (string part in fullDirectory[(fullRoot.Length + 1)..].Split(Path.DirectorySeparatorChar))
        {
            if (problem is not null)
            {
                break;
            }

            current = Path.Combine(current, part);
            problem = CheckEntry(current, checkSecurity: true, create: true);
        }

        return problem;
    }

    /// <summary>
    /// <see langword="null"/> when the owner is SYSTEM or Administrators, the DACL is present, and no
    /// allow ACE (explicit or inherited, inherit-only included) grants any <see cref="WriteMask"/>
    /// right to a principal other than SYSTEM, Administrators, TrustedInstaller or CREATOR OWNER:
    /// that covers Users, Authenticated Users, Everyone, Interactive and any single account. Deny
    /// ACEs only take rights away and are not needed; an ACE of an unknown type is refused.
    /// </summary>
    internal static string? CheckSecurity(RawSecurityDescriptor descriptor)
    {
        SecurityIdentifier? owner = descriptor.Owner;
        if (owner is null)
        {
            return "the owner cannot be read";
        }

        if (owner != LocalSystem && owner != Administrators)
        {
            return $"owner {owner.Value} is not SYSTEM or Administrators";
        }

        if (!descriptor.ControlFlags.HasFlag(ControlFlags.DiscretionaryAclPresent) || descriptor.DiscretionaryAcl is null)
        {
            return "it has no DACL (everyone has full control)";
        }

        foreach (GenericAce ace in descriptor.DiscretionaryAcl)
        {
            if (ace is not QualifiedAce qualified)
            {
                return $"it has an ACE of unknown type {ace.AceType}";
            }

            if (qualified.AceQualifier != AceQualifier.AccessAllowed)
            {
                continue;
            }

            SecurityIdentifier sid = qualified.SecurityIdentifier;
            if (sid == LocalSystem || sid == Administrators || sid == TrustedInstaller || sid == CreatorOwner)
            {
                continue;
            }

            if ((qualified.AccessMask & WriteMask) != 0)
            {
                return $"{sid.Value} may write, append, delete or change the security of its content (0x{qualified.AccessMask:x})";
            }
        }

        return null;
    }

    /// <summary>
    /// Creates <paramref name="path"/> (its parent must exist) with an explicit protected DACL,
    /// never by inheritance from <c>C:\ProgramData</c>: SYSTEM and Administrators full control,
    /// Users read and execute (to read the logs for a bug report without elevation), all inherited
    /// by files and subfolders. The same ACL the installer sets. Does nothing if it already exists.
    /// </summary>
    internal static void CreateProtected(string path)
    {
        const InheritanceFlags Both = InheritanceFlags.ContainerInherit | InheritanceFlags.ObjectInherit;
        var security = new DirectorySecurity();
        security.SetAccessRuleProtection(isProtected: true, preserveInheritance: false);
        security.AddAccessRule(new FileSystemAccessRule(LocalSystem, FileSystemRights.FullControl, Both, PropagationFlags.None, AccessControlType.Allow));
        security.AddAccessRule(new FileSystemAccessRule(Administrators, FileSystemRights.FullControl, Both, PropagationFlags.None, AccessControlType.Allow));
        security.AddAccessRule(new FileSystemAccessRule(Users, FileSystemRights.ReadAndExecute, Both, PropagationFlags.None, AccessControlType.Allow));
        new DirectoryInfo(path).Create(security);
    }

    /// <summary>A regular file: neither a directory nor a reparse point (link, junction, mount point).</summary>
    internal static bool IsRegularFile(FileAttributes attributes) =>
        (attributes & (FileAttributes.Directory | FileAttributes.ReparsePoint)) == 0;

    /// <summary>
    /// Opens the entry itself (never a link's target), creating it first when it is missing and
    /// <paramref name="create"/> is set, then checks through that one handle that it is a real
    /// directory and, with <paramref name="checkSecurity"/>, that its security passes.
    /// </summary>
    private static string? CheckEntry(string path, bool checkSecurity, bool create)
    {
        SafeFileHandle handle = OpenEntry(path, out int error);
        if (handle.IsInvalid && create && error == ErrorFileNotFound)
        {
            handle.Dispose();
            CreateProtected(path);
            handle = OpenEntry(path, out error);
        }

        using (handle)
        {
            if (handle.IsInvalid)
            {
                return $"{path} cannot be opened (error {error})";
            }

            FileAttributes attributes = File.GetAttributes(handle);
            if (attributes.HasFlag(FileAttributes.ReparsePoint))
            {
                return $"{path} is a junction or link";
            }

            if (!attributes.HasFlag(FileAttributes.Directory))
            {
                return $"{path} is not a directory";
            }

            if (!checkSecurity)
            {
                return null;
            }

            var descriptor = new RawSecurityDescriptor(new HandleSecurity(handle).GetSecurityDescriptorBinaryForm(), 0);
            string? problem = CheckSecurity(descriptor);
            return problem is null ? null : $"{path}: {problem}";
        }
    }

    private const int ErrorFileNotFound = 2;
    private const uint ReadControl = 0x00020000;
    private const uint FileReadAttributes = 0x0080;
    private const uint FileShareAll = 0x7; // read | write | delete: never block the service's own writers
    private const uint OpenExisting = 3;
    private const uint FileFlagBackupSemantics = 0x02000000; // required to open a directory
    private const uint FileFlagOpenReparsePoint = 0x00200000; // the link itself, never its target

    private static SafeFileHandle OpenEntry(string path, out int error)
    {
        SafeFileHandle handle = CreateFileW(path, ReadControl | FileReadAttributes, FileShareAll, IntPtr.Zero, OpenExisting, FileFlagBackupSemantics | FileFlagOpenReparsePoint, IntPtr.Zero);
        error = handle.IsInvalid ? Marshal.GetLastPInvokeError() : 0;
        return handle;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafeFileHandle CreateFileW(string lpFileName, uint dwDesiredAccess, uint dwShareMode, IntPtr lpSecurityAttributes, uint dwCreationDisposition, uint dwFlagsAndAttributes, IntPtr hTemplateFile);

    /// <summary>
    /// Owner and DACL read from an open handle (<c>GetSecurityInfo</c>), so they belong to the very
    /// object whose attributes were checked. Only the binary form is used; the rule factories exist
    /// because <see cref="NativeObjectSecurity"/> requires them.
    /// </summary>
    private sealed class HandleSecurity(SafeHandle handle)
        : NativeObjectSecurity(isContainer: true, ResourceType.FileObject, handle, AccessControlSections.Owner | AccessControlSections.Access)
    {
        public override Type AccessRightType => typeof(FileSystemRights);

        public override Type AccessRuleType => typeof(FileSystemAccessRule);

        public override Type AuditRuleType => typeof(FileSystemAuditRule);

        public override AccessRule AccessRuleFactory(IdentityReference identityReference, int accessMask, bool isInherited, InheritanceFlags inheritanceFlags, PropagationFlags propagationFlags, AccessControlType type) =>
            throw new NotSupportedException();

        public override AuditRule AuditRuleFactory(IdentityReference identityReference, int accessMask, bool isInherited, InheritanceFlags inheritanceFlags, PropagationFlags propagationFlags, AuditFlags flags) =>
            throw new NotSupportedException();
    }
}
