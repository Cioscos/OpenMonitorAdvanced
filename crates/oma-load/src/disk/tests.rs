//! The disk engine with a fake I/O queue and an accelerated clock (at most 3 s each), and
//! one short run on a real 64 MiB file in the temporary folder.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use oma_core::disk_block::{check_block, write_block, BLOCK_BYTES};
use oma_ipc::load::{
    ComputeError, DataSize, DiskJob, DiskTarget, ErrorKind, FinishReason, Isa, KernelId,
    LoadMessage, LoadMode, Notice, Phase, PhaseDone, Placement, Plan, Progress,
};

use super::engine::{
    run_disk_with, used_blocks, DataFileApi, DiskHooks, DiskRunEnd, IoDone, IoQueue,
};
use super::file::DiskError;
use super::offsets::IoReq;
use crate::link::{EXIT_IO, EXIT_OK};

pub(super) const MIB: u64 = 1 << 20;
pub(super) const SEED: u64 = 0xD15C;

#[derive(Clone, Copy)]
pub(super) struct Clock {
    base: Instant,
    speed: u32,
}

impl Clock {
    pub(super) fn new(speed: u32) -> Self {
        Self {
            base: Instant::now(),
            speed,
        }
    }

    pub(super) fn now(&self) -> Instant {
        self.base + (Instant::now() - self.base) * self.speed
    }
}

/// What the fake disk does.
#[derive(Default, Clone)]
struct Script {
    /// The I/O with this index (from 0, over the whole run) fails once with the code.
    fail_once: Option<(u64, u32)>,
    /// Every I/O from this index on fails with the code.
    fail_from: Option<(u64, u32)>,
    /// Writes fail with ERROR_DISK_FULL once this many bytes were written.
    full_after: Option<u64>,
    /// Complete only the oldest I/O at each `wait`, so many stay in flight.
    one_per_wait: bool,
    /// Check every written 4 KiB block with `check_block`, generation 1.
    check_writes: bool,
    /// Reads return the valid generation-1 blocks of their offset.
    serve_reads: bool,
    /// Reads at this offset return a block of another index, this many times.
    bad_offset: Option<u64>,
    bad_reads: u32,
    /// After this many completed I/Os a `wait` takes four times longer (800 us, not 200), so the speed of a sequential writer drops to a quarter.
    slow_after_ios: Option<u64>,
}

#[derive(Default)]
struct Disk {
    script: Script,
    submitted: AtomicU64,
    completed: AtomicU64,
    write_bytes: AtomicU64,
    /// Block index -> times written (only with `check_writes`).
    blocks: Mutex<HashMap<u64, u32>>,
    bad_blocks: AtomicU64,
    bad_left: AtomicU32,
    /// The (write, length) of every I/O.
    shapes: Mutex<HashSet<(bool, u32)>>,
}

struct FakeQueue {
    disk: Arc<Disk>,
    bufs: Vec<Vec<u8>>,
    pending: Vec<(usize, Result<u32, u32>)>,
}

impl IoQueue for FakeQueue {
    fn submit(&mut self, req: IoReq, slot: usize) -> Result<(), DiskError> {
        let n = self.disk.submitted.fetch_add(1, Ordering::Relaxed);
        let s = &self.disk.script;
        let mut result = Ok(req.len);
        if s.fail_from.is_some_and(|(k, _)| n >= k) {
            result = Err(s.fail_from.unwrap().1);
        } else if s.fail_once.is_some_and(|(k, _)| n == k) {
            result = Err(s.fail_once.unwrap().1);
        } else if req.write {
            let before = self
                .disk
                .write_bytes
                .fetch_add(u64::from(req.len), Ordering::Relaxed);
            if s.full_after.is_some_and(|limit| before >= limit) {
                result = Err(112);
            }
        }
        if req.write && s.check_writes && result.is_ok() {
            let buf = &self.bufs[slot][..req.len as usize];
            let mut blocks = self.disk.blocks.lock().unwrap();
            for (i, chunk) in buf.chunks_exact(BLOCK_BYTES).enumerate() {
                let index = req.offset / BLOCK_BYTES as u64 + i as u64;
                if check_block(chunk, SEED, index, 1).is_err() {
                    self.disk.bad_blocks.fetch_add(1, Ordering::Relaxed);
                }
                *blocks.entry(index).or_default() += 1;
            }
        }
        self.disk
            .shapes
            .lock()
            .unwrap()
            .insert((req.write, req.len));
        if !req.write && s.serve_reads && result.is_ok() {
            let first = req.offset / BLOCK_BYTES as u64;
            let wrong = s.bad_offset == Some(req.offset)
                && self
                    .disk
                    .bad_left
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                    .is_ok();
            let buf = &mut self.bufs[slot][..req.len as usize];
            for (i, chunk) in buf.chunks_exact_mut(BLOCK_BYTES).enumerate() {
                let shift = if wrong && i == 0 { 1000 } else { 0 };
                write_block(chunk, SEED, first + i as u64 + shift, 1, false);
            }
        }
        self.pending.push((slot, result));
        Ok(())
    }

