//! V1-V4 and the injected fault, on an in-memory "disk" that can corrupt, move, drop or zero
//! blocks (at most 3 s each), and one run of V1 on a real 64 MiB file.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use oma_core::disk_block::{write_block, BLOCK_BYTES};
use oma_ipc::load::{
    ComputeError, DiskJob, DiskTarget, ErrorKind, FinishReason, KernelId, LoadMessage, PhaseDone,
};

use super::engine::{
    run_disk_with, used_blocks, DataFileApi, DiskHooks, DiskRunEnd, IoDone, IoQueue,
};
use super::file::DiskError;
use super::offsets::IoReq;
use super::tests::{job, phase, plan, Clock, MIB, SEED};
use crate::args::Inject;

const GIB: u64 = 1 << 30;
const MAIN: i64 = -1;
const SYNC: i64 = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ev {
    Write(i64, u64),
    Read(i64, u64),
    Flush(i64),
}

#[derive(Default, Clone)]
struct Faults {
    /// File id -> the file whose storage it really uses.
    alias: HashMap<i64, i64>,
    /// The write with this index (from 0, over the whole run) is ignored.
    drop_write: Option<u64>,
    /// The first N reads of the chunk at 15 MiB come back with bit 0 of byte 100 flipped.
    corrupt_reads: u32,
    /// Every read comes back as zeros.
    zero_reads: bool,
}

struct Mem {
    faults: Faults,
    store: Mutex<HashMap<i64, Vec<u8>>>,
    events: Mutex<Vec<Ev>>,
    writes: AtomicU64,
    reads: AtomicU64,
    free: AtomicU64,
}

fn id_of(suffix: Option<&str>) -> i64 {
    match suffix {
        None => MAIN,
        Some("-sync") => SYNC,
        Some(s) => s.trim_start_matches('-').parse().unwrap(),
    }
}

struct MemFile {
    id: i64,
    bytes: u64,
    mem: Arc<Mem>,
}

