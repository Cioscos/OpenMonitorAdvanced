//! The verification loads V1-V4 (DC9): each writes known blocks, flushes, and reads them back
//! in a different order (last to first), so a stale, misplaced, zeroed or flipped block
//! shows. A phase is a sequence of passes ([`PassSource`]); the engine runs each pass with
//! its own workers and queues. The reread of a bad block and the error reports are the
//! engine's (`Worker::verify`); the loads only say which blocks to write and what to expect.
//!
//! - V1: per cycle `c`, the whole file with generation `c`, flush, a backward read pass.
//! - V2: a table of generations (starts at 1, V2 follows a fill); rate-limited random 4 KiB
//!   writes with generation + 1 in windows of at most 300 s, each followed by a flush and a
//!   backward read pass against the table.
//! - V3: files `-<n>` of 1 GiB while they fit, flushed one by one, then verified last to first.
//!   The block index is the global one: `n * 262 144 + block`.
//! - V4: sequential write-through up to the write cap on `-sync`, flush, backward read pass.

use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use oma_core::disk_block::{check_block, write_block, BlockFault, BLOCK_BYTES};
use oma_ipc::load::{DiskJob, DiskTarget, KernelId};

use super::engine::{DataFileApi, DiskHooks, Fatal, Fault, WorkerLoad};
use super::file::{fits, DiskError};
use super::offsets::IoReq;
use crate::rng::Xoshiro256ss;

/// The read chunk of every verify pass.
const CHUNK: u64 = 1 << 20;
/// The size of a V3 file, and the blocks the global index skips per file.
const PART_BYTES: u64 = 1 << 30;
const PART_BLOCKS: u64 = PART_BYTES / BLOCK_BYTES as u64;
/// V2 flushes and verifies at least this often.
const V2_WINDOW: Duration = Duration::from_secs(300);

/// The armed fault injection of a phase: the first block checked has a bit flipped (after
/// the read, before the check), once.
pub type Armed = Option<Arc<AtomicBool>>;

/// `check_block`, with the injected fault applied to the first block if armed.
pub fn check_armed(
    block: &[u8],
    armed: &Armed,
    session: u64,
    index: u64,
    generation: u32,
) -> Result<(), BlockFault> {
    if armed
        .as_ref()
        .is_some_and(|a| a.swap(false, Ordering::Relaxed))
    {
        let mut copy = [0u8; BLOCK_BYTES];
        copy.copy_from_slice(block);
        copy[100] ^= 1;
        return check_block(&copy, session, index, generation);
    }
    check_block(block, session, index, generation)
}

/// One run of workers over one file.
pub struct Pass {
    pub file: Rc<dyn DataFileApi>,
    /// The shape the queues are sized by.
    pub job: DiskJob,
    pub loads: Vec<Box<dyn WorkerLoad>>,
    /// The pass halts after this long (a V2 write window); `None` runs it to its end.
    pub limit: Option<Duration>,
}

/// The passes of a phase, one at a time; `elapsed` is the phase time so far. `None` ends it.
pub trait PassSource {
    fn next(&mut self, elapsed: Duration) -> Result<Option<Pass>, Fatal>;
}

/// What the sources need from the run.
pub struct VerifyEnv<'h> {
    pub hooks: &'h DiskHooks<'h>,
    pub target: &'h DiskTarget,
    pub main: Rc<dyn DataFileApi>,
    pub session: u64,
    pub compressible: bool,
    pub seed: u64,
    pub armed: Armed,
    /// The phase duration (the time cap of V1 and V3, the length of V2).
    pub duration: Duration,
}

