using System.Security.Cryptography;

namespace OpenMonitorAdvanced.Service.Frames;

/// <summary>The PresentMon build this release is pinned to (the installer ships and verifies the same hash).</summary>
internal static class PresentMonPin
{
    /// <summary>Uppercase SHA-256; equal to <c>app/src-tauri/nsis/presentmon.sha256</c>.</summary>
    public const string Sha256 = "B2A706BC6AD475749E3B7E3409263AA1E6906D45BDCF993F6DBC0F660188F1AF";

    public const string FileName = "PresentMon-2.6.0-x64.exe";

    public const string Version = "2.6.0";

    /// <summary>Uppercase SHA-256 of the file, or <see langword="null"/> if it does not exist.</summary>
    public static string? HashOf(string path)
    {
        try
        {
            using FileStream stream = new(path, FileMode.Open, FileAccess.Read, FileShare.Read);
            return Convert.ToHexString(SHA256.HashData(stream));
        }
        catch (FileNotFoundException)
        {
            return null;
        }
        catch (DirectoryNotFoundException)
        {
            return null;
        }
    }
}
