using System.Buffers;
using System.Globalization;
using System.Text.Json;

namespace OpenMonitorAdvanced.Service.Sensors;

/// <summary>The memory type a supported AMD CPU's controller drives.</summary>
public enum DramGeneration
{
    Ddr4,
    Ddr5,
}

/// <summary>The raw UMC registers of one channel, by offset within the channel (<c>0x50000</c>-based).</summary>
public sealed record UmcChannel(int Index, IReadOnlyDictionary<uint, uint> Registers);

/// <summary>
/// The DRAM timings in use, read once from the AMD Zen memory controller (UMC) over SMN and
/// published as the <c>memoryTimings</c> property of <c>memory/0</c>. Register offsets and bit
/// fields follow ZenStates-Core (GPL-3.0, attribution in THIRD_PARTY_NOTICES.md); nothing is
/// copied. Reads only. Written with <see cref="Utf8JsonWriter"/>: the service is trimmed, no reflection.
/// </summary>
public static class UmcTimings
{
    private const int MaxChannels = 12;
    private const uint ChannelStep = 1u << 20;

    /// <summary>Registers a DDR5 decode needs, besides the three presence registers.</summary>
    private static readonly uint[] Ddr5Registers =
    [
        0x5012C, 0x50200, 0x50204, 0x50208, 0x5020C, 0x50210, 0x50214, 0x50218, 0x50230,
        0x50260, 0x50264, 0x50268, 0x5026C,
        0x502C0, 0x502C4, 0x502C8, 0x502CC,
    ];

    /// <summary>Registers a DDR4 decode needs, besides the three presence registers.</summary>
    private static readonly uint[] Ddr4Registers =
    [
        0x5012C, 0x50200, 0x50204, 0x50208, 0x5020C, 0x50210, 0x50214, 0x50218, 0x50230,
        0x50260, 0x50264,
    ];

    /// <summary>The DDR5 tRFC registers hold this value when unused.</summary>
    private const uint UnusedTrfc = 0x00C00138;

    /// <summary>The generation of the supported CPU models; <see langword="null"/> for any other CPU.</summary>
    public static DramGeneration? Generation(CpuInfo cpu)
    {
        if (cpu.Vendor != "AMD")
        {
            return null;
        }

        return (cpu.Family, cpu.Model) switch
        {
            (0x19, >= 0x60 and <= 0x7F) => DramGeneration.Ddr5,
            (0x1A, >= 0x40 and <= 0x4F) => DramGeneration.Ddr5,
            (0x17, 0x71) => DramGeneration.Ddr4,
            (0x19, 0x21) => DramGeneration.Ddr4,
            _ => null,
        };
    }

    /// <summary>
    /// Reads and decodes every channel; <see langword="null"/> for an unsupported CPU (no SMN read
    /// is made then), when no channel gives plausible timings, or when a read failed (see <see cref="TryRead"/>).
    /// </summary>
    public static string? Read(Func<uint, uint?> smn, CpuInfo cpu, Action<string>? log = null)
    {
        TryRead(smn, cpu, out string? json, log);
        return json;
    }

    /// <summary>
    /// Like <see cref="Read"/>, and says whether the answer is final. It is not when any register
    /// a channel needs could not be read (the global mutex timed out, the module failed): then
    /// <paramref name="json"/> is <see langword="null"/> and the caller should try again later,
    /// instead of publishing zeros. A channel the presence bits say is empty or disabled is
    /// final; so is an unsupported CPU.
    /// </summary>
    public static bool TryRead(Func<uint, uint?> smn, CpuInfo cpu, out string? json, Action<string>? log = null)
    {
        json = null;
        if (Generation(cpu) is not { } generation)
        {
            return true;
        }

        uint[] needed = generation == DramGeneration.Ddr5 ? Ddr5Registers : Ddr4Registers;
        var channels = new List<UmcChannel>();
        for (int i = 0; i < MaxChannels; i++)
        {
            uint offset = (uint)i * ChannelStep;
            if (smn(offset | 0x50DF0) is not { } disabled)
            {
                return false;
            }

            if ((disabled >> 19 & 1) != 0)
            {
                continue;
            }

            if (smn(offset | 0x50000) is not { } dimm1 || smn(offset | 0x50008) is not { } dimm2)
            {
                return false;
            }

            if (((dimm1 | dimm2) & 1) == 0)
            {
                continue;
            }

            var registers = new Dictionary<uint, uint>();
            foreach (uint address in needed)
            {
                if (smn(offset | address) is not { } value)
                {
                    return false;
                }

                registers[address] = value;
            }

            channels.Add(new UmcChannel(i, registers));
        }

        json = Decode(channels, generation, log);
        return true;
    }

