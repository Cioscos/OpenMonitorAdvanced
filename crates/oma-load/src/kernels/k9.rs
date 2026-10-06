//! K9, core-to-core exchange (§4.1, DA9): worker `i` pairs with `i + workers/2` (with the
//! core order of DA4 a pair crosses the CCDs when there are several). Each pair shares a
//! ring of 64 cache lines (6 data words, a sequence number, a checksum). In every round
//! one side writes the 64 lines and the other reads and checks them, then the roles swap,
//! so the lines keep moving between the two cores. Self-checking (DA7): no reference.
//!
//! An odd worker count leaves the last worker without a partner; the engine drops it.

use std::any::Any;
use std::hint::spin_loop;
use std::sync::atomic::{AtomicU64, Ordering::*};
use std::time::{Duration, Instant};

use crate::kernel::{Check, Kernel, KernelError, KernelFactory, RefFailure, WorkerCtx};
use crate::rng::Xoshiro256ss;
use crate::verify::digest_words;

const LINES: usize = 64;
const DATA: usize = 6;
/// Words of a line after the data: sequence, checksum.
const SEQ: usize = DATA;
const SUM: usize = DATA + 1;
/// A waiting side bumps `beat` this often.
const BEAT_EVERY: Duration = Duration::from_millis(100);

#[repr(align(64))]
struct Line([AtomicU64; 8]);

/// The rings of all the pairs of a phase, in `PhaseShared::slot`.
struct Rings(Vec<Vec<Line>>);

fn new_rings(pairs: usize) -> Rings {
    Rings(
        (0..pairs)
            .map(|_| {
                (0..LINES)
                    .map(|_| Line(std::array::from_fn(|_| AtomicU64::new(0))))
                    .collect()
            })
            .collect(),
    )
}

pub(crate) struct K9 {
    ctx: WorkerCtx,
    pair: usize,
    /// Whether this is the lower worker, which writes the even rounds.
    lower: bool,
    round: u64,
    /// Test hook: (data word index over the whole ring, bit) flipped once in a line this
    /// worker writes, after its checksum.
    #[cfg(test)]
    pub(crate) flip: Option<(usize, u32)>,
}

impl K9 {
    pub(crate) fn new(ctx: &WorkerCtx) -> Result<Self, KernelError> {
        let half = ctx.workers / 2;
        let i = ctx.worker;
        if half == 0 || i >= 2 * half {
            return Err(KernelError::Unsupported);
        }
        ctx.shared
            .slot
            .get_or_init(|| Box::new(new_rings(half as usize)) as Box<dyn Any + Send + Sync>);
        Ok(Self {
            ctx: ctx.clone(),
            pair: (i % half) as usize,
            lower: i < half,
            round: 0,
            #[cfg(test)]
            flip: None,
        })
    }

    fn lines(&self) -> &[Line] {
        let rings = self
            .ctx
            .shared
            .slot
            .get()
            .and_then(|s| s.downcast_ref::<Rings>())
            .expect("the rings are set by the first worker");
        &rings.0[self.pair]
    }

    /// The data words of `line` in `round`: different every round, so a stale line shows.
    fn words(&self, round: u64, line: usize) -> [u64; DATA] {
        let mut rng = Xoshiro256ss::new(
            self.ctx.seed
                ^ (self.pair as u64).rotate_left(48)
                ^ round.wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ (line as u64).rotate_left(32),
        );
        std::array::from_fn(|_| rng.next_u64())
    }

    fn write(&mut self, round: u64) {
        #[cfg(test)]
        let mut flip = self.flip.take();
        for (l, line) in self.lines().iter().enumerate() {
            #[allow(unused_mut)] // only the test hook mutates it
            let mut data = self.words(round, l);
            let seq = round * LINES as u64 + l as u64 + 1;
            let sum = digest_words(&[data[0], data[1], data[2], data[3], data[4], data[5], seq]);
            #[cfg(test)]
            if let Some((w, bit)) = flip.filter(|f| f.0 / DATA == l) {
                data[w % DATA] ^= 1 << bit;
                flip = None;
            }
            for (w, d) in data.iter().enumerate() {
                line.0[w].store(*d, Relaxed);
            }
            line.0[SUM].store(sum, Relaxed);
            line.0[SEQ].store(seq, Release);
        }
    }

