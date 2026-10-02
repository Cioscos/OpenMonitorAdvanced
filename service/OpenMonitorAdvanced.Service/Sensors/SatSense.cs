namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>
/// The ATA registers a SCSI/ATA translation layer returns in the sense data of an
/// <c>ATA PASS-THROUGH</c> command sent with <c>CK_COND</c> set. Pure: unit-tested over the bytes
/// captured from a SATA HDD (descriptor format) and from a USB stick (fixed format).
/// </summary>
internal static class SatSense
{
    private const int HeaderLength = 8;
    private const byte ResponseCodeMask = 0x7F; // bit 7 is VALID
    private const byte AtaStatusReturnDescriptor = 0x09;
    private const byte AtaStatusReturnLength = 0x0C;
    private const int FixedFormatLength = 14; // up to the ASCQ
    private const byte AtaPassThroughInformationAvailableAscq = 0x1D; // with ASC 0x00

    /// ATA status and sector count from SAT sense data (descriptor format 0x72/0x73 with a 0x09
    /// descriptor of at least 0x0C bytes, or fixed format 0x70/0x71 with ASC/ASCQ 00/1D).
    /// <see langword="false"/> for anything else, including sense data that declares more bytes
    /// than <paramref name="sense"/> holds.
    internal static bool TryReadRegisters(ReadOnlySpan<byte> sense, out byte status, out byte sectorCount)
    {
        status = 0;
        sectorCount = 0;
        if (sense.Length < HeaderLength)
        {
            return false;
        }

        // Byte 7 is the additional length in both formats; bytes past it are not sense data.
        int declared = HeaderLength + sense[7];
        if (declared > sense.Length)
        {
            return false;
        }

        sense = sense[..declared];
        return (sense[0] & ResponseCodeMask) switch
        {
            0x72 or 0x73 => TryReadDescriptors(sense, out status, out sectorCount),
            0x70 or 0x71 => TryReadFixed(sense, out status, out sectorCount),
            _ => false,
        };
    }

    private static bool TryReadDescriptors(ReadOnlySpan<byte> sense, out byte status, out byte sectorCount)
    {
        status = 0;
        sectorCount = 0;
        int offset = HeaderLength;
        while (offset + 2 <= sense.Length)
        {
            int length = sense[offset + 1];
            ReadOnlySpan<byte> body = sense[(offset + 2)..];
            if (length > body.Length)
            {
                return false; // the descriptor runs past the declared end
            }

            if (sense[offset] == AtaStatusReturnDescriptor && length >= AtaStatusReturnLength)
            {
                // Descriptor bytes: 3 error, 5 sector count, 13 status.
                sectorCount = sense[offset + 5];
                status = sense[offset + 13];
                return true;
            }

            offset += 2 + length;
        }

        return false;
    }

    private static bool TryReadFixed(ReadOnlySpan<byte> sense, out byte status, out byte sectorCount)
    {
        status = 0;
        sectorCount = 0;
        if (sense.Length < FixedFormatLength || sense[12] != 0x00 || sense[13] != AtaPassThroughInformationAvailableAscq)
        {
            return false;
        }

        // The Information field carries the registers: byte 3 error, 4 status, 6 sector count.
        status = sense[4];
        sectorCount = sense[6];
        return true;
    }
}