/// The source of a V kernel's passes, `None` for any other kernel.
pub fn source<'h>(
    kernel: KernelId,
    job: &DiskJob,
    env: VerifyEnv<'h>,
) -> Option<Box<dyn PassSource + 'h>> {
    let job = job.clone();
    Some(match kernel {
        KernelId::V1 => Box::new(V1 {
            env,
            job,
            cycle: 0,
            verify: false,
        }),
        KernelId::V2 => {
            let blocks = env.main.bytes() / BLOCK_BYTES as u64;
            let table = (0..blocks).map(|_| AtomicU16::new(1)).collect();
            Box::new(V2 {
                env,
                job,
                table,
                windows: 0,
                verify: false,
            })
        }
        KernelId::V3 => Box::new(V3 {
            env,
            job,
            parts: Vec::new(),
            state: V3State::Open,
        }),
        KernelId::V4 => Box::new(V4 {
            env,
            job,
            file: None,
            stage: 0,
        }),
        _ => return None,
    })
}

fn flush(file: &dyn DataFileApi) -> Result<(), Fatal> {
    file.flush().map_err(Fatal::from_open)
}

/// A sequential job of one block size, reading or writing, for the queue buffers.
fn seq_job(job: &DiskJob, block: u32, read: bool) -> DiskJob {
    DiskJob {
        block_bytes: block,
        seq_block_bytes: block,
        random_percent: 0,
        read_percent: if read { 100 } else { 0 },
        write_cap_bytes: None,
        cycles: None,
        rate_limit_bps: None,
        ..job.clone()
    }
}

/// Which generation a block must have.
#[derive(Clone)]
enum Gens {
    Fixed(u32),
    /// V2: indexed by the block in the file.
    Table(Arc<[AtomicU16]>),
}

#[derive(Clone)]
enum Mode {
    Write(u32),
    Check(Gens),
}

/// A worker's stripe of a file, walked in `chunk` steps, forward or backward.
struct Sweep {
    lo: u64,
    hi: u64,
    cursor: u64,
    chunk: u64,
    forward: bool,
    /// The global index of the file's first 4 KiB block.
    base: u64,
    mode: Mode,
    session: u64,
    compressible: bool,
    armed: Armed,
}

impl WorkerLoad for Sweep {
    fn next(&mut self) -> Option<IoReq> {
        let write = matches!(self.mode, Mode::Write(_));
        let (offset, len) = if self.forward {
            if self.cursor >= self.hi {
                return None;
            }
            let len = self.chunk.min(self.hi - self.cursor);
            let at = self.cursor;
            self.cursor += len;
            (at, len)
        } else {
            if self.cursor <= self.lo {
                return None;
            }
            let start = ((self.cursor - 1) / self.chunk * self.chunk).max(self.lo);
            let len = self.cursor - start;
            self.cursor = start;
            (start, len)
        };
        Some(IoReq {
            offset,
            len: len as u32,
            write,
        })
    }

    fn prepare(&mut self, req: &IoReq, buf: &mut [u8]) {
        let Mode::Write(generation) = self.mode else {
            return;
        };
        let first = self.base + req.offset / BLOCK_BYTES as u64;
        for (i, block) in buf.chunks_exact_mut(BLOCK_BYTES).enumerate() {
            write_block(
                block,
                self.session,
                first + i as u64,
                generation,
                self.compressible,
            );
        }
    }

    fn check_read(&mut self, req: &IoReq, buf: &[u8], faults: &mut Vec<Fault>) -> u64 {
        let Mode::Check(gens) = &self.mode else {
            return 0;
        };
        let local = req.offset / BLOCK_BYTES as u64;
        let mut checked = 0;
        for (i, block) in buf.chunks_exact(BLOCK_BYTES).enumerate() {
            let local = local + i as u64;
            let generation = match gens {
                Gens::Fixed(g) => *g,
                Gens::Table(t) => u32::from(t[local as usize].load(Ordering::Relaxed)),
            };
            let index = self.base + local;
            if let Err(fault) = check_armed(block, &self.armed, self.session, index, generation) {
                faults.push((index, fault));
            }
            checked += 1;
        }
        checked
    }
}

