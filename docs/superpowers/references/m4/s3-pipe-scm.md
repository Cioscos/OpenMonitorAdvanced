# Spike S3 — Named pipe (.NET server, Rust client) and SCM as non-admin

Date 2026-09-26, Windows 11 Pro 26200, not elevated, Rust 1.90.0, `windows` 0.62.2, .NET SDK 10.0.303.
Throwaway code: `<scratchpad>\m4\pipe-scm\` (`dotnet-server/` = .NET 10 console server, `rust/` = client + SCM probes + in-process fake server). Every process started was stopped; no real service was started or stopped.
Source excerpts kept for reproduction: §1 = the .NET named-pipe server, [`code/s3-dotnet-server/`](code/s3-dotnet-server/) (`Program.cs`, `PipeServer.csproj`); §2–3 = the Rust overlapped client and SCM calls, [`code/s3-rust-client/`](code/s3-rust-client/) (`main.rs`, `pipe.rs`, `scm.rs`, `fake.rs`, `synctest.rs`, `Cargo.toml`).

## 1. Pipe server (.NET)

Full working server: [`code/s3-dotnet-server/Program.cs`](code/s3-dotnet-server/Program.cs).

### Rights: `GRGW` for IU is too broad — it lets IU create pipe instances

Read back from the live pipe (`GetAccessControl().GetSecurityDescriptorSddlForm`):

| DACL requested | Stored ACE for IU | Second process of an IU user, same name |
|---|---|---|
| `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)` (spec) | `0x12019f` | `FirstPipeInstance` → `UnauthorizedAccessException` 0x80070005 (**Win32 5**); **without** `FirstPipeInstance` → **instance created** (squat) |
| `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x0012019b;;;IU)` (tight) | `0x12019b` | both → Win32 5 |

`GW` = `FILE_GENERIC_WRITE` includes `FILE_APPEND_DATA` (0x4), which on a pipe **is** `FILE_CREATE_PIPE_INSTANCE`. With the spec DACL any interactive user can add a server instance of `OpenMonitorAdvanced.Sensors.v1` while the service runs (up to the 8-instance cap) and clients may connect to it. The PID check (§6) would catch it, but there is no reason to grant it.

- **Recommended DACL:** `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x0012019b;;;IU)`. `0x12019b` = `PipeAccessRights.ReadWrite | PipeAccessRights.Synchronize` (ReadData, WriteData, Read/Write attributes and EA, ReadPermissions, Synchronize). **No `CreateNewInstance`.** Synchronize is included; it is harmless and some APIs expect it.
- The client still opens with `GENERIC_READ | GENERIC_WRITE` against this DACL: it worked against both the .NET and the Rust fake server. NPFS does not check `FILE_APPEND_DATA` on client opens. `FILE_GENERIC_READ | FILE_WRITE_DATA` (0x12008b) also works.
- **Additional instances need `FILE_CREATE_PIPE_INSTANCE` on the server's own token as well.** An unprivileged server with the tight DACL cannot create its own 2nd instance: Win32 5. In production SYSTEM has `GA`, so this does not matter. It does matter for tests: see §6.
- `additionalAccessRights` of `NamedPipeServerStreamAcl.Create` is not needed; the server handle gets full access as creator.
- The owner of the pipe is the creating token: SYSTEM in production.

### FirstPipeInstance and the "zero instances" window

- A second process creating the name with `FirstPipeInstance` gets `UnauthorizedAccessException`, HResult 0x80070005 (Win32 5 `ERROR_ACCESS_DENIED`). There is no distinct "already exists" error.
- **Bug found in the first server version:** it created the next listening instance only after handing the connected one to a handler. A fast client finished and the handler disposed its instance before the next one existed. In that gap the pipe name vanished, and the next client got `ERROR_FILE_NOT_FOUND` (2). In the gap **any process can also create the name with `FirstPipeInstance`**.
- **Rule:** create the next listening instance *before* starting the handler of the connected one. If 8 instances already exist (`IOException` HResult 0x800700E7, Win32 231), wait for a handler to free one. The connected instances keep the name alive.
- Even so, the client must treat `NotFound` as "retry later" (5 s per spec), never as fatal.

### Remote clients

- `\\localhost\pipe\<name>` (SMB loopback) **connected** to the .NET server with both DACLs.
- The same open against a Rust `CreateNamedPipeW` server with `PIPE_REJECT_REMOTE_CLIENTS` got Win32 5.
- So `NamedPipeServerStream` (via `NamedPipeServerStreamAcl.Create`) **does not set `PIPE_REJECT_REMOTE_CLIENTS`** (observed; the .NET source was not checked).
- A genuinely remote non-admin has `NU` and not `IU`, so the DACL already denies them. For defence in depth, either:
  - add `(D;;GA;;;NU)` in front (not testable here, because a loopback token is local); or
  - create the handle via P/Invoke `CreateNamedPipeW` with `PIPE_REJECT_REMOTE_CLIENTS` and wrap it with `new NamedPipeServerStream(PipeDirection.InOut, isAsync: true, isConnected: false, SafePipeHandle)`.

  Recommend the P/Invoke route, or at least the NU deny ACE.

### Minimal C# that worked

The project is `net10.0`. Use `net10.0-windows` with a proper TPV in production; the ACL APIs are Windows-only (CA1416).

```csharp
const string Sddl = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x0012019b;;;IU)";
NamedPipeServerStream Create(bool first) {
    var ps = new PipeSecurity(); ps.SetSecurityDescriptorSddlForm(Sddl);
    return NamedPipeServerStreamAcl.Create(name, PipeDirection.InOut, 8, PipeTransmissionMode.Byte,
        PipeOptions.Asynchronous | (first ? PipeOptions.FirstPipeInstance : PipeOptions.None),
        64 * 1024, 64 * 1024, ps);
}
var listening = Create(true);                       // UnauthorizedAccessException (5) => name taken
while (!ct.IsCancellationRequested) {
    await listening!.WaitForConnectionAsync(ct);
    var connected = listening;
    listening = TryCreate(false);                   // BEFORE the handler can dispose `connected`
    _ = Task.Run(async () => { try { await Handle(connected, ct); }
                               finally { connected.Dispose(); instanceFreed.Release(); } });
    while (listening is null) { await instanceFreed.WaitAsync(ct); listening = TryCreate(false); }
}
// TryCreate: catch IOException with (HResult & 0xFFFF) == 231 (8 instances exist) -> null
// Frames: 4-byte LE length + payload; ReadExactlyAsync; EndOfStreamException on header = client gone.
```

Server-side errors seen:

- a write to a client that has gone → `IOException` 0x800700E8 ("Pipe is broken", Win32 232 `ERROR_NO_DATA`);
- a client that closes → `ReadExactlyAsync` throws `EndOfStreamException`.

## 2. Pipe client (Rust)

Full working client: [`code/s3-rust-client/pipe.rs`](code/s3-rust-client/pipe.rs), driven from [`main.rs`](code/s3-rust-client/main.rs); in-process fake server for tests: [`fake.rs`](code/s3-rust-client/fake.rs); synchronous-handle comparison probes: [`synctest.rs`](code/s3-rust-client/synctest.rs).

### Opening and connecting

- **Open:**
  - `CreateFileW(r"\\.\pipe\<name>", GENERIC_READ|GENERIC_WRITE, FILE_SHARE_MODE(0), None, OPEN_EXISTING, FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, None)`;
  - `SECURITY_SQOS_PRESENT` and `SECURITY_IDENTIFICATION` are `FILE_FLAGS_AND_ATTRIBUTES` constants in `Win32_Storage_FileSystem`.
- **Absent pipe:**
  - `CreateFileW` → `ERROR_FILE_NOT_FOUND` (2) in about 40 µs;
  - `WaitNamedPipeW(2000)` on an absent name returns FALSE **immediately**, with `GetLastError` = 2: it does not wait for the pipe to appear. Poll every 5 s as the spec says.
- **All 8 instances busy:**
  - `CreateFileW` → `ERROR_PIPE_BUSY` (231);
  - `WaitNamedPipeW(500)` → FALSE, `ERROR_SEM_TIMEOUT` (121) after 505 ms;
  - when a slot frees 300 ms later, `WaitNamedPipeW` returns and the retried `CreateFileW` succeeds (301 ms total).
  - `WaitNamedPipeW` returning TRUE does not reserve the instance, so loop `CreateFileW` → `PIPE_BUSY` → `WaitNamedPipeW` until a deadline.
- **Server PID:** `GetNamedPipeServerProcessId(h, &mut pid)` returned exactly the .NET `Environment.ProcessId` (matched in every run). This works on the client handle with no extra access right.

### Stopping a blocked reader: measured options

| Option (sync handle unless noted) | Result |
|---|---|
| `WriteFile` from another thread while a sync `ReadFile` is pending | **Blocks.** Sync I/O on one file object is serialized: the write waited until the read was cancelled (2 s watchdog). A sync handle cannot send `Subscribe` while reading. |
| `CancelIoEx(h, NULL)` from another thread | Works: the read fails with `ERROR_OPERATION_ABORTED` (995) in about 20 µs. **Racy:** if the reader is between two `ReadFile` calls, `CancelIoEx` returns `ERROR_NOT_FOUND` (1168) and the next read blocks forever. You need a flag plus a retry loop. |
| `CancelSynchronousIo(thread)` | Works (995, about 27 µs), with the same race. |
| `CloseHandle` from another thread | **CloseHandle itself blocked** (more than 2 s) and the read stayed blocked. Unusable, and a handle-reuse hazard anyway. |
| **Overlapped handle + `WaitForMultipleObjects([io_event, stop_event])`** | **Chosen.** Stop fires → `CancelIoEx(h, &ov)` + `GetOverlappedResult(bWait=TRUE)` → the reader returns `Stopped`; thread joined in 93 µs. No race, because the stop event is level-triggered (manual reset). Writes from another thread proceed while the read is pending (9.9 µs). Writes get a real timeout via the same helper. |

### Disconnect detection (reader in `ReadFile`)

| Event | Read result | Write result |
|---|---|---|
| Server `Disconnect()` / `DisconnectNamedPipe` | `ERROR_PIPE_NOT_CONNECTED` (233) | 233 |
| Server process killed (handle closed) | `ERROR_BROKEN_PIPE` (109), 52 ms after `taskkill` | — |

Map 109, 232 and 233 all to `Disconnected`.

### Minimal Rust that worked (0.62 API)

```rust
let mut flags = SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION | FILE_FLAG_OVERLAPPED;
loop {
    // SAFETY: path is a valid NUL-terminated wide string for the call's duration.
    match unsafe { CreateFileW(&path, GENERIC_READ.0 | GENERIC_WRITE.0, FILE_SHARE_MODE(0), None,
                               OPEN_EXISTING, flags, None) } {
        Ok(h) => break Ok(OwnedHandle(h)),
        Err(e) => match WIN32_ERROR::from_error(&e).unwrap() {
            ERROR_FILE_NOT_FOUND => break Err(NotFound),
            // SAFETY: as above. FALSE + GetLastError: 121 = timed out, 2 = pipe vanished.
            ERROR_PIPE_BUSY => if !unsafe { WaitNamedPipeW(&path, left_ms) }.as_bool() { /* 121 -> Busy, 2 -> NotFound */ },
            other => break Err(Win32(other)),
        },
    }
}