    /// <summary>
    /// The <c>memoryTimings</c> JSON of the plausible channels (one copy when they all agree,
    /// otherwise a <c>channels</c> array); <see langword="null"/> when none is plausible.
    /// </summary>
    public static string? Decode(IReadOnlyList<UmcChannel> channels, DramGeneration generation, Action<string>? log = null)
    {
        var good = new List<(int Index, List<KeyValuePair<string, object>> Fields)>();
        foreach (UmcChannel channel in channels)
        {
            List<KeyValuePair<string, object>> fields = generation == DramGeneration.Ddr5 ? DecodeDdr5(channel.Registers) : DecodeDdr4(channel.Registers);
            if (Implausible(fields) is { } why)
            {
                log?.Invoke($"UMC channel {channel.Index} dropped from memoryTimings: {why}");
                continue;
            }

            good.Add((channel.Index, fields));
        }

        if (good.Count == 0)
        {
            return null;
        }

        var buffer = new ArrayBufferWriter<byte>();
        using (var w = new Utf8JsonWriter(buffer))
        {
            w.WriteStartObject();
            w.WriteString("generation", generation == DramGeneration.Ddr5 ? "ddr5" : "ddr4");
            if (good.All(g => SameFields(g.Fields, good[0].Fields)))
            {
                WriteFields(w, good[0].Fields);
            }
            else
            {
                w.WriteStartArray("channels");
                foreach ((int index, List<KeyValuePair<string, object>> fields) in good)
                {
                    w.WriteStartObject();
                    w.WriteString("channel", ((char)('A' + index)).ToString());
                    WriteFields(w, fields);
                    w.WriteEndObject();
                }

                w.WriteEndArray();
            }

            w.WriteEndObject();
        }

        return System.Text.Encoding.UTF8.GetString(buffer.WrittenSpan);
    }

    private static List<KeyValuePair<string, object>> DecodeDdr5(IReadOnlyDictionary<uint, uint> r)
    {
        uint clock = Reg(r, 0x50200);
        double mclk = Bits(clock, 15, 0);
        uint refresh = Reg(r, 0x5012C);
        uint fgr = Bits(refresh, 18, 16);
        bool perBank = Bits(refresh, 1, 1) == 1;
        string mode = (perBank, fgr != 0) switch
        {
            (false, false) => "normal",
            (false, true) => "fgr",
            (true, true) => "mixed",
            (true, false) => "pbOnly",
        };

        uint trfcReg = 0;
        foreach (uint address in new uint[] { 0x50260, 0x50264, 0x50268, 0x5026C })
        {
            uint v = Reg(r, address);
            if (v > 0 && v != UnusedTrfc)
            {
                trfcReg = v;
                break;
            }
        }

        uint trfc = Bits(trfcReg, 15, 0);
        uint trfc2 = Bits(trfcReg, 31, 16);
        uint trfcSb = 0;
        foreach (uint address in new uint[] { 0x502C0, 0x502C4, 0x502C8, 0x502CC })
        {
            trfcSb = Bits(Reg(r, address), 10, 0);
            if (trfcSb != 0)
            {
                break;
            }
        }

        uint inUse = mode is "normal" or "pbOnly" ? trfc : trfc2;
        return Fields(
            mclk,
            Bits(clock, 17, 17) == 1,
            Bits(clock, 18, 18) == 1,
            mode,
            Reg(r, 0x50204),
            Reg(r, 0x50208),
            Reg(r, 0x5020C),
            Bits(Reg(r, 0x50210), 7, 0),
            Reg(r, 0x50214),
            Bits(Reg(r, 0x50218), 7, 0),
            Bits(Reg(r, 0x50230), 15, 0),
            trfc,
            trfc2,
            trfcSb,
            inUse);
    }

    private static List<KeyValuePair<string, object>> DecodeDdr4(IReadOnlyDictionary<uint, uint> r)
    {
        uint clock = Reg(r, 0x50200);
        double mclk = Bits(clock, 6, 0) * 100.0 / 3.0;
        uint fgr = Bits(Reg(r, 0x5012C), 18, 16);

        uint t0 = Reg(r, 0x50260);
        uint t1 = Reg(r, 0x50264);
        uint trfcReg = t0 != t1 ? (t0 != 0x21060138 ? t0 : t1) : t0;
        uint trfc = Bits(trfcReg, 10, 0);
        uint trfc2 = Bits(trfcReg, 21, 11);
        uint trfc4 = Bits(trfcReg, 31, 22);

        // The FGR code is the DDR4 MR3 A8:A6 field: 1 and 5 are 2x, 2 and 6 are 4x.
        uint inUse = fgr switch
        {
            0 => trfc,
            1 or 5 => trfc2,
            2 or 6 => trfc4,
            _ => 0,
        };

        uint timing0 = Reg(r, 0x50204);
        uint timing1 = Reg(r, 0x50208);
        return Fields(
            mclk,
            Bits(clock, 10, 10) == 1,
            Bits(clock, 11, 11) == 1,
            fgr == 0 ? "normal" : "fgr",
            timing0,
            timing1,
            Reg(r, 0x5020C),
            Bits(Reg(r, 0x50210), 6, 0),
            Reg(r, 0x50214),
            Bits(Reg(r, 0x50218), 6, 0),
            Bits(Reg(r, 0x50230), 15, 0),
            trfc,
            trfc2,
            0,
            inUse);
    }

