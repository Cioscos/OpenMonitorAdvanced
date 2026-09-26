using System;
using System.Buffers;
using System.Collections.Generic;
using System.IO;
using MessagePack;

// Manual, low-level MessagePackWriter encoding — mirrors exactly the field
// order and shapes produced by the Rust side (rmp_serde::to_vec_named with
// serde's `tag = "type", content = "body"` adjacently-tagged enums).
// MessagePackWriter is a mutable struct, so all writer-mutating callbacks use
// a `ref`-taking delegate (declared below, after top-level statements)
// instead of Action<T> (which would copy it and silently discard the writes).

static byte[] Build(WriteFn body)
{
    var buffer = new ArrayBufferWriter<byte>();
    var writer = new MessagePackWriter(buffer);
    body(ref writer);
    writer.Flush();
    return buffer.WrittenSpan.ToArray();
}

static void WriteEnvelope(ref MessagePackWriter w, string type, WriteFn writeBody)
{
    w.WriteMapHeader(2);
    w.Write("type");
    w.Write(type);
    w.Write("body");
    var inner = new ArrayBufferWriter<byte>();
    var iw = new MessagePackWriter(inner);
    writeBody(ref iw);
    iw.Flush();
    w.WriteRaw(inner.WrittenSpan);
}

static void WriteHint(ref MessagePackWriter w, string? kind, WriteFn? writeValue)
{
    if (kind is null)
    {
        w.WriteNil();
        return;
    }
    w.WriteMapHeader(2);
    w.Write("kind");
    w.Write(kind);
    w.Write("value");
    if (writeValue is null)
    {
        w.WriteMapHeader(0);
    }
    else
    {
        writeValue(ref w);
    }
}

static void WriteDevice(ref MessagePackWriter w, string id, string kind, string name, string? vendor,
    IReadOnlyList<(string, string)> properties, string? hintKind, WriteFn? hintValue)
{
    w.WriteMapHeader(6);
    w.Write("id"); w.Write(id);
    w.Write("kind"); w.Write(kind);
    w.Write("name"); w.Write(name);
    w.Write("vendor");
    if (vendor is null) w.WriteNil(); else w.Write(vendor);
    w.Write("properties");
    w.WriteMapHeader(properties.Count);
    foreach (var (k, v) in properties) { w.Write(k); w.Write(v); }
    w.Write("hint");
    WriteHint(ref w, hintKind, hintValue);
}

static void WriteSensor(ref MessagePackWriter w, string id, string deviceId, string kind, string unit,
    string labelKey, string? labelText, string category, string source)
{
    w.WriteMapHeader(8);
    w.Write("id"); w.Write(id);
    w.Write("device_id"); w.Write(deviceId);
    w.Write("kind"); w.Write(kind);
    w.Write("unit"); w.Write(unit);
    w.Write("label_key"); w.Write(labelKey);
    w.Write("label_text");
    if (labelText is null) w.WriteNil(); else w.Write(labelText);
    w.Write("category"); w.Write(category);
    w.Write("source"); w.Write(source);
}

void WriteFile(string dir, string name, byte[] bytes)
{
    var path = Path.Combine(dir, name + "_dotnet.msgpack");
    File.WriteAllBytes(path, bytes);
    Console.WriteLine($"{name}: {bytes.Length} bytes -> {path}");
    Console.WriteLine("  hex: " + Convert.ToHexString(bytes).ToLowerInvariant());
}

bool CompareToRust(string dir, string name)
{
    var rustPath = Path.Combine(dir, name + ".msgpack");
    var dotnetPath = Path.Combine(dir, name + "_dotnet.msgpack");
    var rust = File.ReadAllBytes(rustPath);
    var dotnet = File.ReadAllBytes(dotnetPath);
    bool same = rust.AsSpan().SequenceEqual(dotnet);
    Console.WriteLine($"{name}: rust={rust.Length}B dotnet={dotnet.Length}B IDENTICAL={same}");
    if (!same)
    {
        Console.WriteLine("  rust:   " + Convert.ToHexString(rust).ToLowerInvariant());
        Console.WriteLine("  dotnet: " + Convert.ToHexString(dotnet).ToLowerInvariant());
    }
    return same;
}

