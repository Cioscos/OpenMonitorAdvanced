using System.Globalization;
using OpenMonitorAdvanced.Service.Protocol;

namespace OpenMonitorAdvanced.Service.Frames;

/// <summary>One parsed PresentMon CSV row: the process it belongs to and its wire frame.</summary>
internal sealed record PresentMonRow(uint Pid, string Name, string PresentMode, WireFrame Frame);

/// <summary>
/// Parser for the PresentMon 2.x CSV on stdout (spec M7b SD2/SD3). Columns are found by name, so
/// their order does not matter; the CSV is plain comma-split (PresentMon never quotes). Bad rows are
/// counted in <see cref="Rejected"/> and dropped, never fatal. Not thread safe: one reader owns it.
/// </summary>
internal sealed class PresentMonCsv
{
    internal const int MaxLineLength = 4096;

    private static readonly string[] Required =
    [
        "Application", "ProcessID", "SwapChainAddress", "PresentMode", "TimeInQPC", "MsBetweenPresents",
        "MsBetweenDisplayChange", "MsUntilDisplayed", "MsBetweenAppStart",
    ];

    private int _fieldCount;
    private int _name, _pid, _swapchain, _mode, _qpc, _betweenPresents, _betweenDisplay, _untilDisplayed, _appStart;
    private int _frameType = -1, _pcLatency = -1, _pclFrameId = -1, _gpuBusy = -1;

    /// <summary>Rows dropped so far (too long, wrong field count, bad number or header not read).</summary>
    public long Rejected { get; private set; }

    /// <summary>Reads the header line (BOM and trailing CR tolerated); false names the first missing required column.</summary>
    public bool TryReadHeader(string line, out string? missingColumn)
    {
        line = Clean(line);
        var names = line.Split(',');
        var index = new Dictionary<string, int>(names.Length, StringComparer.Ordinal);
        for (var i = 0; i < names.Length; i++)
        {
            index[names[i].Trim()] = i;
        }

        foreach (var column in Required)
        {
            if (!index.ContainsKey(column))
            {
                missingColumn = column;
                _fieldCount = 0;
                return false;
            }
        }

        int Optional(string name) => index.TryGetValue(name, out var i) ? i : -1;

        _fieldCount = names.Length;
        _name = index["Application"];
        _pid = index["ProcessID"];
        _swapchain = index["SwapChainAddress"];
        _mode = index["PresentMode"];
        _qpc = index["TimeInQPC"];
        _betweenPresents = index["MsBetweenPresents"];
        _betweenDisplay = index["MsBetweenDisplayChange"];
        _untilDisplayed = index["MsUntilDisplayed"];
        _appStart = index["MsBetweenAppStart"];
        _frameType = Optional("FrameType");
        _pcLatency = Optional("MsPCLatency");
        _pclFrameId = Optional("PCLFrameId");
        _gpuBusy = Optional("MsGPUBusy");
        missingColumn = null;
        return true;
    }

    /// <summary>Parses one data row; null (and <see cref="Rejected"/> + 1) when the row is dropped. Blank lines are ignored.</summary>
    public PresentMonRow? ParseRow(string line)
    {
        if (string.IsNullOrWhiteSpace(line))
        {
            return null;
        }

        if (_fieldCount == 0 || line.Length > MaxLineLength)
        {
            return Reject();
        }

        var f = Clean(line).Split(',');
        if (f.Length != _fieldCount)
        {
            return Reject();
        }

        if (!uint.TryParse(f[_pid], NumberStyles.None, CultureInfo.InvariantCulture, out var pid)
            || !ulong.TryParse(f[_qpc], NumberStyles.None, CultureInfo.InvariantCulture, out var qpc)
            || !TryHex(f[_swapchain], out var swapchain)
            || !double.TryParse(f[_betweenPresents], NumberStyles.Float, CultureInfo.InvariantCulture, out var betweenPresents)
            || !TryOptional(f[_betweenDisplay], out var betweenDisplay)
            || !TryOptional(f[_untilDisplayed], out var untilDisplayed)
            || !TryOptional(f[_appStart], out var appStart))
        {
            return Reject();
        }

        double? pcLatency = null, gpuBusy = null;
        ulong? pclFrameId = null;
        if (_pcLatency >= 0 && !TryOptional(f[_pcLatency], out pcLatency))
        {
            return Reject();
        }

        if (_gpuBusy >= 0 && !TryOptional(f[_gpuBusy], out gpuBusy))
        {
            return Reject();
        }

        if (_pclFrameId >= 0 && !IsNull(f[_pclFrameId]))
        {
            if (!ulong.TryParse(f[_pclFrameId], NumberStyles.None, CultureInfo.InvariantCulture, out var id))
            {
                return Reject();
            }

            pclFrameId = id == 0 ? null : id;
        }

        var frame = new WireFrame(
            qpc, swapchain, MapFrameType(_frameType >= 0 ? f[_frameType] : null),
            Displayed: betweenDisplay is not null, betweenPresents, betweenDisplay, untilDisplayed,
            appStart, pcLatency, gpuBusy, pclFrameId);
        return new PresentMonRow(pid, f[_name], f[_mode], frame);
    }

    private PresentMonRow? Reject()
    {
        Rejected++;
        return null;
    }

    private static string Clean(string line) => line.TrimStart('﻿').TrimEnd('\r', '\n');

    private static bool IsNull(string value) => value.Length == 0 || value == "NA";

    private static bool TryOptional(string value, out double? result)
    {
        result = null;
        if (IsNull(value))
        {
            return true;
        }

        if (!double.TryParse(value, NumberStyles.Float, CultureInfo.InvariantCulture, out var d))
        {
            return false;
        }

        result = d;
        return true;
    }

    private static bool TryHex(string value, out ulong result)
    {
        var span = value.AsSpan();
        if (span.StartsWith("0x", StringComparison.OrdinalIgnoreCase))
        {
            span = span[2..];
        }

        return ulong.TryParse(span, NumberStyles.AllowHexSpecifier, CultureInfo.InvariantCulture, out result);
    }

    private static string MapFrameType(string? text) => text switch
    {
        null => "unknown",
        "Application" => "app",
        "Intel XeSS-FG" => "generated_intel_xefg",
        "AMD AFMF" => "generated_amd_afmf",
        _ => "generated_other",
    };
}