/// One `Sweep` per thread, over `len` bytes of a file, each on its own stripe.
#[allow(clippy::too_many_arguments)]
fn sweeps(
    env: &VerifyEnv<'_>,
    threads: u16,
    len: u64,
    chunk: u64,
    forward: bool,
    base: u64,
    mode: Mode,
) -> Vec<Box<dyn WorkerLoad>> {
    let threads = u128::from(threads.max(1));
    let stripe = |t: u128| (u128::from(len) * t / threads) as u64 / chunk * chunk;
    (0..threads)
        .map(|t| {
            let lo = stripe(t);
            let hi = if t + 1 == threads { len } else { stripe(t + 1) };
            Box::new(Sweep {
                lo,
                hi,
                cursor: if forward { lo } else { hi },
                chunk,
                forward,
                base,
                mode: mode.clone(),
                session: env.session,
                compressible: env.compressible,
                armed: env.armed.clone(),
            }) as Box<dyn WorkerLoad>
        })
        .collect()
}

/// A forward write pass of a whole file (`len` bytes in blocks of `block`).
fn write_pass(
    env: &VerifyEnv<'_>,
    job: &DiskJob,
    file: &Rc<dyn DataFileApi>,
    len: u64,
    block: u32,
    base: u64,
    generation: u32,
) -> Pass {
    Pass {
        file: Rc::clone(file),
        job: seq_job(job, block, false),
        loads: sweeps(
            env,
            job.threads,
            len,
            u64::from(block),
            true,
            base,
            Mode::Write(generation),
        ),
        limit: None,
    }
}

/// A backward read pass of a whole file in 1 MiB chunks.
fn verify_pass(
    env: &VerifyEnv<'_>,
    job: &DiskJob,
    file: &Rc<dyn DataFileApi>,
    len: u64,
    base: u64,
    gens: Gens,
) -> Pass {
    let chunk = CHUNK.min(len);
    Pass {
        file: Rc::clone(file),
        job: seq_job(job, chunk as u32, true),
        loads: sweeps(env, job.threads, len, chunk, false, base, Mode::Check(gens)),
        limit: None,
    }
}

struct V1<'h> {
    env: VerifyEnv<'h>,
    job: DiskJob,
    cycle: u32,
    verify: bool,
}

impl PassSource for V1<'_> {
    fn next(&mut self, elapsed: Duration) -> Result<Option<Pass>, Fatal> {
        let (env, job) = (&self.env, &self.job);
        let len = env.main.bytes();
        if self.verify {
            self.verify = false;
            flush(&*env.main)?;
            let gens = Gens::Fixed(self.cycle);
            return Ok(Some(verify_pass(env, job, &env.main, len, 0, gens)));
        }
        // The time cap is checked between cycles: a cut pass would leave old blocks behind.
        let cycles = job.cycles.unwrap_or(1);
        if self.cycle >= cycles || (self.cycle > 0 && elapsed >= env.duration) {
            return Ok(None);
        }
        self.cycle += 1;
        self.verify = true;
        let block = job.seq_block_bytes;
        Ok(Some(write_pass(
            env, job, &env.main, len, block, 0, self.cycle,
        )))
    }
}

/// The length of the next V2 write window at `elapsed` of `duration`; `None` when the
/// phase time is spent.
pub fn v2_window(elapsed: Duration, duration: Duration) -> Option<Duration> {
    let left = duration.checked_sub(elapsed).filter(|d| !d.is_zero())?;
    Some(left.min(V2_WINDOW))
}

struct V2<'h> {
    env: VerifyEnv<'h>,
    job: DiskJob,
    table: Arc<[AtomicU16]>,
    windows: u32,
    verify: bool,
}