string fixturesDir = Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "..", "..", "..", "..", "..", "protocol", "fixtures"));
Directory.CreateDirectory(fixturesDir);
Console.WriteLine($"fixtures dir: {fixturesDir}");

// ---- 1. Hello ----
WriteFile(fixturesDir, "hello", Build((ref MessagePackWriter w) => WriteEnvelope(ref w, "hello", (ref MessagePackWriter bw) =>
{
    bw.WriteMapHeader(2);
    bw.Write("protocol_version"); bw.Write(1u);
    bw.Write("service_version"); bw.Write("0.4.0");
})));

// ---- 2. Subscribe ----
WriteFile(fixturesDir, "subscribe", Build((ref MessagePackWriter w) => WriteEnvelope(ref w, "subscribe", (ref MessagePackWriter bw) =>
{
    bw.WriteMapHeader(1);
    bw.Write("interval_ms"); bw.Write(1000u);
})));

// ---- 3. Schema ----
WriteFile(fixturesDir, "schema", Build((ref MessagePackWriter w) => WriteEnvelope(ref w, "schema", (ref MessagePackWriter bw) =>
{
    bw.WriteMapHeader(2);
    bw.Write("devices");
    bw.WriteArrayHeader(5);
    WriteDevice(ref bw, "cpu/0", "cpu", "Ryzen 9 9950X", null, Array.Empty<(string, string)>(),
        "cpu", (ref MessagePackWriter cw) => { cw.WriteMapHeader(1); cw.Write("index"); cw.Write(0u); });
    WriteDevice(ref bw, "motherboard/0", "motherboard", "Temperatura °C sensor board", "ASUS",
        new List<(string, string)> { ("alpha", "1"), ("mid", "5"), ("zeta", "9") }, null, null);
    WriteDevice(ref bw, "storage/0", "storage", "NVMe SSD", "Samsung", Array.Empty<(string, string)>(),
        "storage", (ref MessagePackWriter cw) =>
        {
            cw.WriteMapHeader(2);
            cw.Write("physical_drive"); cw.Write(0u);
            cw.Write("serial"); cw.Write("S6XPNX0T123456");
        });
    WriteDevice(ref bw, "storage/1", "storage", "Unknown drive", null, Array.Empty<(string, string)>(),
        "storage", (ref MessagePackWriter cw) =>
        {
            cw.WriteMapHeader(2);
            cw.Write("physical_drive"); cw.Write(1u);
            cw.Write("serial"); cw.WriteNil();
        });
    WriteDevice(ref bw, "memory/0", "memory", "DDR5 64GB", null, Array.Empty<(string, string)>(),
        "memory", null);

    bw.Write("sensors");
    bw.WriteArrayHeader(2);
    WriteSensor(ref bw, "cpu/0/temperature/package", "cpu/0", "temperature", "celsius",
        "cpu.temperature.package", null, "temperature", "lhm");
    WriteSensor(ref bw, "motherboard/0/temperature/vrm", "motherboard/0", "temperature", "celsius",
        "motherboard.temperature.vrm", "Temperatura °C VRM", "temperature", "lhm");
})));

// ---- 4. Snapshot ----
WriteFile(fixturesDir, "snapshot", Build((ref MessagePackWriter w) => WriteEnvelope(ref w, "snapshot", (ref MessagePackWriter bw) =>
{
    bw.WriteMapHeader(3);
    bw.Write("seq"); bw.Write(9_876_543_210UL);
    bw.Write("timestamp_ms"); bw.Write(17_000_000_000_123UL);
    bw.Write("values");
    bw.WriteArrayHeader(5);
    bw.Write(45.0);
    bw.Write(-12.5);
    bw.WriteNil();
    bw.Write(0.0);
    bw.Write(100.0);
})));

