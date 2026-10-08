//! The stress loads N1-N4 (DC7): the offsets, sizes and read/write mix come from the
//! phase's [`DiskJob`]. Writes carry the signed block of generation 1 at their own index;
//! every block read is verified against the same (DC9), so what an earlier pass or the fill
//! wrote must read back as it was.

use oma_core::disk_block::{check_block, write_block, BLOCK_BYTES};

use super::engine::{Fault, LoadCtx, WorkerLoad};
use super::offsets::{IoPicker, IoReq};

pub struct Stress {
    picker: IoPicker,
    session: u64,
    compressible: bool,
}

impl Stress {
    pub fn new(ctx: &LoadCtx<'_>, t: u16) -> Stress {
        Stress {
            picker: IoPicker::new(ctx.job, ctx.file_bytes, t, ctx.seed),
            session: ctx.session,
            compressible: ctx.compressible,
        }
    }
}

impl WorkerLoad for Stress {
    fn next(&mut self) -> Option<IoReq> {
        Some(self.picker.next())
    }

    fn prepare(&mut self, req: &IoReq, buf: &mut [u8]) {
        let first = req.offset / BLOCK_BYTES as u64;
        for (i, block) in buf.chunks_exact_mut(BLOCK_BYTES).enumerate() {
            write_block(block, self.session, first + i as u64, 1, self.compressible);
        }
    }

    fn check_read(&mut self, req: &IoReq, buf: &[u8], faults: &mut Vec<Fault>) -> u64 {
        let first = req.offset / BLOCK_BYTES as u64;
        let mut checked = 0;
        for (i, block) in buf.chunks_exact(BLOCK_BYTES).enumerate() {
            let index = first + i as u64;
            if let Err(fault) = check_block(block, self.session, index, 1) {
                faults.push((index, fault));
            }
            checked += 1;
        }
        checked
    }
}