impl PassSource for V2<'_> {
    fn next(&mut self, elapsed: Duration) -> Result<Option<Pass>, Fatal> {
        let (env, job) = (&self.env, &self.job);
        let len = env.main.bytes();
        if self.verify {
            self.verify = false;
            flush(&*env.main)?;
            let gens = Gens::Table(Arc::clone(&self.table));
            return Ok(Some(verify_pass(env, job, &env.main, len, 0, gens)));
        }
        // The first window always runs, so even a zero duration verifies something.
        let Some(window) = v2_window(elapsed, env.duration)
            .or_else(|| (self.windows == 0).then_some(Duration::from_secs(1)))
        else {
            return Ok(None);
        };
        self.windows += 1;
        self.verify = true;
        let threads = u128::from(job.threads.max(1));
        let blocks = len / BLOCK_BYTES as u64;
        let stripe = |t: u128| (u128::from(blocks) * t / threads) as u64;
        let rate = job.rate_limit_bps.unwrap_or(u64::MAX) / threads as u64;
        let loads = (0..threads)
            .map(|t| {
                let (lo, hi) = (stripe(t), stripe(t + 1));
                Box::new(RandomWrites {
                    lo,
                    blocks: hi - lo,
                    rng: Xoshiro256ss::new(env.seed ^ (u64::from(self.windows) << 32) ^ t as u64),
                    table: Arc::clone(&self.table),
                    recent: VecDeque::new(),
                    depth: usize::from(job.queue),
                    rate: rate.max(1),
                    sent: 0,
                    started: None,
                    session: env.session,
                    compressible: env.compressible,
                }) as Box<dyn WorkerLoad>
            })
            .collect();
        Ok(Some(Pass {
            file: Rc::clone(&env.main),
            job: DiskJob {
                block_bytes: BLOCK_BYTES as u32,
                random_percent: 100,
                read_percent: 0,
                write_cap_bytes: None,
                cycles: None,
                ..job.clone()
            },
            loads,
            limit: Some(window),
        }))
    }
}

/// V2's writer: random 4 KiB blocks of its stripe, each at its table generation + 1, at
/// most `rate` bytes a second.
struct RandomWrites {
    lo: u64,
    blocks: u64,
    rng: Xoshiro256ss,
    table: Arc<[AtomicU16]>,
    /// The last `depth` blocks handed out: two writes to one block must never be in flight
    /// together, or the one that lands last is anyone's guess.
    recent: VecDeque<u64>,
    depth: usize,
    rate: u64,
    sent: u64,
    started: Option<Instant>,
    session: u64,
    compressible: bool,
}

impl WorkerLoad for RandomWrites {
    fn throttle(&mut self, now: Instant) -> Option<Duration> {
        let started = *self.started.get_or_insert(now);
        let block = BLOCK_BYTES as u64;
        // One block of burst.
        let allowed = (self.rate as f64 * now.saturating_duration_since(started).as_secs_f64())
            as u64
            + block;
        if self.sent + block <= allowed {
            self.sent += block;
            return None;
        }
        Some(Duration::from_secs_f64(
            (self.sent + block - allowed) as f64 / self.rate as f64,
        ))
    }

    fn next(&mut self) -> Option<IoReq> {
        if self.blocks == 0 {
            return None;
        }
        let mut block = self.lo + self.rng.next_u64() % self.blocks;
        if self.blocks > self.depth as u64 {
            while self.recent.contains(&block) {
                block = self.lo + self.rng.next_u64() % self.blocks;
            }
        }
        self.recent.push_back(block);
        if self.recent.len() > self.depth {
            self.recent.pop_front();
        }
        Some(IoReq {
            offset: block * BLOCK_BYTES as u64,
            len: BLOCK_BYTES as u32,
            write: true,
        })
    }

    fn prepare(&mut self, req: &IoReq, buf: &mut [u8]) {
        let index = req.offset / BLOCK_BYTES as u64;
        let generation = self.table[index as usize].fetch_add(1, Ordering::Relaxed) + 1;
        write_block(
            buf,
            self.session,
            index,
            u32::from(generation),
            self.compressible,
        );
    }
}

enum V3State {
    /// Create the next part.
    Open,
    /// Flush the part just written.
    Flush,
    /// Verify the parts from this count down to the first.
    Verify(usize),
}

struct V3<'h> {
    env: VerifyEnv<'h>,
    job: DiskJob,
    parts: Vec<Rc<dyn DataFileApi>>,
    state: V3State,
}