WriteFile(fixturesDir, "snapshot_empty_values", Build((ref MessagePackWriter w) => WriteEnvelope(ref w, "snapshot", (ref MessagePackWriter bw) =>
{
    bw.WriteMapHeader(3);
    bw.Write("seq"); bw.Write(0UL);
    bw.Write("timestamp_ms"); bw.Write(0UL);
    bw.Write("values");
    bw.WriteArrayHeader(0);
})));

// ---- 5. Error ----
WriteFile(fixturesDir, "error", Build((ref MessagePackWriter w) => WriteEnvelope(ref w, "error", (ref MessagePackWriter bw) =>
{
    bw.WriteMapHeader(2);
    bw.Write("code"); bw.Write("service_unreachable");
    bw.Write("message"); bw.Write("Impossibile connettersi al servizio: Temperatura °C non disponibile");
})));

// ---- NaN / Infinity probe (documentation only, not part of the protocol) ----
var nanProbe = Build((ref MessagePackWriter w) =>
{
    w.WriteMapHeader(3);
    w.Write("seq"); w.Write(1UL);
    w.Write("timestamp_ms"); w.Write(1UL);
    w.Write("values");
    w.WriteArrayHeader(3);
    w.Write(double.NaN);
    w.Write(double.PositiveInfinity);
    w.Write(double.NegativeInfinity);
});
File.WriteAllBytes(Path.Combine(fixturesDir, "_nan_probe_dotnet.msgpack"), nanProbe);
Console.WriteLine("nan_probe (dotnet): " + Convert.ToHexString(nanProbe).ToLowerInvariant());
Console.WriteLine("double.NaN bits = " + BitConverter.DoubleToUInt64Bits(double.NaN).ToString("x16"));

// ---- Compare against Rust-produced fixtures ----
Console.WriteLine();
Console.WriteLine("=== Byte comparison vs Rust fixtures ===");
bool allSame = true;
foreach (var name in new[] { "hello", "subscribe", "schema", "snapshot", "snapshot_empty_values", "error" })
{
    allSame &= CompareToRust(fixturesDir, name);
}

var nanRust = File.ReadAllBytes(Path.Combine(fixturesDir, "_nan_probe_rust.msgpack"));
bool nanSame = nanRust.AsSpan().SequenceEqual(nanProbe);
Console.WriteLine($"nan_probe: rust={nanRust.Length}B dotnet={nanProbe.Length}B IDENTICAL={nanSame}");

Console.WriteLine();
Console.WriteLine(allSame ? "ALL FIXTURES BYTE-IDENTICAL" : "MISMATCH DETECTED");

// ---- Cross-decode check: read Rust-produced hello.msgpack with MessagePackReader ----
Console.WriteLine();
Console.WriteLine("=== Cross-decode: reading Rust's hello.msgpack ===");
{
    var bytes = File.ReadAllBytes(Path.Combine(fixturesDir, "hello.msgpack"));
    var reader = new MessagePackReader(bytes);
    int mapCount = reader.ReadMapHeader();
    string? type = null;
    uint protocolVersion = 0;
    string? serviceVersion = null;
    for (int i = 0; i < mapCount; i++)
    {
        var key = reader.ReadString();
        if (key == "type") { type = reader.ReadString(); }
        else if (key == "body")
        {
            int bodyCount = reader.ReadMapHeader();
            for (int j = 0; j < bodyCount; j++)
            {
                var bkey = reader.ReadString();
                if (bkey == "protocol_version") protocolVersion = reader.ReadUInt32();
                else if (bkey == "service_version") serviceVersion = reader.ReadString();
                else reader.Skip();
            }
        }
        else { reader.Skip(); }
    }
    Console.WriteLine($"decoded: type={type} protocol_version={protocolVersion} service_version={serviceVersion}");
}