/// One overlapped op waited together with a stop event; on stop/timeout cancel AND wait.
fn ov_io(h: HANDLE, io_ev: &OwnedHandle, stop: Option<&OwnedHandle>, timeout_ms: u32,
         op: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>) -> Result<u32, PipeError> {
    let mut ov = OVERLAPPED { hEvent: io_ev.0, ..Default::default() };   // manual-reset event
    if let Err(e) = op(&mut ov) {
        let c = code(&e);
        if c != ERROR_IO_PENDING { return Err(map_io(c)); }             // 109/232/233 -> Disconnected
        let mut hs = vec![io_ev.0]; if let Some(s) = stop { hs.push(s.0); }
        // SAFETY: all handles are live events.
        let w = unsafe { WaitForMultipleObjects(&hs, false, timeout_ms) };
        if w != WAIT_OBJECT_0 {
            let mut n = 0;
            // SAFETY: cancel only this OVERLAPPED, then wait: the kernel owns ov + buffer until then.
            unsafe { let _ = CancelIoEx(h, Some(&ov)); let _ = GetOverlappedResult(h, &ov, &mut n, true); }
            return Err(if w == WAIT_TIMEOUT { TimedOut } else { Stopped });
        }
    }
    let mut n = 0;
    // SAFETY: completed (event signalled or synchronous success).
    unsafe { GetOverlappedResult(h, &ov, &mut n, false) }.map_err(|e| map_io(code(&e)))?;
    Ok(n)
}
// read_frame: read_exact(4) via ov_io(ReadFile(h, Some(chunk), None, Some(ov))), len <= 4 MiB else FrameTooLarge,
// read_exact(len). write_frame: separate event + OVERLAPPED, ov_io(WriteFile(...)), timeout 1000 ms.
// Handle wrapper needs `unsafe impl Send/Sync` (HANDLE wraps *mut c_void) with SAFETY comments.
```

Round trip verified: client connects → reads `hello` → writes `subscribe` → receives `echo:subscribe` plus `snap-1..3`. Then `silent` (the server stops sending), a 10 s idle period, a concurrent write, the stop event, and the `bye` disconnect.

## 3. SCM (non-admin)

Full working probes: [`code/s3-rust-client/scm.rs`](code/s3-rust-client/scm.rs).

| Call | Result |
|---|---|
| `OpenSCManagerW(null, null, SC_MANAGER_CONNECT)` | OK, 0.45 ms |
| `OpenServiceW("OpenMonitorAdvanced.NoSuchSvc", SERVICE_QUERY_STATUS)` | **1060** `ERROR_SERVICE_DOES_NOT_EXIST` |
| `OpenServiceW("Spooler", SERVICE_QUERY_STATUS)` + `QueryServiceStatusEx(SC_STATUS_PROCESS_INFO)` | OK, state 4 (RUNNING); open plus query 0.13 ms |
| `OpenServiceW("Spooler", SERVICE_START)` / `SERVICE_STOP` / `QUERY_STATUS\|START` | **5 at `OpenServiceW`**. The access check happens at open, all-or-nothing, and never reaches `StartServiceW`. Spooler SD: AU = `CCLCSWLOCRRC` (no RP/WP). |
| `OpenServiceW("EasyAntiCheat_EOS", QUERY_STATUS\|START\|STOP)` | **OK** (handle closed, not started); status (1 STOPPED, pid 0). SD `(A;;RPWPRC;;;WD)…(A;;CCLCSWLOCRRC;;;IU)`: start and stop for Everyone. |
| `OpenServiceW("EABackgroundService", QUERY_STATUS\|START\|STOP)` | **OK** (not started); SD grants IU, SU and BU `CCLCSWRPWPDTLOCRRC`. |
| `OpenServiceW(<either>, SERVICE_CHANGE_CONFIG)` | 5 |
| `StartServiceW` on a running service (`RpcEptMapper`: BU has RP, always running, not stoppable) | **1056** `ERROR_SERVICE_ALREADY_RUNNING` |

This confirms the §2.2 model: with `(A;;RPWP;;;IU)` added to the service SD, plus the default `LC` for IU/AU, a non-admin can open with `QUERY_STATUS|START|STOP` and start and stop the service; `CHANGE_CONFIG` stays denied.

- **Expected `StartServiceW` codes to handle:** 1056 (already running → fine); 1058 (disabled); 1053 (did not respond in time). Poll `QueryServiceStatusEx` for START_PENDING (2) → RUNNING (4), then read `dwProcessId` for the §6 PID check.
- **Stop:** `ControlService(SERVICE_CONTROL_STOP)` (not exercised).
- `SERVICE_STATUS_PROCESS` is 36 bytes (compile-time assert). Pass it to `QueryServiceStatusEx` as a byte view of the struct.

## 4. `windows` 0.62 features

| Feature | Needed for | Already in `oma-win`? |
|---|---|---|
| `Win32_Foundation` | HANDLE, WIN32_ERROR, error codes, CloseHandle | yes |
| `Win32_Security` | `CreateFileW` signature (SECURITY_ATTRIBUTES), `CreateNamedPipeW` | yes |
| `Win32_Storage_FileSystem` | CreateFileW, ReadFile, WriteFile, `SECURITY_SQOS_PRESENT`, `SECURITY_IDENTIFICATION`, `FILE_FLAG_OVERLAPPED`, `PIPE_ACCESS_DUPLEX`, `FILE_FLAG_FIRST_PIPE_INSTANCE` | yes |
| `Win32_System_IO` | OVERLAPPED, CancelIoEx, GetOverlappedResult (also gates ReadFile/WriteFile/ConnectNamedPipe) | yes |
| `Win32_System_Threading` | CreateEventW, SetEvent, WaitForMultipleObjects, INFINITE | yes |
| **`Win32_System_Pipes`** | WaitNamedPipeW, GetNamedPipeServerProcessId; fake server: CreateNamedPipeW, ConnectNamedPipe, DisconnectNamedPipe, `PIPE_*` | **no, add** |
| **`Win32_System_Services`** | OpenSCManagerW, OpenServiceW, QueryServiceStatusEx, StartServiceW, ControlService, CloseServiceHandle, SERVICE_STATUS_PROCESS | **no, add** |
| `Win32_Security_Authorization` | only for an SDDL-based fake server in tests (`ConvertStringSecurityDescriptorToSecurityDescriptorW`; `LocalFree` is in Foundation) | no. Avoidable: the test fake can pass `None` (default DACL). |
| `Win32_System_WindowsProgramming` | `QueryThreadCycleTime`, measurement only | no, not needed |

## 5. Measurements

| What | Value |
|---|---|
| Connect + first frame (`hello`), warm server | min 38 µs, median 87–94 µs |
| Connect + first frame, first connection to a freshly started .NET server | 2.8–41 ms (JIT on the server side) |
| Write → echo round trip | min 26 µs, median 42 µs, max 2.7 ms |
| Reader thread idle 10 s (overlapped read pending) | **0 cycles** (`QueryThreadCycleTime`), 0 ns thread CPU, 0 ns process CPU |
| Stop event → reader thread joined | 93 µs |
| Server kill → client sees 109 | 52 ms |

## 6. Recommendations for production

1. **Pipe DACL:** use `0x0012019b` for IU, not `GRGW`. Update §6 of the spec: "lettura e scrittura dei messaggi" = `ReadWrite | Synchronize`, without `CreateNewInstance`. Consider `PIPE_REJECT_REMOTE_CLIENTS`: P/Invoke `CreateNamedPipeW` + the `SafePipeHandle` constructor, or at least `(D;;GA;;;NU)`.
2. **.NET accept loop:** always create the next listening instance before dispatching the connected one. Wait on a signal when 8 exist (231). The name must never drop to zero instances while the service runs.
3. **Client handle:** overlapped (`FILE_FLAG_OVERLAPPED`). Keep one dedicated reader thread blocking in `WaitForMultipleObjects([read_event, stop_event], INFINITE)`: 0 CPU when idle, clean stop. Do not use sync handles:
   - writes serialize behind the pending read;
   - `CancelIoEx`/`CancelSynchronousIo` are racy;
   - `CloseHandle` blocks.
4. **Cancellation:** use a manual-reset stop event per connection. On stop, `CancelIoEx(h, &ov)`, then **always** `GetOverlappedResult(bWait=TRUE)` before the buffer or `OVERLAPPED` goes out of scope. Join the thread, then close the handle.
5. **Timeouts:**
   - writes: 1 s through the same helper, then treat the connection as dead;
   - reads: `INFINITE` plus the stop event. Staleness is handled by the "3 intervals" rule of §6, not by a read timeout;
   - connect: `WaitNamedPipeW` loop bounded to about 1–2 s on `PIPE_BUSY`;
   - `NotFound` means "service not ready" → retry in 5 s.
6. **Error mapping:**
   - 2 → not running;
   - 231/121 → busy;
   - 109/232/233 → disconnected (reconnect after 5 s);
   - 995 → our own stop;
   - frame length > 4 MiB → close.
7. **Order on connect:**
   1. `QueryServiceStatusEx` → RUNNING + pid;
   2. `CreateFileW`;
   3. `GetNamedPipeServerProcessId`;
   4. compare the PIDs; if they differ, close and log;
   5. then read `Hello`.

   The PID check is essential, because the name is squattable whenever the service is stopped.
8. **SCM:** open with `SERVICE_QUERY_STATUS | SERVICE_START | SERVICE_STOP` in one call. ERROR 5 at `OpenServiceW` means the installer ACE is missing, i.e. the service is installed without the IU start right; report that as its own state, not as a crash. 1060 means not installed. 1056 on start is success.
9. **CI testing without admin or .NET:**
   - put a `FakeServer` in `oma-win` tests using `CreateNamedPipeW` (as in [`code/s3-rust-client/fake.rs`](code/s3-rust-client/fake.rs)), with a unique name `OpenMonitorAdvanced.Sensors.test-<pid>-<n>`, serving scripted frames;
   - either use a **single instance reused** with `DisconnectNamedPipe` + `ConnectNamedPipe`, or pass `None` security (default DACL: creator full access), because an unprivileged server with the tight DACL cannot create a 2nd instance;
   - make the pipe name and the "expected server PID" injectable, with `std::process::id()` for the fake, so the PID check is tested both ways;
   - everything above (round trip, disconnect 233, stop, `FirstPipeInstance` → 5, busy → 231/121, absent → 2) ran unprivileged;
   - SCM tests: pure mapping of error codes, plus `#[ignore]` hardware-style tests on `Spooler` / a missing name, which are also safe unprivileged.

## Summary

Everything requested works unprivileged: the ACL'd .NET 10 pipe with `FirstPipeInstance` and 8 instances, and the Rust client with SQOS identification, busy/absent handling, the server-PID check and framed round trip. The SCM probes all behave as expected: 1060, 5 at `OpenServiceW`, 1056, and non-admin open with `START|STOP` succeeds where the SD grants RP/WP.

Three corrections to the design:

- `GRGW` for IU includes `FILE_CREATE_PIPE_INSTANCE`, which lets users squat instances; use `0x12019b`.
- The server must never let the instance count reach zero.
- .NET does not reject remote clients by default.

Recommended reader: an overlapped handle with a stop event. It measured 0 CPU when idle, a 93 µs clean stop and concurrent writes, while every sync-handle cancellation route is racy or blocks. Warm connect plus first frame is about 90 µs, and the round trip about 42 µs. Add the `windows` features `Win32_System_Pipes` and `Win32_System_Services` to `oma-win`; the rest are already enabled.
