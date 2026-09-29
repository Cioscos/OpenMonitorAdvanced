using System.Security.Cryptography;
using System.Text;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// The wire key of a physical disk (spec M5 §2.8, ruling P8):
/// <c>sha256(trim(model) + "\0" + trim(serial))</c> in lowercase hexadecimal, computed on the
/// texts of the disk's storage device descriptor. The app computes the same value from its own
/// inventory (<c>oma_ipc::drive_key</c>); <c>protocol/fixtures/drive_key.json</c> is the vector
/// both test suites read.
/// </summary>
public static class DriveKey
{
    /// <summary>
    /// Rust's <c>str::trim</c> set (Unicode <c>White_Space</c>), as code points.
    /// <see cref="string.Trim()"/> would also strip U+001C..U+001F, which Rust keeps, so the set
    /// is spelled out to keep the keys equal.
    /// </summary>
    private static readonly char[] WhiteSpace = Array.ConvertAll(
        new[]
        {
            0x0009, 0x000A, 0x000B, 0x000C, 0x000D, 0x0020, 0x0085, 0x00A0, 0x1680,
            0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200A,
            0x2028, 0x2029, 0x202F, 0x205F, 0x3000,
        },
        codePoint => (char)codePoint);

    /// <summary>The key, or <see langword="null"/> when either text is missing or empty after trimming.</summary>
    public static string? Compute(string? model, string? serial)
    {
        string? trimmedModel = model?.Trim(WhiteSpace);
        string? trimmedSerial = serial?.Trim(WhiteSpace);
        if (string.IsNullOrEmpty(trimmedModel) || string.IsNullOrEmpty(trimmedSerial))
        {
            return null;
        }

        byte[] hash = SHA256.HashData(Encoding.UTF8.GetBytes(trimmedModel + "\0" + trimmedSerial));
        return Convert.ToHexStringLower(hash);
    }
}
