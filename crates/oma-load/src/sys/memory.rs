//! Page-aligned, zeroed memory for the big kernel buffers (K3, K4, K10), taken straight
//! from the system with `VirtualAlloc` in chunks of at most 1 GiB. A failure is an `Err`,
//! never an abort; `Drop` gives everything back.
//!
//! Design: a `Region` is a list of chunks, each one a contiguous slice. Code that needs
//! one slice (the FFT buffers, at most 128 MiB) uses `as_slice`/`as_mut_slice`, which
//! panics on a multi-chunk region; K10 walks `chunks_mut`. Every chunk starts on a page
//! (4 KiB) boundary, which is what the non-temporal stores of K10 need (32 bytes).

use crate::kernel::KernelError;

/// The largest chunk: more is split over several `VirtualAlloc` calls.
pub const MAX_CHUNK: u64 = 1 << 30;
/// More than any machine can commit (4 TiB): refused at once, so an absurd request does not
/// commit chunk after chunk until the system runs out.
const MAX_TOTAL: u64 = 1 << 42;

struct Chunk {
    ptr: *mut u64,
    words: usize,
}

pub struct Region {
    chunks: Vec<Chunk>,
}

// SAFETY: a `Region` owns its chunks exclusively, like a `Vec<u64>`.
unsafe impl Send for Region {}

#[cfg(windows)]
fn os_alloc(bytes: usize) -> *mut u64 {
    use windows::Win32::System::Memory::{VirtualAlloc, MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE};
    // SAFETY: a null address asks the system to pick one; the arguments are plain values.
    // Committed pages come back zeroed.
    unsafe { VirtualAlloc(None, bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) as *mut u64 }
}

#[cfg(windows)]
fn os_free(ptr: *mut u64, _bytes: usize) {
    use windows::Win32::System::Memory::{VirtualFree, MEM_RELEASE};
    // SAFETY: `ptr` is the base of a block `os_alloc` returned and not yet freed;
    // MEM_RELEASE needs size 0.
    let _ = unsafe { VirtualFree(ptr.cast(), 0, MEM_RELEASE) };
}

#[cfg(not(windows))]
fn layout(bytes: usize) -> std::alloc::Layout {
    std::alloc::Layout::from_size_align(bytes, 4096).unwrap()
}

#[cfg(not(windows))]
fn os_alloc(bytes: usize) -> *mut u64 {
    // SAFETY: `bytes` is not zero (the caller never asks for an empty chunk).
    unsafe { std::alloc::alloc_zeroed(layout(bytes)) as *mut u64 }
}

#[cfg(not(windows))]
fn os_free(ptr: *mut u64, bytes: usize) {
    // SAFETY: `ptr` came from `os_alloc(bytes)` and is freed once.
    unsafe { std::alloc::dealloc(ptr.cast(), layout(bytes)) }
}

impl Region {
    /// `bytes` of zeroed memory, rounded down to a multiple of 64 (at least 64). Failure
    /// is `Insufficient`; the caller turns it into the DA10 reduction.
    pub fn alloc(bytes: u64) -> Result<Self, KernelError> {
        Self::alloc_chunked(bytes, MAX_CHUNK)
    }

    fn alloc_chunked(bytes: u64, max_chunk: u64) -> Result<Self, KernelError> {
        if bytes > MAX_TOTAL {
            return Err(KernelError::Insufficient);
        }
        let mut left = bytes.max(64) & !63;
        let mut region = Region { chunks: Vec::new() };
        while left > 0 {
            let size = left.min(max_chunk);
            let ptr = usize::try_from(size)
                .map(os_alloc)
                .map_err(|_| KernelError::Insufficient)?;
            if ptr.is_null() {
                // `region` drops here and frees the chunks already taken.
                return Err(KernelError::Insufficient);
            }
            region.chunks.push(Chunk {
                ptr,
                words: (size / 8) as usize,
            });
            left -= size;
        }
        Ok(region)
    }

    pub fn len_bytes(&self) -> u64 {
        self.chunks.iter().map(|c| c.words as u64 * 8).sum()
    }

    /// The only chunk; panics when the region has several.
    pub fn as_slice(&self) -> &[u64] {
        assert_eq!(
            self.chunks.len(),
            1,
            "a multi-chunk region has no single slice"
        );
        let c = &self.chunks[0];
        // SAFETY: `ptr` is a live allocation of `words` u64, zeroed or written by us; the
        // lifetime is tied to `&self`, which keeps `Drop` away.
        unsafe { std::slice::from_raw_parts(c.ptr, c.words) }
    }

    /// The only chunk, mutable; panics when the region has several.
    pub fn as_mut_slice(&mut self) -> &mut [u64] {
        assert_eq!(
            self.chunks.len(),
            1,
            "a multi-chunk region has no single slice"
        );
        let c = &self.chunks[0];
        // SAFETY: as in `as_slice`, and `&mut self` makes the slice the only access.
        unsafe { std::slice::from_raw_parts_mut(c.ptr, c.words) }
    }

    /// Every chunk, in address order of allocation.
    pub fn chunks_mut(&mut self) -> impl Iterator<Item = &mut [u64]> {
        // SAFETY: the chunks are disjoint live allocations; `&mut self` bounds the slices.
        self.chunks
            .iter()
            .map(|c| unsafe { std::slice::from_raw_parts_mut(c.ptr, c.words) })
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        for c in &self.chunks {
            os_free(c.ptr, c.words * 8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_alloc_and_free() {
        let mut r = Region::alloc(8 << 20).unwrap();
        assert_eq!(r.len_bytes(), 8 << 20);
        let s = r.as_mut_slice();
        assert!(s.iter().all(|&w| w == 0));
        assert_eq!(s.as_ptr() as usize % 4096, 0);
        s[0] = 1;
        s[s.len() - 1] = 2;
        assert_eq!((r.as_slice()[0], r.as_slice()[(8 << 20) / 8 - 1]), (1, 2));
        drop(r);
        // Several chunks: 3 MiB in chunks of 1 MiB, each chunk page-aligned and usable.
        let mut r = Region::alloc_chunked(3 << 20, 1 << 20).unwrap();
        let chunks: Vec<_> = r.chunks_mut().collect();
        assert_eq!(chunks.len(), 3);
        for c in chunks {
            assert_eq!((c.len(), c.as_ptr() as usize % 4096), (1 << 17, 0));
            c[5] = 7;
        }
        assert_eq!(r.len_bytes(), 3 << 20);
    }
}
