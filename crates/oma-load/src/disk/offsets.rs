//! Which block the next I/O touches (DC3): sequential striping per thread, random aligned
//! offsets, and the read/write choice. Pure, so the engine and the tests share it.

use crate::rng::Xoshiro256ss;
use oma_ipc::load::DiskJob;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoReq {
    pub offset: u64,
    pub len: u32,
    pub write: bool,
}

pub struct IoPicker {
    rng: Xoshiro256ss,
    file_bytes: u64,
    block: u64,
    seq_block: u64,
    random_percent: u64,
    read_percent: u64,
    cursor: u64,
}

impl IoPicker {
    /// `thread` is the worker index; its sequential cursor starts at `thread * file / threads`,
    /// aligned down to the sequential block. The random stream is seeded `seed ^ thread`.
    pub fn new(job: &DiskJob, file_bytes: u64, thread: u16, seed: u64) -> Self {
        let block = u64::from(job.block_bytes.max(1));
        let seq_block = u64::from(job.seq_block_bytes.max(1));
        let threads = u64::from(job.threads.max(1));
        let start = (u128::from(file_bytes) * u128::from(thread) / u128::from(threads)) as u64;
        Self {
            rng: Xoshiro256ss::new(seed ^ u64::from(thread)),
            file_bytes,
            block,
            seq_block,
            random_percent: u64::from(job.random_percent),
            read_percent: u64::from(job.read_percent),
            cursor: start / seq_block * seq_block,
        }
    }

    #[allow(clippy::should_implement_trait)] // the stream never ends; not an Iterator
    pub fn next(&mut self) -> IoReq {
        let random = self.rng.next_u64() % 100 < self.random_percent;
        let write = self.rng.next_u64() % 100 >= self.read_percent;
        if random {
            let blocks = (self.file_bytes / self.block).max(1);
            IoReq {
                offset: self.rng.next_u64() % blocks * self.block,
                len: self.block as u32,
                write,
            }
        } else {
            if self.cursor + self.seq_block > self.file_bytes {
                self.cursor = 0;
            }
            let offset = self.cursor;
            self.cursor += self.seq_block;
            IoReq {
                offset,
                len: self.seq_block as u32,
                write,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_ipc::load::DiskJob;

    const MIB: u64 = 1 << 20;

    fn job(block: u32, seq: u32, random: u8, read: u8, threads: u16) -> DiskJob {
        DiskJob {
            block_bytes: block,
            seq_block_bytes: seq,
            random_percent: random,
            read_percent: read,
            queue: 1,
            threads,
            write_cap_bytes: None,
            cycles: None,
            rate_limit_bps: None,
        }
    }

    #[test]
    fn sequential_offsets_stripe_by_thread_and_wrap() {
        let j = job(4096, 1 << 20, 0, 100, 2);
        let mut p0 = IoPicker::new(&j, 64 * MIB, 0, 7);
        let mut p1 = IoPicker::new(&j, 64 * MIB, 1, 7);
        assert_eq!(p0.next().offset, 0);
        let first = p1.next();
        assert_eq!(first.offset, 32 * MIB);
        assert_eq!(first.len, 1 << 20);
        for _ in 0..31 {
            p1.next();
        }
        assert_eq!(p1.next().offset, 0);
    }

    #[test]
    fn random_offsets_are_aligned_and_in_range() {
        let j = job(4096, 1 << 20, 100, 50, 1);
        let mut p = IoPicker::new(&j, 64 * MIB, 0, 1);
        for _ in 0..5000 {
            let r = p.next();
            assert_eq!(r.len, 4096);
            assert_eq!(r.offset % 4096, 0);
            assert!(r.offset + 4096 <= 64 * MIB);
        }
    }

    #[test]
    fn read_percent_is_respected() {
        let j = job(4096, 1 << 20, 100, 70, 1);
        let mut p = IoPicker::new(&j, 64 * MIB, 0, 3);
        let reads = (0..10_000).filter(|_| !p.next().write).count();
        assert!((6800..=7200).contains(&reads), "reads {reads}");
    }

    #[test]
    fn mixed_job_uses_both_block_sizes() {
        // N1 (DC7): 4 KiB random, 128 KiB sequential, 50 % random, 70 % reads.
        let j = job(4096, 128 << 10, 50, 70, 2);
        let mut p = IoPicker::new(&j, 64 * MIB, 1, 9);
        let (mut small, mut large) = (0, 0);
        for _ in 0..2000 {
            let r = p.next();
            match r.len {
                4096 => {
                    small += 1;
                    assert_eq!(r.offset % 4096, 0);
                }
                131_072 => {
                    large += 1;
                    assert_eq!(r.offset % 131_072, 0);
                }
                n => panic!("unexpected length {n}"),
            }
            assert!(r.offset + u64::from(r.len) <= 64 * MIB);
        }
        assert!(small > 800 && large > 800, "{small} {large}");
    }
}