impl DataFileApi for MemFile {
    fn bytes(&self) -> u64 {
        self.bytes
    }
    fn sector(&self) -> u32 {
        4096
    }
    fn flush(&self) -> Result<(), DiskError> {
        self.mem.events.lock().unwrap().push(Ev::Flush(self.id));
        Ok(())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

struct MemQueue {
    mem: Arc<Mem>,
    id: i64,
    key: i64,
    bufs: Vec<Vec<u8>>,
    pending: Vec<(usize, u32)>,
}

impl IoQueue for MemQueue {
    fn submit(&mut self, req: IoReq, slot: usize) -> Result<(), DiskError> {
        let (off, len) = (req.offset as usize, req.len as usize);
        let mut store = self.mem.store.lock().unwrap();
        let data = store.get_mut(&self.key).unwrap();
        if req.write {
            let n = self.mem.writes.fetch_add(1, Ordering::Relaxed);
            if self.mem.faults.drop_write != Some(n) {
                data[off..off + len].copy_from_slice(&self.bufs[slot][..len]);
            }
            self.mem
                .events
                .lock()
                .unwrap()
                .push(Ev::Write(self.id, req.offset));
        } else {
            let n = if req.offset == 15 * MIB {
                self.mem.reads.fetch_add(1, Ordering::Relaxed)
            } else {
                u64::MAX
            };
            let buf = &mut self.bufs[slot][..len];
            buf.copy_from_slice(&data[off..off + len]);
            if self.mem.faults.zero_reads {
                buf.fill(0);
            } else if n < u64::from(self.mem.faults.corrupt_reads) {
                buf[100] ^= 1;
            }
            self.mem
                .events
                .lock()
                .unwrap()
                .push(Ev::Read(self.id, req.offset));
        }
        self.pending.push((slot, req.len));
        Ok(())
    }

    fn wait(&mut self, _timeout_ms: u32, out: &mut Vec<IoDone>) -> Result<(), DiskError> {
        thread::sleep(Duration::from_micros(20));
        for (slot, len) in self.pending.drain(..) {
            out.push(IoDone {
                slot,
                result: Ok(len),
                latency_us: 100,
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

#[derive(Default)]
struct MemSetup {
    faults: Faults,
    /// The main file starts as the valid generation-1 blocks a `disk_fill` leaves.
    preload: bool,
    /// How many 1 GiB parts V3 can still create.
    free_parts: u64,
    inject: Option<Inject>,
    speed: u32,
}

struct MemRan {
    end: DiskRunEnd,
    msgs: Vec<LoadMessage>,
    mem: Arc<Mem>,
    opens: Vec<(Option<String>, u64, bool)>,
}

impl MemRan {
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

    fn events(&self) -> Vec<Ev> {
        self.mem.events.lock().unwrap().clone()
    }

    fn reads(&self) -> Vec<(i64, u64)> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                Ev::Read(f, o) => Some((f, o)),
                _ => None,
            })
            .collect()
    }
}

const PART_BYTES: u64 = 4 * MIB;

fn run_mem(p: &oma_ipc::load::Plan, setup: MemSetup) -> MemRan {
    let reserve = p.disk.as_ref().unwrap().reserve_bytes;
    let mem = Arc::new(Mem {
        faults: setup.faults.clone(),
        store: Mutex::new(HashMap::new()),
        events: Mutex::new(Vec::new()),
        writes: AtomicU64::new(0),
        reads: AtomicU64::new(0),
        // Room for the main file, never for a whole part more.
        free: AtomicU64::new(reserve + 64 * MIB + setup.free_parts * GIB),
    });
    let opens = Mutex::new(Vec::new());
    let msgs = Mutex::new(Vec::new());
    let out = |m: LoadMessage| msgs.lock().unwrap().push(m);
    let stop = AtomicBool::new(false);
    let clock = Clock::new(setup.speed.max(1));
    let now = move || clock.now();
    let omem = Arc::clone(&mem);
    let preload = setup.preload;
    let open = |_: &DiskTarget, suffix: Option<&str>, bytes: u64, wt: bool| {
        opens
            .lock()
            .unwrap()
            .push((suffix.map(str::to_owned), bytes, wt));
        let id = id_of(suffix);
        // V3 parts are small here: the loads use the size the file reports.
        let bytes = match suffix {
            Some(s) if s != "-sync" => {
                omem.free.fetch_sub(GIB, Ordering::Relaxed);
                PART_BYTES
            }
            _ => bytes,
        };
        let key = *omem.faults.alias.get(&id).unwrap_or(&id);
        let mut store = omem.store.lock().unwrap();
        store.entry(key).or_insert_with(|| {
            let mut v = vec![0u8; bytes as usize];
            if preload && id == MAIN {
                for (i, b) in v.chunks_exact_mut(BLOCK_BYTES).enumerate() {
                    write_block(b, SEED, i as u64, 1, false);
                }
            }
            v
        });
        Ok(Box::new(MemFile {
            id,
            bytes,
            mem: Arc::clone(&omem),
        }) as Box<dyn DataFileApi>)
    };
    let qmem = Arc::clone(&mem);
    let queue = move |f: &dyn DataFileApi, j: &DiskJob| {
        let f = f.as_any().downcast_ref::<MemFile>().unwrap();
        let size = used_blocks(j).1 as usize;
        let key = *qmem.faults.alias.get(&f.id).unwrap_or(&f.id);
        Ok(Box::new(MemQueue {
            mem: Arc::clone(&qmem),
            id: f.id,
            key,
            bufs: vec![vec![0u8; size]; usize::from(j.queue)],
            pending: Vec::new(),
        }) as Box<dyn IoQueue>)
    };
    let fmem = Arc::clone(&mem);
    let free = move || Some(fmem.free.load(Ordering::Relaxed));
    let hooks = DiskHooks {
        open: &open,
        queue: &queue,
        free_bytes: &free,
        clock: &now,
    };
    let t0 = Instant::now();
    let end = run_disk_with(p, &out, &stop, setup.inject, &hooks);
    let real = t0.elapsed();
    assert!(real < Duration::from_secs(3), "the test took {real:?}");
    MemRan {
        end,
        msgs: msgs.into_inner().unwrap(),
        mem,
        opens: opens.into_inner().unwrap(),
    }
}

fn v1_job(cycles: u32) -> DiskJob {
    DiskJob {
        cycles: Some(cycles),
        ..job(4096, 1 << 20, 0, 0, 4, 1)
    }
}

fn v1_plan(file_mib: u64, cycles: u32) -> oma_ipc::load::Plan {
    plan(
        file_mib * MIB,
        vec![phase(KernelId::V1, 3600, v1_job(cycles))],
    )
}

#[test]
fn v1_verifies_every_block_each_cycle_in_reverse() {
    let r = run_mem(
        &v1_plan(16, 2),
        MemSetup {
            speed: 50,
            ..MemSetup::default()
        },
    );
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
    assert!(r.errors().is_empty(), "{:?}", r.errors());
    let expected: Vec<_> = (0..16u64).rev().map(|c| (MAIN, c * MIB)).collect();
    let reads = r.reads();
    assert_eq!(reads.len(), 32);
    assert_eq!(&reads[..16], &expected[..]);
    assert_eq!(&reads[16..], &expected[..]);
    // Cycles x blocks.
    assert_eq!(r.done()[0].checks, 2 * 16 * MIB / 4096);
    assert_eq!(r.end.finished.checks, 2 * 16 * MIB / 4096);
}

#[test]
fn v1_flushes_before_verifying() {
    let r = run_mem(
        &v1_plan(16, 2),
        MemSetup {
            speed: 50,
            ..MemSetup::default()
        },
    );
    let ev = r.events();
    let flushes: Vec<_> = ev
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, Ev::Flush(_)))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(flushes.len(), 2);
    for f in flushes {
        assert!(matches!(ev[f - 1], Ev::Write(..)), "{:?}", ev[f - 1]);
        assert!(matches!(ev[f + 1], Ev::Read(..)), "{:?}", ev[f + 1]);
    }
}

fn v2_plan(duration_s: u32, rate_mib: u64) -> oma_ipc::load::Plan {
    let j = DiskJob {
        rate_limit_bps: Some(rate_mib * MIB),
        ..job(4096, 1 << 20, 100, 0, 4, 1)
    };
    plan(16 * MIB, vec![phase(KernelId::V2, duration_s, j)])
}

#[test]
fn v2_finds_a_stale_block() {
    let r = run_mem(
        &v2_plan(2, 1),
        MemSetup {
            preload: true,
            speed: 50,
            faults: Faults {
                drop_write: Some(3),
                ..Faults::default()
            },
            ..MemSetup::default()
        },
    );
    let errors = r.errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    let e = errors[0];
    assert_eq!(e.kind, ErrorKind::Stale);
    assert_eq!((e.expected, e.actual), (2, 1));
    assert_eq!(e.transient, Some(false));
    assert_eq!(e.kernel, KernelId::V2);
    assert!(e.iteration < 16 * MIB / 4096);
}

#[test]
fn v2_respects_the_rate_limit() {
    let r = run_mem(
        &v2_plan(3, 1),
        MemSetup {
            preload: true,
            speed: 50,
            ..MemSetup::default()
        },
    );
    assert!(r.errors().is_empty(), "{:?}", r.errors());
    let written = r.done()[0].disk.as_ref().unwrap().write.bytes;
    // 1 MiB/s for 3 s, plus one block of burst and the engine's last slice.
    assert!(written <= 3 * MIB + 512 * 1024, "{written}");
    assert!(written >= MIB / 2, "{written}");
    // The pass at the end read the whole file.
    assert!(r.done()[0].checks >= 16 * MIB / 4096);
}

#[test]
fn v2_windows_are_300_s_then_the_rest() {
    use super::verify::v2_window;
    let d = Duration::from_secs;
    assert_eq!(v2_window(d(0), d(700)), Some(d(300)));
    assert_eq!(v2_window(d(300), d(700)), Some(d(300)));
    assert_eq!(v2_window(d(650), d(700)), Some(d(50)));
    assert_eq!(v2_window(d(700), d(700)), None);
}

fn v3_plan() -> oma_ipc::load::Plan {
    plan(
        16 * MIB,
        vec![phase(KernelId::V3, 3600, job(4096, 1 << 20, 0, 0, 4, 1))],
    )
}

#[test]
fn v3_fills_the_free_space_and_verifies_all_parts() {
    let r = run_mem(
        &v3_plan(),
        MemSetup {
            free_parts: 3,
            speed: 50,
            ..MemSetup::default()
        },
    );
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
    assert!(r.errors().is_empty(), "{:?}", r.errors());
    let names: Vec<_> = r.opens.iter().filter_map(|(s, ..)| s.clone()).collect();
    assert_eq!(names, ["-0", "-1", "-2"]);
    assert!(r.opens.iter().skip(1).all(|(_, b, wt)| *b == GIB && !wt));
    assert_eq!(r.done()[0].checks, 3 * PART_BYTES / 4096);
    // Verified last part to first, each flushed after its writes.
    let parts: Vec<_> = r.reads().iter().map(|(f, _)| *f).collect();
    assert_eq!(parts.iter().filter(|f| **f == 2).count(), 4);
    assert_eq!(parts[0], 2);
    assert_eq!(*parts.last().unwrap(), 0);
    let ev = r.events();
    for part in 0..3 {
        let flush = ev.iter().position(|e| *e == Ev::Flush(part)).unwrap();
        let first_read = ev
            .iter()
            .position(|e| matches!(e, Ev::Read(f, _) if *f == part))
            .unwrap();
        let last_write = ev
            .iter()
            .rposition(|e| matches!(e, Ev::Write(f, _) if *f == part))
            .unwrap();
        assert!(last_write < flush && flush < first_read, "part {part}");
    }
}

#[test]
fn v3_finds_a_fake_capacity_wraparound() {
    let r = run_mem(
        &v3_plan(),
        MemSetup {
            free_parts: 3,
            speed: 50,
            faults: Faults {
                alias: HashMap::from([(2, 0)]),
                ..Faults::default()
            },
            ..MemSetup::default()
        },
    );
    let errors = r.errors();
    assert!(!errors.is_empty());
    let e = errors[0];
    assert_eq!(e.kind, ErrorKind::Misplaced);
    assert_eq!(e.transient, Some(false));
    // Part 0 reads back part 2's blocks: expected the global index of part 0, found part 2's.
    assert!(e.iteration < PART_BYTES / 4096);
    assert_eq!(e.expected, e.iteration);
    assert_eq!(e.actual, 2 * 262_144 + e.iteration);
}

#[test]
fn v4_writes_through_and_verifies() {
    let j = DiskJob {
        write_cap_bytes: Some(4 * MIB),
        ..job(4096, 64 << 10, 0, 0, 1, 1)
    };
    let p = plan(16 * MIB, vec![phase(KernelId::V4, 600, j)]);
    let r = run_mem(
        &p,
        MemSetup {
            free_parts: 100,
            speed: 50,
            ..MemSetup::default()
        },
    );
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
    assert!(r.errors().is_empty(), "{:?}", r.errors());
    let sync: Vec<_> = r
        .opens
        .iter()
        .filter(|(s, ..)| s.as_deref() == Some("-sync"))
        .collect();
    assert_eq!(sync.len(), 1);
    assert_eq!((sync[0].1, sync[0].2), (4 * MIB, true));
    assert_eq!(r.done()[0].disk.as_ref().unwrap().write.bytes, 4 * MIB);
    assert_eq!(r.done()[0].checks, 4 * MIB / 4096);
    let ev = r.events();
    let flush = ev.iter().position(|e| *e == Ev::Flush(SYNC)).unwrap();
    assert!(matches!(ev[flush - 1], Ev::Write(..)));
    let reads = r.reads();
    assert_eq!(reads.len(), 4);
    assert_eq!(reads[0], (SYNC, 3 * MIB));
    assert_eq!(reads[3], (SYNC, 0));
}

#[test]
fn reread_marks_transient_and_persistent() {
    for (bad, transient) in [(1, true), (2, false)] {
        let r = run_mem(
            &v1_plan(16, 1),
            MemSetup {
                speed: 50,
                faults: Faults {
                    corrupt_reads: bad,
                    ..Faults::default()
                },
                ..MemSetup::default()
            },
        );
        let errors = r.errors();
        assert_eq!(errors.len(), 1, "bad {bad}: {errors:?}");
        assert_eq!(errors[0].kind, ErrorKind::BitFlip);
        assert_eq!(errors[0].transient, Some(transient));
    }
}

#[test]
fn errors_per_phase_are_capped_at_16() {
    let r = run_mem(
        &v1_plan(16, 1),
        MemSetup {
            speed: 50,
            faults: Faults {
                zero_reads: true,
                ..Faults::default()
            },
            ..MemSetup::default()
        },
    );
    assert_eq!(r.errors().len(), 16);
    assert_eq!(r.done()[0].errors, 16 * MIB / 4096);
    assert!(r.errors().iter().all(|e| e.kind == ErrorKind::Zeros));
    // The data errors do not end the run.
    assert_eq!(r.end.finished.reason, FinishReason::Completed);
}

#[cfg(debug_assertions)]
#[test]
fn inject_flips_one_byte_in_the_first_verify_pass() {
    for kernel in [KernelId::V1, KernelId::V2, KernelId::V3, KernelId::V4] {
        let (p, free_parts) = match kernel {
            KernelId::V1 => (v1_plan(16, 1), 0),
            KernelId::V2 => (v2_plan(1, 1), 0),
            KernelId::V3 => (v3_plan(), 1),
            _ => {
                let j = DiskJob {
                    write_cap_bytes: Some(2 * MIB),
                    ..job(4096, 64 << 10, 0, 0, 1, 1)
                };
                (plan(16 * MIB, vec![phase(KernelId::V4, 600, j)]), 100)
            }
        };
        let r = run_mem(
            &p,
            MemSetup {
                preload: kernel == KernelId::V2,
                free_parts,
                speed: 50,
                inject: Some(Inject { kernel, core: None }),
                ..MemSetup::default()
            },
        );
        let errors = r.errors();
        assert_eq!(errors.len(), 1, "{kernel:?}: {errors:?}");
        // One flipped bit, and the reread (without the flip) is right.
        assert_eq!(errors[0].kind, ErrorKind::BitFlip, "{kernel:?}");
        assert_eq!(errors[0].actual, 1, "{kernel:?}");
        assert_eq!(errors[0].transient, Some(true), "{kernel:?}");
    }
}

#[cfg(debug_assertions)]
#[test]
fn inject_for_another_kernel_does_nothing() {
    let r = run_mem(
        &v1_plan(16, 1),
        MemSetup {
            speed: 50,
            inject: Some(Inject {
                kernel: KernelId::V2,
                core: None,
            }),
            ..MemSetup::default()
        },
    );
    assert!(r.errors().is_empty());
}

#[test]
#[cfg(windows)]
#[ignore = "requires real Windows hardware"]
fn v1_on_a_real_file() {
    let dir = std::env::temp_dir().join(format!("oma-c6-v1-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut p = v1_plan(64, 1);
    p.disk.as_mut().unwrap().dir = dir.to_str().unwrap().to_owned();
    let msgs = Mutex::new(Vec::new());
    let out = |m: LoadMessage| msgs.lock().unwrap().push(m);
    let end = super::engine::run_disk(&p, &out, &AtomicBool::new(false), None);
    let msgs = msgs.into_inner().unwrap();
    assert_eq!(end.finished.reason, FinishReason::Completed, "{msgs:?}");
    assert_eq!(end.finished.errors, 0, "{msgs:?}");
    assert_eq!(end.finished.checks, 64 * MIB / 4096);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir(&dir).unwrap();
}
