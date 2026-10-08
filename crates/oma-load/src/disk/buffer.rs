//! Page-aligned buffers for unbuffered I/O (`FILE_FLAG_NO_BUFFERING` wants sector alignment;
//! `VirtualAlloc` gives 4 KiB at least).

use std::io;
use windows::Win32::System::Memory::{
    VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};

pub struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
}

// SAFETY: the buffer owns its allocation exclusively; nothing in it is tied to a thread.
unsafe impl Send for AlignedBuf {}

impl AlignedBuf {
    pub fn new(bytes: usize) -> io::Result<AlignedBuf> {
        if bytes == 0 {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        // SAFETY: a null base address asks the system to choose; the size is non-zero.
        let p = unsafe { VirtualAlloc(None, bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        if p.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(AlignedBuf {
            ptr: p.cast(),
            len: bytes,
        })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: `ptr` is a live committed allocation of `len` bytes (zeroed by the system),
        // exclusively borrowed through `&mut self`.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `VirtualAlloc` with MEM_RESERVE and is released once; with
        // MEM_RELEASE the size must be 0.
        unsafe {
            let _ = VirtualFree(self.ptr.cast(), 0, MEM_RELEASE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_is_aligned_zeroed_and_writable() {
        let mut b = AlignedBuf::new(1 << 20).unwrap();
        assert_eq!(b.as_ptr() as usize % 4096, 0);
        assert!(b.as_mut_slice().iter().all(|&x| x == 0));
        b.as_mut_slice()[100] = 7;
        assert_eq!(b.as_mut_slice()[100], 7);
        assert!(AlignedBuf::new(0).is_err());
    }
}
