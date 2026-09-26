// Throwaway M4 spike: named pipe server mechanics (ACL, FirstPipeInstance, 8 instances, frames).
using System.Buffers.Binary;
using System.IO.Pipes;
using System.Security.AccessControl;
using System.Text;

const string ProdSddl = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";
// FILE_GENERIC_READ | FILE_WRITE_DATA | FILE_WRITE_ATTRIBUTES | FILE_WRITE_EA == PipeAccessRights.ReadWrite | Synchronize
// i.e. GRGW minus FILE_APPEND_DATA (== FILE_CREATE_PIPE_INSTANCE on a pipe).
const string TightSddl = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x0012019b;;;IU)";

if (args.Length < 2) { Console.Error.WriteLine("usage: serve|probe <name> [prod|tight] [lifetimeSec]"); return 2; }
string mode = args[0], name = args[1];
string sddl = (args.Length > 2 && args[2] == "tight") ? TightSddl : ProdSddl;
int lifetime = args.Length > 3 ? int.Parse(args[3]) : 60;

PipeSecurity MakeSecurity()
{
    var ps = new PipeSecurity();
    ps.SetSecurityDescriptorSddlForm(sddl);
    return ps;
}

NamedPipeServerStream Create(bool first) => NamedPipeServerStreamAcl.Create(
    name, PipeDirection.InOut, 8, PipeTransmissionMode.Byte,
    PipeOptions.Asynchronous | (first ? PipeOptions.FirstPipeInstance : PipeOptions.None),
    64 * 1024, 64 * 1024, MakeSecurity());

string Describe(Exception e) => $"{e.GetType().Name} HResult=0x{e.HResult:X8} (Win32 {e.HResult & 0xFFFF}) \"{e.Message}\"";

if (mode == "probe")
{
    foreach (var first in new[] { true, false })
    {
        try
        {
            using var s = Create(first);
            Console.WriteLine($"PROBE first={first}: created an instance (squat possible)");
        }
        catch (Exception e) { Console.WriteLine($"PROBE first={first}: {Describe(e)}"); }
    }
    return 0;
}

var cts = new CancellationTokenSource(TimeSpan.FromSeconds(lifetime));
_ = Task.Run(() => { Console.In.ReadLine(); cts.Cancel(); }); // any stdin line (or EOF) => quit
var instanceFreed = new SemaphoreSlim(0);
int conn = 0;
NamedPipeServerStream? TryCreate(bool first)
{
    try { return Create(first); }
    catch (IOException e) when ((e.HResult & 0xFFFF) == 231) // ERROR_PIPE_BUSY: 8 instances exist
    {
        Console.WriteLine($"CREATE: max instances reached: {Describe(e)}");
        return null;
    }
    catch (Exception e) { Console.WriteLine($"CREATE next failed: {Describe(e)}"); return null; }
}
NamedPipeServerStream? listening;
try { listening = Create(true); }
catch (Exception e) { Console.WriteLine($"CREATE first failed: {Describe(e)}"); return 1; }
var actual = listening.GetAccessControl().GetSecurityDescriptorSddlForm(AccessControlSections.Access | AccessControlSections.Owner);
Console.WriteLine($"READY pid={Environment.ProcessId} sddl={actual}");
try
{
    while (!cts.IsCancellationRequested)
    {
        await listening!.WaitForConnectionAsync(cts.Token);
        var connected = listening;
        // Create the next listening instance BEFORE the handler can dispose `connected`,
        // so the pipe name never disappears (no NotFound window, no squatting window).
        listening = TryCreate(false);
        int id = Interlocked.Increment(ref conn);
        Console.WriteLine($"CONNECTED #{id}");
        _ = Task.Run(async () =>
        {
            try { await Handle(connected, id, cts.Token); }
            catch (Exception e) { Console.WriteLine($"HANDLER #{id}: {Describe(e)}"); }
            finally { connected.Dispose(); instanceFreed.Release(); Console.WriteLine($"CLOSED #{id}"); }
        });
        while (listening is null)
        {
            await instanceFreed.WaitAsync(cts.Token);
            listening = TryCreate(false);
        }
    }
}
catch (OperationCanceledException) { }
Console.WriteLine("EXIT");
return 0;

static async Task WriteFrame(Stream s, string text, CancellationToken ct)
{
    var payload = Encoding.UTF8.GetBytes(text);
    var buf = new byte[4 + payload.Length];
    BinaryPrimitives.WriteUInt32LittleEndian(buf, (uint)payload.Length);
    payload.CopyTo(buf, 4);
    await s.WriteAsync(buf, ct);
    await s.FlushAsync(ct);
}

static async Task<string?> ReadFrame(Stream s, CancellationToken ct)
{
    var hdr = new byte[4];
    try { await s.ReadExactlyAsync(hdr, ct); } catch (EndOfStreamException) { return null; }
    uint len = BinaryPrimitives.ReadUInt32LittleEndian(hdr);
    if (len > 4 * 1024 * 1024) throw new InvalidDataException("frame too large");
    var body = new byte[len];
    await s.ReadExactlyAsync(body, ct);
    return Encoding.UTF8.GetString(body);
}

static async Task Handle(NamedPipeServerStream pipe, int id, CancellationToken ct)
{
    await WriteFrame(pipe, "hello", ct);
    while (true)
    {
        var msg = await ReadFrame(pipe, ct);
        if (msg is null) { Console.WriteLine($"EOF #{id}"); return; }
        Console.WriteLine($"RECV #{id}: {msg}");
        switch (msg)
        {
            case "bye": pipe.Disconnect(); return;            // server-side disconnect
            case "silent": break;                              // send nothing, keep reading
            default:
                await WriteFrame(pipe, "echo:" + msg, ct);
                for (int i = 1; i <= 3; i++) await WriteFrame(pipe, $"snap-{i}", ct);
                break;
        }
    }
}