    fn read(&self, round: u64, beat: &AtomicU64) -> Check {
        let mut last = Instant::now();
        for (l, line) in self.lines().iter().enumerate() {
            let want = round * LINES as u64 + l as u64 + 1;
            let mut spins = 0u32;
            let seq = loop {
                let seq = line.0[SEQ].load(Acquire);
                if seq >= want {
                    break seq;
                }
                spin_loop();
                spins += 1;
                if spins.is_multiple_of(1024) {
                    if self.ctx.shared.quit.load(Relaxed) {
                        // Stopped, or the partner is gone: nothing to check.
                        return Check::Ok;
                    }
                    if last.elapsed() >= BEAT_EVERY {
                        beat.fetch_add(1, Relaxed);
                        last = Instant::now();
                    }
                }
            };
            if seq != want {
                return Check::Mismatch {
                    expected: want,
                    actual: seq,
                };
            }
            let mut all: [u64; DATA + 1] = std::array::from_fn(|w| line.0[w].load(Relaxed));
            all[SEQ] = seq;
            let (sum, actual) = (line.0[SUM].load(Relaxed), digest_words(&all));
            if actual != sum {
                return Check::Mismatch {
                    expected: sum,
                    actual,
                };
            }
            let mut expected = [0u64; DATA + 1];
            expected[..DATA].copy_from_slice(&self.words(round, l));
            expected[SEQ] = seq;
            if all != expected {
                // The checksum holds but the content is another round's.
                return Check::Mismatch {
                    expected: digest_words(&expected),
                    actual,
                };
            }
        }
        Check::Ok
    }
}

impl Kernel for K9 {
    fn iterate(&mut self, beat: &AtomicU64) -> Check {
        let round = self.round;
        self.round += 1;
        if self.lower == round.is_multiple_of(2) {
            self.write(round);
            Check::Ok
        } else {
            self.read(round, beat)
        }
    }
}

pub(crate) struct K9Factory;

impl KernelFactory for K9Factory {
    fn reference(&self, _: &WorkerCtx) -> Option<Result<Vec<u64>, RefFailure>> {
        None
    }

    fn worker(&self, ctx: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError> {
        Ok(Box::new(K9::new(ctx)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{PhaseShared, ThreadBudget};
    use oma_ipc::load::{DataSize, Isa};
    use std::sync::Arc;
    use std::thread;

    fn ctxs(workers: u32) -> Vec<WorkerCtx> {
        let shared = Arc::new(PhaseShared::default());
        (0..workers)
            .map(|worker| WorkerCtx {
                isa: Isa::Sse2,
                size: DataSize::Auto,
                budget: ThreadBudget::default(),
                seed: 11,
                worker,
                workers,
                patterns: Vec::new(),
                shared: Arc::clone(&shared),
            })
            .collect()
    }

    #[test]
    fn k9_two_threads_exchange_without_errors() {
        let c = ctxs(2);
        let mut ks: Vec<K9> = c.iter().map(|c| K9::new(c).unwrap()).collect();
        let t = Instant::now();
        let rounds: Vec<u64> = thread::scope(|s| {
            let hs: Vec<_> = ks
                .iter_mut()
                .map(|k| {
                    s.spawn(|| {
                        let beat = AtomicU64::new(0);
                        let mut n = 0u64;
                        while t.elapsed() < Duration::from_millis(800) {
                            assert_eq!(k.iterate(&beat), Check::Ok);
                            n += 1;
                        }
                        // Release the partner, which may wait for our last round.
                        k.ctx.shared.quit.store(true, Relaxed);
                        n
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(rounds.iter().all(|&n| n > 100), "{rounds:?}");
    }

    #[test]
    fn k9_corrupted_line_is_a_mismatch() {
        let c = ctxs(2);
        let mut w = K9::new(&c[0]).unwrap();
        let mut r = K9::new(&c[1]).unwrap();
        w.flip = Some((5 * DATA + 2, 9));
        let beat = AtomicU64::new(0);
        assert_eq!(w.iterate(&beat), Check::Ok);
        assert!(matches!(r.iterate(&beat), Check::Mismatch { .. }));
    }

    #[test]
    fn k9_single_worker_is_unsupported() {
        for (workers, worker) in [(1, 0), (3, 2)] {
            let mut c = ctxs(workers);
            assert_eq!(
                K9Factory.worker(&c.remove(worker)).err(),
                Some(KernelError::Unsupported)
            );
        }
    }

    #[test]
    fn k9_stop_unblocks_a_waiting_side() {
        let c = ctxs(2);
        // Worker 1 reads round 0, which nobody writes.
        let mut r = K9::new(&c[1]).unwrap();
        let shared = Arc::clone(&c[1].shared);
        let t = Instant::now();
        thread::scope(|s| {
            let h = s.spawn(|| r.iterate(&AtomicU64::new(0)));
            thread::sleep(Duration::from_millis(200));
            shared.quit.store(true, Relaxed);
            assert_eq!(h.join().unwrap(), Check::Ok);
        });
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}