    fn wait(&mut self, _timeout_ms: u32, out: &mut Vec<IoDone>) -> Result<(), DiskError> {
        match self.disk.script.slow_after_ios {
            Some(n) => {
                // Spins: a sleep this short rounds up to the timer tick (~15 ms here).
                let slow = self.disk.completed.load(Ordering::Relaxed) >= n;
                let until = Instant::now() + Duration::from_micros(if slow { 800 } else { 200 });
                while Instant::now() < until {
                    std::hint::spin_loop();
                }
            }
            None => thread::sleep(Duration::from_micros(50)),
        }
        let take = if self.disk.script.one_per_wait {
            self.pending.len().min(1)
        } else {
            self.pending.len()
        };
        for (slot, result) in self.pending.drain(..take) {
            self.disk.completed.fetch_add(1, Ordering::Relaxed);
            out.push(IoDone {
                slot,
                result,
                latency_us: 250,
            });
        }
        Ok(())
    }

    fn slots(&self) -> usize {
        self.bufs.len()
    }

    fn buffer(&mut self, slot: usize) -> &mut [u8] {
        &mut self.bufs[slot]
    }
}

struct FakeFile {
    bytes: u64,
    sector: u32,
}

impl DataFileApi for FakeFile {
    fn bytes(&self) -> u64 {
        self.bytes
    }
    fn sector(&self) -> u32 {
        self.sector
    }
    fn flush(&self) -> Result<(), DiskError> {
        Ok(())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct Setup {
    script: Script,
    sector: u32,
    open_error: Option<DiskError>,
    /// Real milliseconds after which the stop flag is raised.
    stop_after_ms: Option<u64>,
    speed: u32,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            script: Script::default(),
            sector: 4096,
            open_error: None,
            stop_after_ms: None,
            speed: 50,
        }
    }
}

struct Ran {
    end: DiskRunEnd,
    msgs: Vec<LoadMessage>,
    disk: Arc<Disk>,
    real: Duration,
}

impl Ran {
    fn done(&self) -> Vec<&PhaseDone> {
        self.msgs
            .iter()
            .filter_map(|m| match m {
                LoadMessage::PhaseDone(d) => Some(d),
                _ => None,
            })
            .collect()
    }