// ---- MessagePackSecurity / untrusted-input limits probe ----
Console.WriteLine();
Console.WriteLine("=== MessagePackSecurity probe ===");
var untrusted = MessagePackSecurity.UntrustedData;
Console.WriteLine($"UntrustedData.MaximumObjectGraphDepth = {untrusted.MaximumObjectGraphDepth}");
var trusted = MessagePackSecurity.TrustedData;
Console.WriteLine($"TrustedData.MaximumObjectGraphDepth = {trusted.MaximumObjectGraphDepth}");
var customSecurity = untrusted.WithMaximumObjectGraphDepth(50);
Console.WriteLine($"Custom.WithMaximumObjectGraphDepth(50) = {customSecurity.MaximumObjectGraphDepth}");
var opts = MessagePackSerializerOptions.Standard.WithSecurity(untrusted);
Console.WriteLine($"MessagePackSerializerOptions.Standard.WithSecurity(untrusted) built OK, Security={opts.Security.MaximumObjectGraphDepth}");

// ---- Full decode of Rust's schema.msgpack (nested maps/arrays/hint union) ----
Console.WriteLine();
Console.WriteLine("=== Full decode of Rust's schema.msgpack ===");
{
    var bytes = File.ReadAllBytes(Path.Combine(fixturesDir, "schema.msgpack"));
    var reader = new MessagePackReader(bytes);
    int envCount = reader.ReadMapHeader(); // type, body
    string? envType = null;
    int deviceCount = 0, sensorCount = 0;
    for (int i = 0; i < envCount; i++)
    {
        var key = reader.ReadString();
        if (key == "type") envType = reader.ReadString();
        else if (key == "body")
        {
            int bodyCount = reader.ReadMapHeader();
            for (int j = 0; j < bodyCount; j++)
            {
                var bkey = reader.ReadString();
                if (bkey == "devices")
                {
                    deviceCount = reader.ReadArrayHeader();
                    for (int d = 0; d < deviceCount; d++)
                    {
                        int fieldCount = reader.ReadMapHeader();
                        string? id = null, kind = null, name = null, vendor = null, hintKind = null;
                        for (int f = 0; f < fieldCount; f++)
                        {
                            var fk = reader.ReadString();
                            switch (fk)
                            {
                                case "id": id = reader.ReadString(); break;
                                case "kind": kind = reader.ReadString(); break;
                                case "name": name = reader.ReadString(); break;
                                case "vendor": vendor = reader.TryReadNil() ? null : reader.ReadString(); break;
                                case "properties":
                                    int propCount = reader.ReadMapHeader();
                                    for (int p = 0; p < propCount; p++) { reader.ReadString(); reader.ReadString(); }
                                    break;
                                case "hint":
                                    if (reader.TryReadNil()) { hintKind = null; }
                                    else
                                    {
                                        int hc = reader.ReadMapHeader();
                                        for (int h = 0; h < hc; h++)
                                        {
                                            var hk = reader.ReadString();
                                            if (hk == "kind") hintKind = reader.ReadString();
                                            else reader.Skip();
                                        }
                                    }
                                    break;
                                default: reader.Skip(); break;
                            }
                        }
                        Console.WriteLine($"  device: id={id} kind={kind} name={name} vendor={vendor ?? "(nil)"} hint={hintKind ?? "(nil)"}");
                    }
                }
                else if (bkey == "sensors")
                {
                    sensorCount = reader.ReadArrayHeader();
                    for (int s = 0; s < sensorCount; s++) reader.Skip();
                }
                else { reader.Skip(); }
            }
        }
        else { reader.Skip(); }
    }
    Console.WriteLine($"envelope type={envType}, devices={deviceCount}, sensors={sensorCount}");
}

delegate void WriteFn(ref MessagePackWriter w);