impl V3<'_> {
    /// Whether another part may be created: the time and the write cap allow it and the
    /// volume has room for it above the reserve.
    fn room(&self, elapsed: Duration) -> bool {
        let (env, n) = (&self.env, self.parts.len() as u64);
        if n > 0 && elapsed >= env.duration {
            return false;
        }
        if self
            .job
            .write_cap_bytes
            .is_some_and(|cap| (n + 1) * PART_BYTES > cap)
        {
            return false;
        }
        (env.hooks.free_bytes)().is_none_or(|free| fits(free, PART_BYTES, env.target.reserve_bytes))
    }
}

impl PassSource for V3<'_> {
    fn next(&mut self, elapsed: Duration) -> Result<Option<Pass>, Fatal> {
        loop {
            match self.state {
                V3State::Open => {
                    let n = self.parts.len();
                    let suffix = format!("-{n}");
                    let opened = if self.room(elapsed) {
                        let open = self.env.hooks.open;
                        match open(self.env.target, Some(&suffix), PART_BYTES, false) {
                            Ok(file) => Some(Rc::<dyn DataFileApi>::from(file)),
                            // The volume is full: that is how the parts end.
                            Err(DiskError::Full) if n > 0 => None,
                            Err(e) => return Err(Fatal::from_open(e)),
                        }
                    } else {
                        None
                    };
                    let Some(file) = opened else {
                        if n == 0 {
                            return Err(Fatal::Full);
                        }
                        self.state = V3State::Verify(n);
                        continue;
                    };
                    self.parts.push(Rc::clone(&file));
                    self.state = V3State::Flush;
                    let base = n as u64 * PART_BLOCKS;
                    let block = self.job.seq_block_bytes;
                    let len = file.bytes();
                    return Ok(Some(write_pass(
                        &self.env, &self.job, &file, len, block, base, 1,
                    )));
                }
                V3State::Flush => {
                    flush(&**self.parts.last().expect("a part was just written"))?;
                    self.state = V3State::Open;
                }
                V3State::Verify(0) => return Ok(None),
                V3State::Verify(k) => {
                    self.state = V3State::Verify(k - 1);
                    let file = &self.parts[k - 1];
                    let base = (k as u64 - 1) * PART_BLOCKS;
                    return Ok(Some(verify_pass(
                        &self.env,
                        &self.job,
                        file,
                        file.bytes(),
                        base,
                        Gens::Fixed(1),
                    )));
                }
            }
        }
    }
}

struct V4<'h> {
    env: VerifyEnv<'h>,
    job: DiskJob,
    file: Option<(Rc<dyn DataFileApi>, u64)>,
    stage: u8,
}

impl PassSource for V4<'_> {
    fn next(&mut self, _elapsed: Duration) -> Result<Option<Pass>, Fatal> {
        let (env, job) = (&self.env, &self.job);
        self.stage += 1;
        match self.stage {
            1 => {
                let block = u64::from(job.seq_block_bytes);
                let cap = job.write_cap_bytes.unwrap_or(env.main.bytes());
                let region = cap / block * block;
                if region == 0 {
                    return Ok(None);
                }
                let room = (env.hooks.free_bytes)()
                    .is_none_or(|free| fits(free, region, env.target.reserve_bytes));
                if !room {
                    return Err(Fatal::Full);
                }
                let file = (env.hooks.open)(env.target, Some("-sync"), region, true)
                    .map_err(Fatal::from_open)?;
                let file = Rc::<dyn DataFileApi>::from(file);
                let pass = write_pass(env, job, &file, region, job.seq_block_bytes, 0, 1);
                self.file = Some((file, region));
                Ok(Some(pass))
            }
            2 => {
                let (file, region) = self.file.as_ref().expect("written in stage 1");
                flush(&**file)?;
                Ok(Some(verify_pass(
                    env,
                    job,
                    file,
                    *region,
                    0,
                    Gens::Fixed(1),
                )))
            }
            _ => Ok(None),
        }
    }
}
