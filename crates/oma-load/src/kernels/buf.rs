//! A zeroed `f64` buffer whose data starts at a chosen offset inside the 4 KiB page.
//!
//! Buffers of the same size that a kernel reads and writes together (the matrices of K7,
//! the input, work and twiddle arrays of the FFT) land at the same offset in their page
//! when the heap or `VirtualAlloc` hands them out: a load from one then has the low 12
//! address bits of a recent store to another, and the core stalls on the false
//! dependency (4K aliasing). How often that happens depends on where the heap put the
//! buffers, which differs between processes and runs, and so does the rate: up to 15 %
//! on the FFT at its fixed size. Giving each buffer of a kernel its own `slot` makes the
//! offsets distinct and the same in every run.

use crate::kernel::KernelError;

/// Bytes between the page offsets of two slots: six slots fit in a page with room.
pub const SLOT_BYTES: usize = 640;
const PAGE: usize = 4096;

pub struct OffsetBuf {
    v: Vec<f64>,
    off: usize,
}

impl OffsetBuf {
    /// `len` zeroed values whose first one is at `slot * SLOT_BYTES` bytes into its page;
    /// `Insufficient` when the memory is not there (never aborts).
    pub fn zeroed(len: usize, slot: usize) -> Result<Self, KernelError> {
        let spare = PAGE / 8;
        let mut v = Vec::new();
        v.try_reserve_exact(len.checked_add(spare).ok_or(KernelError::Insufficient)?)
            .map_err(|_| KernelError::Insufficient)?;
        let base = v.as_ptr() as usize % PAGE;
        let want = slot * SLOT_BYTES % PAGE;
        let off = (want + PAGE - base) % PAGE / 8;
        v.resize(off + len, 0.0);
        Ok(Self { v, off })
    }
}

impl std::ops::Deref for OffsetBuf {
    type Target = [f64];
    fn deref(&self) -> &[f64] {
        &self.v[self.off..]
    }
}

impl std::ops::DerefMut for OffsetBuf {
    fn deref_mut(&mut self) -> &mut [f64] {
        &mut self.v[self.off..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_sit_at_their_page_offset() {
        for slot in 0..6 {
            for len in [64usize, 4096, 65536] {
                let b = OffsetBuf::zeroed(len, slot).unwrap();
                assert_eq!(b.len(), len);
                assert_eq!(
                    b.as_ptr() as usize % PAGE,
                    slot * SLOT_BYTES,
                    "slot {slot} len {len}"
                );
                assert!(b.iter().all(|&x| x == 0.0));
            }
        }
    }

    #[test]
    fn the_buffer_is_writable_and_keeps_its_length() {
        let mut b = OffsetBuf::zeroed(10, 3).unwrap();
        b[9] = 1.5;
        assert_eq!(b[9], 1.5);
        assert_eq!(b.len(), 10);
    }
}