    fn errors(&self) -> Vec<&ComputeError> {
        self.msgs
            .iter()
            .filter_map(|m| match m {
                LoadMessage::Error(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    fn notices(&self) -> Vec<&Notice> {
        self.msgs
            .iter()
            .filter_map(|m| match m {
                LoadMessage::Notice(n) => Some(n),
                _ => None,
            })
            .collect()
    }

    fn progress(&self, phase: u32) -> Vec<&Progress> {
        self.msgs
            .iter()
            .filter_map(|m| match m {
                LoadMessage::Progress(p) if p.phase == phase && p.disk.is_some() => Some(p),
                _ => None,
            })
            .collect()
    }
}

pub(super) fn job(block: u32, seq: u32, random: u8, read: u8, queue: u16, threads: u16) -> DiskJob {
    DiskJob {
        block_bytes: block,
        seq_block_bytes: seq,
        random_percent: random,
        read_percent: read,
        queue,
        threads,
        write_cap_bytes: None,
        cycles: None,
        rate_limit_bps: None,
    }
}

pub(super) fn phase(kernel: KernelId, duration_s: u32, job: DiskJob) -> Phase {
    Phase {
        kernel,
        alt_kernel: None,
        isa: Isa::Sse2,
        size: DataSize::Auto,
        mode: LoadMode::Steady,
        placement: Placement::AllLogical,
        duration_s,
        per_core_s: None,
        both_smt: false,
        cores: None,
        patterns: vec![],
        stop_on_error: false,
        iterations: None,
        pause_before_ms: 0,
        windows: None,
        disk: Some(job),
    }
}

pub(super) fn plan(file_bytes: u64, phases: Vec<Phase>) -> Plan {
    Plan {
        seed: SEED,
        ram_bytes: 0,
        phases,
        gpu: None,
        disk: Some(DiskTarget {
            dir: r"C:\fake".into(),
            file_bytes,
            compressible: false,
            reserve_bytes: 1 << 30,
        }),
    }
}

fn run(plan: &Plan, setup: Setup) -> Ran {
    let disk = Arc::new(Disk {
        script: setup.script.clone(),
        bad_left: AtomicU32::new(setup.script.bad_reads),
        ..Disk::default()
    });
    let msgs = Mutex::new(Vec::new());
    let out = |m: LoadMessage| msgs.lock().unwrap().push(m);
    let stop = AtomicBool::new(false);
    let clock = Clock::new(setup.speed);
    let now = move || clock.now();
    let sector = setup.sector;
    let open_error = setup.open_error;
    let open = move |_: &DiskTarget, _: Option<&str>, bytes: u64, _: bool| match open_error {
        Some(e) => Err(e),
        None => Ok(Box::new(FakeFile { bytes, sector }) as Box<dyn DataFileApi>),
    };
    let qdisk = Arc::clone(&disk);
    let queue = move |_: &dyn DataFileApi, j: &DiskJob| {
        // Sized like the IOCP queue: the largest block the job uses.
        let size = used_blocks(j).1 as usize;
        Ok(Box::new(FakeQueue {
            disk: Arc::clone(&qdisk),
            bufs: vec![vec![0u8; size]; usize::from(j.queue)],
            pending: Vec::new(),
        }) as Box<dyn IoQueue>)
    };
    let free = || Some(u64::MAX / 4);
    let hooks = DiskHooks {
        open: &open,
        queue: &queue,
        free_bytes: &free,
        clock: &now,
    };
    let t0 = Instant::now();
    let end = thread::scope(|s| {
        if let Some(ms) = setup.stop_after_ms {
            let stop = &stop;
            s.spawn(move || {
                thread::sleep(Duration::from_millis(ms));
                stop.store(true, Ordering::Relaxed);
            });
        }
        run_disk_with(plan, &out, &stop, None, &hooks)
    });
    let real = t0.elapsed();
    assert!(real < Duration::from_secs(3), "the test took {real:?}");
    Ran {
        end,
        msgs: msgs.into_inner().unwrap(),
        disk,
        real,
    }
}

#[test]
fn bench_phase_reports_bytes_ios_and_latencies() {
    let p = plan(
        64 * MIB,
        vec![phase(
            KernelId::DiskBench,
            2,
            job(4096, 1 << 20, 100, 100, 4, 1),
        )],
    );
    let r = run(&p, Setup::default());
    assert_eq!(r.end.exit_code, EXIT_OK);
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
    let done = r.done();
    assert_eq!(done.len(), 1);
    let stats = done[0].disk.as_ref().expect("disk stats");
    let ios = r.disk.completed.load(Ordering::Relaxed);
    assert!(ios > 0);
    assert_eq!(stats.read.ios, ios);
    assert_eq!(stats.read.bytes, ios * 4096);
    assert_eq!(stats.read.mean_lat_us, 250.0);
    assert!((stats.read.p99_lat_us - 250.0).abs() <= 250.0 * 0.125);
    assert!(stats.read.elapsed_us > 0);
    assert_eq!(stats.write.ios, 0);
    assert_eq!(stats.write.bytes, 0);
}

#[test]
fn write_cap_stops_the_phase_before_the_duration() {
    let mut j = job(4096, 1 << 20, 0, 0, 4, 1);
    j.write_cap_bytes = Some(64 * MIB);
    let p = plan(64 * MIB, vec![phase(KernelId::DiskBench, 60, j)]);
    let r = run(&p, Setup::default());
    let done = r.done();
    assert!(done[0].duration_ms < 60_000, "{}", done[0].duration_ms);
    assert_eq!(done[0].disk.as_ref().unwrap().write.bytes, 64 * MIB);
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
}

#[test]
fn no_write_starts_past_the_cap() {
    let mut j = job(4096, 1 << 20, 0, 0, 8, 2);
    j.write_cap_bytes = Some(10 * MIB + 4096);
    let p = plan(64 * MIB, vec![phase(KernelId::DiskBench, 60, j)]);
    let r = run(&p, Setup::default());
    assert_eq!(r.disk.write_bytes.load(Ordering::Relaxed), 10 * MIB);
    assert_eq!(r.done()[0].disk.as_ref().unwrap().write.bytes, 10 * MIB);
}

#[test]
fn phase_ends_at_its_duration_and_drains_in_flight() {
    let setup = Setup {
        script: Script {
            one_per_wait: true,
            ..Script::default()
        },
        // Slower, so the drain of 64 I/Os in flight stays well under a test second.
        speed: 10,
        ..Setup::default()
    };
    let p = plan(
        64 * MIB,
        vec![phase(
            KernelId::DiskBench,
            2,
            job(4096, 1 << 20, 100, 100, 32, 2),
        )],
    );
    let r = run(&p, setup);
    let submitted = r.disk.submitted.load(Ordering::Relaxed);
    assert_eq!(r.disk.completed.load(Ordering::Relaxed), submitted);
    let done = r.done();
    assert_eq!(done[0].disk.as_ref().unwrap().read.ios, submitted);
    assert!(
        (2000..3000).contains(&done[0].duration_ms),
        "{}",
        done[0].duration_ms
    );
}

#[test]
fn progress_every_second_carries_disk_rates() {
    let p = plan(
        64 * MIB,
        vec![phase(
            KernelId::DiskBench,
            5,
            job(4096, 1 << 20, 100, 50, 4, 1),
        )],
    );
    let r = run(&p, Setup::default());
    let prog = r.progress(0);
    assert!((4..=6).contains(&prog.len()), "{} progress", prog.len());
    for p in prog {
        let d = p.disk.as_ref().unwrap();
        assert!(d.read_bps.is_finite() && d.read_bps > 0.0);
        assert!(d.write_bps.is_finite() && d.write_bps > 0.0);
        assert!(d.iops > 0.0);
        assert_eq!(p.rate, Some(d.read_bps + d.write_bps));
    }
    let last = r.progress(0).last().unwrap().disk.clone().unwrap();
    assert!(last.read_bytes > 0 && last.written_bytes > 0);
}

#[test]
fn fill_writes_every_block_once_with_generation_1() {
    let setup = Setup {
        script: Script {
            check_writes: true,
            ..Script::default()
        },
        ..Setup::default()
    };
    let p = plan(
        64 * MIB,
        vec![phase(
            KernelId::DiskFill,
            900,
            job(1 << 20, 1 << 20, 0, 0, 4, 1),
        )],
    );
    let r = run(&p, setup);
    assert_eq!(r.disk.bad_blocks.load(Ordering::Relaxed), 0);
    let blocks = r.disk.blocks.lock().unwrap();
    assert_eq!(blocks.len(), (64 * MIB / 4096) as usize);
    assert!(blocks.values().all(|&n| n == 1));
    let done = r.done();
    assert_eq!(done[0].disk.as_ref().unwrap().write.bytes, 64 * MIB);
    assert!(done[0].duration_ms < 900_000);
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
}

fn assert_every_block_once(r: &Ran) {
    assert_eq!(r.disk.bad_blocks.load(Ordering::Relaxed), 0);
    let blocks = r.disk.blocks.lock().unwrap();
    assert_eq!(blocks.len(), (64 * MIB / 4096) as usize);
    assert!(blocks.values().all(|&n| n == 1));
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
}

#[test]
fn fill_with_a_random_job_shape_uses_the_queue_buffers() {
    // random_percent 100 and a block smaller than the sequential one: the buffers are
    // sized from `used_blocks`, and the fill must write in blocks that fit them.
    let setup = Setup {
        script: Script {
            check_writes: true,
            ..Script::default()
        },
        ..Setup::default()
    };
    let p = plan(
        64 * MIB,
        vec![phase(
            KernelId::DiskFill,
            900,
            job(64 << 10, 1 << 20, 100, 0, 8, 1),
        )],
    );
    let r = run(&p, setup);
    assert_every_block_once(&r);
}

#[test]
fn fill_is_not_cut_by_its_duration() {
    let setup = Setup {
        script: Script {
            check_writes: true,
            one_per_wait: true,
            ..Script::default()
        },
        ..Setup::default()
    };
    // 64 writes, one per wait: far more than 1 s on the test clock.
    let p = plan(
        64 * MIB,
        vec![phase(
            KernelId::DiskFill,
            1,
            job(1 << 20, 1 << 20, 0, 0, 4, 1),
        )],
    );
    let r = run(&p, setup);
    assert_every_block_once(&r);
    assert!(
        r.done()[0].duration_ms > 1000,
        "{}",
        r.done()[0].duration_ms
    );
}

#[test]
fn pause_sends_progress_at_least_every_900_ms() {
    let mut ph = phase(KernelId::DiskBench, 1, job(4096, 1 << 20, 100, 100, 4, 1));
    ph.pause_before_ms = 5000;
    let r = run(&plan(64 * MIB, vec![ph]), Setup::default());
    let beats: Vec<u64> = r
        .msgs
        .iter()
        .filter_map(|m| match m {
            LoadMessage::Progress(p) if p.disk.is_none() => Some(p.elapsed_ms),
            _ => None,
        })
        .collect();
    assert!(beats.len() >= 5, "{beats:?}");
    // `elapsed_ms` counts from the run's start, file setup included: only the gaps between
    // heartbeats are the pause's.
    assert!(beats.windows(2).all(|w| w[1] - w[0] <= 1000), "{beats:?}");
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
}

#[test]
fn stop_during_the_pause_finishes_stopped() {
    let mut ph = phase(KernelId::DiskBench, 5, job(4096, 1 << 20, 100, 100, 4, 1));
    ph.pause_before_ms = 10_000;
    let setup = Setup {
        stop_after_ms: Some(50),
        speed: 10,
        ..Setup::default()
    };
    let r = run(&plan(64 * MIB, vec![ph]), setup);
    assert_eq!(r.end.finished.reason, FinishReason::Stopped);
    assert_eq!(r.disk.submitted.load(Ordering::Relaxed), 0);
    assert!(r.real < Duration::from_millis(900), "{:?}", r.real);
}

#[test]
fn disk_full_finishes_failed_with_the_notice() {
    let setup = Setup {
        script: Script {
            full_after: Some(8 * MIB),
            ..Script::default()
        },
        ..Setup::default()
    };
    let p = plan(
        64 * MIB,
        vec![phase(
            KernelId::DiskBench,
            60,
            job(4096, 1 << 20, 0, 0, 4, 1),
        )],
    );
    let r = run(&p, setup);
    assert!(r.notices().iter().any(|n| n.code == "disk_full"));
    assert!(r.errors().is_empty());
    assert_eq!(r.end.finished.reason, FinishReason::Failed);
    assert_eq!(r.end.exit_code, EXIT_OK);
    // No retry loop: at most the queue's in-flight writes hit the full disk.
    let over = r.disk.write_bytes.load(Ordering::Relaxed) - 8 * MIB;
    assert!(over <= 4 * MIB, "{over} bytes past the full disk");
    assert!(r.real < Duration::from_secs(2));
}

#[test]
fn transient_io_error_is_reported_and_continues() {
    let setup = Setup {
        script: Script {
            fail_once: Some((10, 1117)),
            ..Script::default()
        },
        ..Setup::default()
    };
    let p = plan(
        64 * MIB,
        vec![phase(
            KernelId::DiskBench,
            2,
            job(4096, 1 << 20, 0, 100, 1, 1),
        )],
    );
    let r = run(&p, setup);
    let errs = r.errors();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, ErrorKind::IoError);
    assert_eq!(errs[0].transient, Some(true));
    assert_eq!(errs[0].actual, 1117);
    // Q1T1 sequential 1 MiB: the 11th I/O is at 10 MiB.
    assert_eq!(errs[0].iteration, 10 * MIB / 4096);
    assert_eq!(errs[0].logical, None);
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
    assert_eq!(r.end.finished.errors, 1);
    assert_eq!(r.end.exit_code, EXIT_OK);
}

#[test]
fn persistent_io_error_exits_with_5() {
    let setup = Setup {
        script: Script {
            fail_from: Some((10, 1167)),
            ..Script::default()
        },
        ..Setup::default()
    };
    let p = plan(
        64 * MIB,
        vec![
            phase(KernelId::DiskBench, 60, job(4096, 1 << 20, 0, 100, 4, 1)),
            phase(KernelId::DiskBench, 60, job(4096, 1 << 20, 0, 100, 4, 1)),
        ],
    );
    let r = run(&p, setup);
    let errs = r.errors();
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert_eq!(errs[0].kind, ErrorKind::IoError);
    assert_eq!(errs[0].transient, Some(false));
    assert_eq!(errs[0].actual, 1167);
    assert_eq!(r.end.finished.reason, FinishReason::Failed);
    assert_eq!(r.end.exit_code, EXIT_IO);
    assert!(r.done().is_empty());
    assert!(r.real < Duration::from_secs(2));
}

#[test]
fn stop_finishes_stopped_without_errors() {
    let setup = Setup {
        stop_after_ms: Some(100),
        ..Setup::default()
    };
    let p = plan(
        64 * MIB,
        vec![phase(
            KernelId::DiskBench,
            600,
            job(4096, 1 << 20, 100, 50, 8, 2),
        )],
    );
    let r = run(&p, setup);
    assert_eq!(r.end.finished.reason, FinishReason::Stopped);
    assert!(r.errors().is_empty());
    assert_eq!(
        r.disk.completed.load(Ordering::Relaxed),
        r.disk.submitted.load(Ordering::Relaxed)
    );
}

#[test]
fn access_denied_sends_the_notice_before_any_phase() {
    let setup = Setup {
        open_error: Some(DiskError::AccessDenied),
        ..Setup::default()
    };
    let p = plan(
        64 * MIB,
        vec![phase(
            KernelId::DiskFill,
            900,
            job(1 << 20, 1 << 20, 0, 0, 4, 1),
        )],
    );
    let r = run(&p, setup);
    assert!(matches!(
        r.msgs.first(),
        Some(LoadMessage::Notice(n)) if n.code == "access_denied"
    ));
    assert!(r.done().is_empty());
    assert_eq!(r.disk.submitted.load(Ordering::Relaxed), 0);
    assert_eq!(r.end.finished.reason, FinishReason::Failed);
    assert_eq!(r.end.exit_code, EXIT_OK);
}

#[test]
fn block_smaller_than_the_sector_skips_with_disk_sector() {
    let setup = Setup {
        sector: 8192,
        ..Setup::default()
    };
    let p = plan(
        64 * MIB,
        vec![phase(
            KernelId::DiskBench,
            5,
            job(4096, 1 << 20, 100, 100, 4, 1),
        )],
    );
    let r = run(&p, setup);
    assert!(r
        .notices()
        .iter()
        .any(|n| n.code == "disk_sector" && n.value == Some(8192)));
    assert_eq!(r.done()[0].skipped.as_deref(), Some("disk_sector"));
    assert_eq!(r.disk.submitted.load(Ordering::Relaxed), 0);
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
}

#[test]
#[cfg(windows)]
#[ignore = "requires real Windows hardware"]
fn bench_on_a_real_file() {
    let dir = std::env::temp_dir().join(format!("oma-c4-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut p = plan(
        64 * MIB,
        vec![
            phase(KernelId::DiskFill, 60, job(1 << 20, 1 << 20, 0, 0, 4, 1)),
            phase(KernelId::DiskBench, 1, job(4096, 1 << 20, 100, 100, 4, 1)),
        ],
    );
    p.disk.as_mut().unwrap().dir = dir.to_str().unwrap().to_owned();
    let msgs = Mutex::new(Vec::new());
    let out = |m: LoadMessage| msgs.lock().unwrap().push(m);
    let end = super::engine::run_disk(&p, &out, &AtomicBool::new(false), None);
    let msgs = msgs.into_inner().unwrap();
    assert_eq!(end.finished.reason, FinishReason::Completed, "{msgs:?}");
    let done: Vec<_> = msgs
        .iter()
        .filter_map(|m| match m {
            LoadMessage::PhaseDone(d) => Some(d),
            _ => None,
        })
        .collect();
    assert_eq!(done.len(), 2);
    assert_eq!(done[0].disk.as_ref().unwrap().write.bytes, 64 * MIB);
    let read = &done[1].disk.as_ref().unwrap().read;
    let rate = read.bytes as f64 / read.elapsed_us as f64;
    assert!(rate.is_finite() && rate > 0.0, "{read:?}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir(&dir).unwrap();
}

fn serving() -> Script {
    Script {
        serve_reads: true,
        ..Script::default()
    }
}

#[test]
fn n1_mixes_reads_and_writes_in_both_sizes() {
    let setup = Setup {
        script: Script {
            check_writes: true,
            ..serving()
        },
        ..Setup::default()
    };
    let p = plan(
        64 * MIB,
        vec![phase(KernelId::N1, 3, job(4096, 128 << 10, 50, 70, 16, 2))],
    );
    let r = run(&p, setup);
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
    assert!(r.errors().is_empty(), "{:?}", r.errors());
    let shapes = r.disk.shapes.lock().unwrap().clone();
    for shape in [
        (false, 4096),
        (true, 4096),
        (false, 128 << 10),
        (true, 128 << 10),
    ] {
        assert!(shapes.contains(&shape), "{shape:?} missing in {shapes:?}");
    }
    assert_eq!(r.disk.bad_blocks.load(Ordering::Relaxed), 0);
    let done = r.done();
    let read = done[0].disk.as_ref().unwrap().read.bytes;
    assert!(read > 0);
    assert_eq!(done[0].checks, read / 4096);
    assert_eq!(r.end.finished.checks, done[0].checks);
}

fn n3_with_bad_block(bad_reads: u32) -> Ran {
    let setup = Setup {
        script: Script {
            bad_offset: Some(0),
            bad_reads,
            ..serving()
        },
        ..Setup::default()
    };
    let p = plan(
        64 * MIB,
        vec![phase(KernelId::N3, 1, job(4096, 1 << 20, 0, 100, 8, 1))],
    );
    run(&p, setup)
}

#[test]
fn n3_reads_are_verified_and_errors_classified() {
    // Bad twice: the reread is bad too, so the error is persistent.
    let r = n3_with_bad_block(2);
    let errors = r.errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    let e = errors[0];
    assert_eq!(e.kind, ErrorKind::Misplaced);
    assert_eq!(e.kernel, KernelId::N3);
    assert_eq!((e.iteration, e.expected, e.actual), (0, 0, 1000));
    assert_eq!((e.logical, e.core, e.isa), (None, None, Isa::Sse2));
    assert_eq!(e.transient, Some(false));
    assert_eq!(r.done()[0].errors, 1);
    assert_eq!(r.end.finished.errors, 1);
}

#[test]
fn a_bad_read_that_reads_right_the_second_time_is_transient() {
    let r = n3_with_bad_block(1);
    let errors = r.errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].kind, ErrorKind::Misplaced);
    assert_eq!(errors[0].transient, Some(true));
    assert_eq!(r.done()[0].errors, 1);
}

#[test]
fn n2_sends_the_cliff_notice() {
    let setup = Setup {
        script: Script {
            slow_after_ios: Some(600),
            ..Script::default()
        },
        ..Setup::default()
    };
    let p = plan(
        64 * MIB,
        vec![phase(KernelId::N2, 30, job(4096, 4096, 0, 0, 1, 1))],
    );
    let r = run(&p, setup);
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
    let notice = |code: &str| {
        r.notices()
            .into_iter()
            .filter(|n| n.code == code)
            .collect::<Vec<_>>()
    };
    let cliff = notice("slc_cliff");
    assert_eq!(cliff.len(), 1, "{:?}", r.notices());
    // The bytes written when the speed fell: after the fast I/Os, long before the end.
    let fast = 600 * 4096;
    let at = cliff[0].value.unwrap();
    assert!(at >= fast / 2 && at <= fast * 2, "{at}");
    let written = r.done()[0].disk.as_ref().unwrap().write.bytes;
    assert!(at < written, "{at} of {written}");
    let steady = notice("slc_steady");
    assert_eq!(steady.len(), 1);
    assert!(steady[0].value.unwrap() > 0);
}

#[test]
fn n2_without_a_cliff_sends_no_slc_notice() {
    let p = plan(
        64 * MIB,
        vec![phase(KernelId::N2, 12, job(4096, 1 << 20, 0, 0, 1, 1))],
    );
    let r = run(&p, Setup::default());
    assert!(r.notices().iter().all(|n| !n.code.starts_with("slc_")));
}
