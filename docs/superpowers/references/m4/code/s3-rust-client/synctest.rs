//! Experiments on a *synchronous* (non-overlapped) pipe handle: how to stop a blocked ReadFile.
use std::os::windows::io::AsRawHandle;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::IO::{CancelIoEx, CancelSynchronousIo};

use crate::pipe::{self, OwnedHandle};

fn code(e: &windows::core::Error) -> WIN32_ERROR {
    WIN32_ERROR::from_error(e).unwrap_or(WIN32_ERROR(e.code().0 as u32))
}

fn read_exact(h: HANDLE, buf: &mut [u8]) -> Result<(), WIN32_ERROR> {
    let mut off = 0;
    while off < buf.len() {
        let mut n = 0u32;
        // SAFETY: synchronous read into a live buffer on a live handle.
        unsafe { ReadFile(h, Some(&mut buf[off..]), Some(&mut n), None) }.map_err(|e| code(&e))?;
        off += n as usize;
    }
    Ok(())
}

fn read_frame(h: HANDLE) -> Result<String, WIN32_ERROR> {
    let mut hdr = [0u8; 4];
    read_exact(h, &mut hdr)?;
    let mut body = vec![0u8; u32::from_le_bytes(hdr) as usize];
    read_exact(h, &mut body)?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn write_frame(h: HANDLE, s: &str) -> Result<(), WIN32_ERROR> {
    let mut buf = (s.len() as u32).to_le_bytes().to_vec();
    buf.extend_from_slice(s.as_bytes());
    let mut n = 0u32;
    // SAFETY: synchronous write from a live buffer.
    unsafe { WriteFile(h, Some(&buf), Some(&mut n), None) }.map_err(|e| code(&e))
}

fn open_sync(name: &str) -> Arc<OwnedHandle> {
    let h = pipe::open(name, pipe::CLIENT_ACCESS_GENERIC, false, Duration::from_secs(2))
        .expect("open sync");
    let h = Arc::new(h);
    let hello = read_frame(h.0).expect("hello");
    assert_eq!(hello, "hello");
    // Tell the server to stay quiet so the next read blocks forever.
    write_frame(h.0, "silent").unwrap();
    h
}

/// Spawns a reader that blocks in ReadFile; returns (join handle, receiver of its result).
fn spawn_blocked_reader(
    h: &Arc<OwnedHandle>,
) -> (std::thread::JoinHandle<()>, mpsc::Receiver<(Result<String, WIN32_ERROR>, Instant)>) {
    let (tx, rx) = mpsc::channel();
    let h2 = Arc::clone(h);
    let j = std::thread::spawn(move || {
        let r = read_frame(h2.0);
        let _ = tx.send((r, Instant::now()));
    });
    std::thread::sleep(Duration::from_millis(300)); // let it enter ReadFile
    (j, rx)
}

pub fn run(name: &str) {
    // A. Does a WriteFile from another thread block behind a pending synchronous ReadFile?
    {
        let h = open_sync(name);
        let (_j, rx) = spawn_blocked_reader(&h);
        let (wtx, wrx) = mpsc::channel();
        let h2 = Arc::clone(&h);
        let t0 = Instant::now();
        std::thread::spawn(move || {
            let r = write_frame(h2.0, "ping");
            let _ = wtx.send((r, t0.elapsed()));
        });
        match wrx.recv_timeout(Duration::from_secs(2)) {
            Ok((r, d)) => println!("A. sync WriteFile while ReadFile pending: {r:?} after {d:?} (NOT serialized)"),
            Err(_) => {
                println!("A. sync WriteFile while ReadFile pending: still blocked after 2 s (serialized on the file object)");
                // SAFETY: cancel everything on this handle to unblock the reader, then the writer.
                unsafe {
                    let _ = CancelIoEx(h.0, None);
                }
                let r = wrx.recv_timeout(Duration::from_secs(2));
                println!("   after CancelIoEx: writer -> {r:?}");
            }
        }
        let r = rx.recv_timeout(Duration::from_secs(2));
        println!("   reader -> {:?}", r.map(|x| x.0));
    }

    // B. CancelIoEx(h, NULL) on a synchronous handle from another thread.
    {
        let h = open_sync(name);
        let (j, rx) = spawn_blocked_reader(&h);
        let t0 = Instant::now();
        // SAFETY: live handle; NULL overlapped = cancel all I/O issued on this handle by any thread.
        let c = unsafe { CancelIoEx(h.0, None) };
        let r = rx.recv_timeout(Duration::from_secs(2));
        println!(
            "B. CancelIoEx(sync handle, NULL): call={:?} reader={:?} in {:?}",
            c.map_err(|e| code(&e)),
            r.as_ref().map(|x| &x.0),
            r.as_ref().map(|x| x.1.duration_since(t0))
        );
        let _ = j.join();
        // Race check: CancelIoEx when nothing is pending.
        // SAFETY: as above.
        let c2 = unsafe { CancelIoEx(h.0, None) };
        println!("   CancelIoEx with nothing pending: {:?}", c2.map_err(|e| code(&e)));
    }

    // C. CancelSynchronousIo(thread handle).
    {
        let h = open_sync(name);
        let (j, rx) = spawn_blocked_reader(&h);
        let th = HANDLE(j.as_raw_handle());
        let t0 = Instant::now();
        // SAFETY: th is the live reader thread handle (JoinHandle keeps it open, has THREAD_TERMINATE).
        let c = unsafe { CancelSynchronousIo(th) };
        let r = rx.recv_timeout(Duration::from_secs(2));
        println!(
            "C. CancelSynchronousIo(reader thread): call={:?} reader={:?} in {:?}",
            c.map_err(|e| code(&e)),
            r.as_ref().map(|x| &x.0),
            r.as_ref().map(|x| x.1.duration_since(t0))
        );
        let _ = j.join();
    }

    // D. CloseHandle from another thread while ReadFile is pending (documented as undefined; probe only).
    {
        let h = open_sync(name);
        let raw = h.0 .0 as usize; // HANDLE is !Send; pass the value
        let (_j, rx) = spawn_blocked_reader(&h);
        let (ctx, crx) = mpsc::channel();
        std::thread::spawn(move || {
            let t0 = Instant::now();
            // SAFETY: deliberate probe; the Arc'd OwnedHandle is leaked below so it is not closed twice.
            let r = unsafe { CloseHandle(HANDLE(raw as *mut core::ffi::c_void)) };
            let _ = ctx.send((r.map_err(|e| code(&e)), t0.elapsed()));
        });
        let c = crx.recv_timeout(Duration::from_secs(2));
        let r = rx.recv_timeout(Duration::from_secs(2));
        println!("D. CloseHandle while ReadFile pending: close={c:?} reader={:?}", r.map(|x| x.0));
        std::mem::forget(h); // already closed (or stuck); never close twice
    }
}
