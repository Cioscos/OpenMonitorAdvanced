//! The IOCP queue of one disk worker (DC3): its own handle to the test file (from
//! `ReOpenFile`, so its own file object), bound to its own completion port, with one
//! `VirtualAlloc` buffer and one `OVERLAPPED` per slot. Completions come from
//! `GetQueuedCompletionStatusEx`; latencies are taken with `Instant` (QPC on Windows).
//!
//! Soundness: the `OVERLAPPED`s and the buffers are heap allocations that never move while
//! the queue lives. While a slot's I/O is in flight the kernel owns both; the queue touches
//! them only through raw pointers and never hands out a `&mut [u8]` of that slot. `Drop`
//! cancels what is in flight and waits for its completions before anything is freed; if
//! they do not come, the buffers and `OVERLAPPED`s are leaked rather than freed under the
//! kernel.

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use oma_ipc::load::DiskJob;
use windows::Win32::Foundation::{
    GetLastError, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    ReOpenFile, FILE_FLAG_NO_BUFFERING, FILE_FLAG_OVERLAPPED, FILE_FLAG_WRITE_THROUGH,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows::Win32::System::IO::{
    CancelIoEx, CreateIoCompletionPort, GetOverlappedResult, GetQueuedCompletionStatusEx,
    OVERLAPPED, OVERLAPPED_ENTRY,
};

use super::buffer::AlignedBuf;
use super::engine::{used_blocks, DataFileApi, IoDone, IoQueue};
use super::file::{classify_win32, win32_code, DataFile, DiskError, Owned};
use super::offsets::IoReq;

// Compile-time size checks of the FFI structs passed by pointer.
const _: () = assert!(std::mem::size_of::<OVERLAPPED>() == 32);
const _: () = assert!(std::mem::size_of::<OVERLAPPED_ENTRY>() == 32);

/// `ERROR_INVALID_HANDLE`: the queue was asked for a file that is not a `DataFile`.
const ERROR_INVALID_HANDLE: u32 = 6;
/// The longest wait for the completions of cancelled I/Os when the queue is dropped.
const DRAIN_ON_DROP: Duration = Duration::from_secs(5);

// `ReadFile` and `WriteFile` bound by hand: the `windows` wrappers take a slice, and a
// slice of a buffer the kernel writes to after the call returns must not exist.
#[link(name = "kernel32")]
extern "system" {
    fn ReadFile(
        file: HANDLE,
        buffer: *mut c_void,
        to_read: u32,
        read: *mut u32,
        overlapped: *mut OVERLAPPED,
    ) -> i32;
    fn WriteFile(
        file: HANDLE,
        buffer: *const c_void,
        to_write: u32,
        written: *mut u32,
        overlapped: *mut OVERLAPPED,
    ) -> i32;
}

impl DataFileApi for DataFile {
    fn bytes(&self) -> u64 {
        self.bytes
    }

    fn sector(&self) -> u32 {
        self.sector
    }

    fn flush(&self) -> Result<(), DiskError> {
        DataFile::flush(self)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub struct IocpQueue {
    // Field order is drop order: the port and the file close after `Drop::drop` drained.
    port: Owned,
    file: Owned,
    ovs: Box<[UnsafeCell<OVERLAPPED>]>,
    bufs: Vec<AlignedBuf>,
    /// The submission time of each slot's I/O in flight.
    started: Vec<Option<Instant>>,
    in_flight: usize,
    entries: Vec<OVERLAPPED_ENTRY>,
}

// SAFETY: the queue owns its handles, `OVERLAPPED`s and buffers exclusively; kernel handles
// and the I/O on them may be used from any thread. The queue is used by one thread at a
// time (`&mut self`), and the kernel's access to a slot is ordered by its completion.
unsafe impl Send for IocpQueue {}

impl IocpQueue {
    /// A queue of `job.queue` slots on its own handle to `file`, each buffer as large as the
    /// largest block the job uses.
    pub fn new(file: &dyn DataFileApi, job: &DiskJob) -> Result<IocpQueue, DiskError> {
        let data = file
            .as_any()
            .downcast_ref::<DataFile>()
            .ok_or(DiskError::Io(ERROR_INVALID_HANDLE))?;
        let mut flags = FILE_FLAG_OVERLAPPED | FILE_FLAG_NO_BUFFERING;
        if data.write_through {
            flags |= FILE_FLAG_WRITE_THROUGH;
        }
        // SAFETY: `data.handle()` is a live file handle while `data` lives; the new handle is
        // owned by `Owned` below. The share mode admits the main handle's access (read, write,
        // delete) and its pending delete-on-close.
        let h = unsafe {
            ReOpenFile(
                data.handle(),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                flags,
            )
        }
        .map_err(|e| classify_win32(win32_code(&e)))?;
        let file = Owned(h);
        // SAFETY: a new port bound to our own handle; key 0 and one concurrent thread.
        let port = unsafe { CreateIoCompletionPort(file.0, None, 0, 1) }
            .map_err(|e| classify_win32(win32_code(&e)))?;
        let port = Owned(port);
        let slots = usize::from(job.queue.max(1));
        let size = used_blocks(job).1 as usize;
        let mut bufs = Vec::with_capacity(slots);
        for _ in 0..slots {
            let buf = AlignedBuf::new(size)
                .map_err(|e| DiskError::Io(e.raw_os_error().unwrap_or(8) as u32))?;
            bufs.push(buf);
        }
        Ok(IocpQueue {
            port,
            file,
            ovs: (0..slots)
                .map(|_| UnsafeCell::new(OVERLAPPED::default()))
                .collect(),
            bufs,
            started: vec![None; slots],
            in_flight: 0,
            entries: vec![OVERLAPPED_ENTRY::default(); slots],
        })
    }

    /// The slot of an `OVERLAPPED` of ours, from its address.
    fn slot_of(&self, ov: *mut OVERLAPPED) -> Option<usize> {
        let base = self.ovs.as_ptr() as usize;
        let off = (ov as usize).checked_sub(base)?;
        let size = std::mem::size_of::<UnsafeCell<OVERLAPPED>>();
        (off % size == 0 && off / size < self.ovs.len()).then_some(off / size)
    }

    /// Dequeues up to `entries.len()` completions; each gives its slot, bytes or Win32
    /// error, and latency. A timeout gives none.
    fn dequeue(&mut self, timeout_ms: u32, out: &mut Vec<IoDone>) -> Result<(), DiskError> {
        let mut removed = 0u32;
        // SAFETY: `entries` is a live, writable array whose length is passed by the wrapper;
        // `removed` outlives the call. Not alertable.
        let r = unsafe {
            GetQueuedCompletionStatusEx(
                self.port.0,
                &mut self.entries,
                &mut removed,
                timeout_ms,
                false,
            )
        };
        if let Err(e) = r {
            let code = win32_code(&e);
            return if code == WAIT_TIMEOUT.0 {
                Ok(())
            } else {
                Err(DiskError::Io(code))
            };
        }
        let now = Instant::now();
        for i in 0..(removed as usize).min(self.entries.len()) {
            let entry = self.entries[i];
            let Some(slot) = self.slot_of(entry.lpOverlapped) else {
                continue;
            };
            let Some(t0) = self.started[slot].take() else {
                continue;
            };
            self.in_flight -= 1;
            let mut moved = 0u32;
            // SAFETY: the I/O of this slot is complete (its packet was dequeued), so the
            // call reads the final status without waiting; the `OVERLAPPED` is ours.
            let status = unsafe {
                GetOverlappedResult(self.file.0, self.ovs[slot].get(), &mut moved, false)
            };
            out.push(IoDone {
                slot,
                result: status.map(|()| moved).map_err(|e| win32_code(&e)),
                latency_us: now.saturating_duration_since(t0).as_micros() as u64,
            });
        }
        Ok(())
    }
}

impl IoQueue for IocpQueue {
    fn submit(&mut self, req: IoReq, slot: usize) -> Result<(), DiskError> {
        assert!(slot < self.bufs.len(), "slot {slot} out of range");
        assert!(self.started[slot].is_none(), "slot {slot} is in flight");
        assert!(
            req.len as usize <= self.bufs[slot].len(),
            "block over the buffer"
        );
        let ov = self.ovs[slot].get();
        // SAFETY: the slot has no I/O in flight, so nothing else reads or writes its
        // `OVERLAPPED`; it lives in `ovs`, which never moves while the queue lives.
        unsafe {
            *ov = OVERLAPPED::default();
            (*ov).Anonymous.Anonymous.Offset = req.offset as u32;
            (*ov).Anonymous.Anonymous.OffsetHigh = (req.offset >> 32) as u32;
        }
        let buf = self.bufs[slot].as_mut_ptr().cast::<c_void>();
        self.started[slot] = Some(Instant::now());
        // SAFETY: the handle is open for overlapped unbuffered I/O; `buf` is a page-aligned
        // buffer of at least `req.len` bytes and `ov` a zeroed `OVERLAPPED` with the offset,
        // both kept alive and untouched until the completion is dequeued (or `Drop` drained).
        // A null byte count is required with an `OVERLAPPED`.
        let ok = unsafe {
            if req.write {
                WriteFile(self.file.0, buf, req.len, std::ptr::null_mut(), ov)
            } else {
                ReadFile(self.file.0, buf, req.len, std::ptr::null_mut(), ov)
            }
        };
        if ok == 0 {
            // SAFETY: reads the calling thread's last error, set by the call above.
            let e = unsafe { GetLastError() };
            if e != ERROR_IO_PENDING {
                // Failed at once: no completion packet will come.
                self.started[slot] = None;
                return Err(classify_win32(e.0));
            }
        }
        // Pending or done at once: either way a packet comes to the port.
        self.in_flight += 1;
        Ok(())
    }

    fn wait(&mut self, timeout_ms: u32, out: &mut Vec<IoDone>) -> Result<(), DiskError> {
        self.dequeue(timeout_ms, out)
    }

    fn slots(&self) -> usize {
        self.bufs.len()
    }

    fn buffer(&mut self, slot: usize) -> &mut [u8] {
        assert!(self.started[slot].is_none(), "slot {slot} is in flight");
        self.bufs[slot].as_mut_slice()
    }
}

impl Drop for IocpQueue {
    fn drop(&mut self) {
        if self.in_flight == 0 {
            return;
        }
        // SAFETY: cancels every I/O of this process on our own handle; a failure (nothing
        // left to cancel) is harmless.
        unsafe {
            let _ = CancelIoEx(self.file.0, None);
        }
        let deadline = Instant::now() + DRAIN_ON_DROP;
        let mut done = Vec::new();
        while self.in_flight > 0 && Instant::now() < deadline {
            done.clear();
            if self.dequeue(100, &mut done).is_err() {
                break;
            }
        }
        if self.in_flight > 0 {
            tracing::error!(
                in_flight = self.in_flight,
                "cancelled disk I/O never completed: its buffers are leaked"
            );
            std::mem::forget(std::mem::take(&mut self.bufs));
            std::mem::forget(std::mem::take(&mut self.ovs));
        }
    }
}