    /// <summary>
    /// The channel's fields in publishing order, from the raw registers of the layout the two
    /// generations share (0x50204: CL, tRCD, tRAS; 0x50208: tRP, tRC; 0x5020C; 0x50214).
    /// </summary>
    private static List<KeyValuePair<string, object>> Fields(
        double mclk, bool cmd2t, bool gdm, string mode, uint reg204, uint reg208, uint reg20C,
        uint faw, uint reg214, uint wr, uint refi, uint trfc, uint trfc2, uint trfcSb, uint trfcInUse)
    {
        var f = new List<KeyValuePair<string, object>>
        {
            new("mclkMhz", (long)Math.Round(mclk)),
            new("dataRate", (long)Math.Round(mclk * 2)),
            new("cmd2t", cmd2t),
            new("gdm", gdm),
            new("refreshMode", mode),
            new("cl", (long)Bits(reg204, 5, 0)),
            new("trcdRd", (long)Bits(reg204, 21, 16)),
            new("trcdWr", (long)Bits(reg204, 29, 24)),
            new("trp", (long)Bits(reg208, 21, 16)),
            new("tras", (long)Bits(reg204, 14, 8)),
            new("trc", (long)Bits(reg208, 7, 0)),
            new("trrdS", (long)Bits(reg20C, 4, 0)),
            new("trrdL", (long)Bits(reg20C, 12, 8)),
            new("tfaw", (long)faw),
            new("twtrS", (long)Bits(reg214, 12, 8)),
            new("twtrL", (long)Bits(reg214, 22, 16)),
            new("tcwl", (long)Bits(reg214, 5, 0)),
            new("twr", (long)wr),
            new("trtp", (long)Bits(reg20C, 28, 24)),
            new("trefi", (long)refi),
        };

        // Absent (0) values are left out, like the SPD fields.
        if (trfc != 0)
        {
            f.Add(new("trfc", (long)trfc));
        }

        if (trfc2 != 0)
        {
            f.Add(new("trfc2", (long)trfc2));
        }

        if (trfcSb != 0)
        {
            f.Add(new("trfcSb", (long)trfcSb));
        }

        if (trfcInUse != 0 && mclk > 0)
        {
            f.Add(new("trfcNs", Math.Round(trfcInUse / mclk * 1000, 3)));
        }

        return f;
    }

    /// <summary>Why the channel cannot be right, or <see langword="null"/>: CL outside 4-80, MCLK outside 400-5000 MHz.</summary>
    private static string? Implausible(List<KeyValuePair<string, object>> fields)
    {
        long cl = (long)fields.First(p => p.Key == "cl").Value;
        long mclk = (long)fields.First(p => p.Key == "mclkMhz").Value;
        if (cl is < 4 or > 80)
        {
            return $"CL {cl} is outside 4-80";
        }

        return mclk is < 400 or > 5000 ? $"MCLK {mclk} MHz is outside 400-5000" : null;
    }

    private static bool SameFields(List<KeyValuePair<string, object>> a, List<KeyValuePair<string, object>> b) =>
        a.Count == b.Count && a.Zip(b).All(p => p.First.Key == p.Second.Key && p.First.Value.Equals(p.Second.Value));

    private static void WriteFields(Utf8JsonWriter w, List<KeyValuePair<string, object>> fields)
    {
        foreach ((string name, object value) in fields)
        {
            switch (value)
            {
                case long n:
                    w.WriteNumber(name, n);
                    break;
                case double d:
                    w.WriteNumber(name, d);
                    break;
                case bool b:
                    w.WriteBoolean(name, b);
                    break;
                default:
                    w.WriteString(name, Convert.ToString(value, CultureInfo.InvariantCulture));
                    break;
            }
        }
    }

    private static uint Reg(IReadOnlyDictionary<uint, uint> registers, uint address) =>
        registers.TryGetValue(address, out uint v) ? v : 0;

    private static uint Bits(uint value, int hi, int lo) =>
        (uint)((value >> lo) & ((1UL << (hi - lo + 1)) - 1));
}
