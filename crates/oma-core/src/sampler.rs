//! Background sampling loop: one coalesced timer, never raises the system
//! timer resolution (spec §4.1).

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{JoinHandle, Thread};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::engine::{Engine, TickOutput};

/// Next tick after `prev`. Ticks missed while the machine slept are skipped
/// instead of being replayed in a burst.
pub fn next_deadline(prev: Instant, now: Instant, interval: Duration) -> Instant {
    let next = prev + interval;
    if next > now {
        return next;
    }
    now + interval // Coalesce missed ticks without integer overflow or catch-up bursts.
}

/// Validated application sampling interval; tests may use shorter intervals directly.
pub fn sample_interval(ms: u64) -> Result<Duration, &'static str> {
    if (500..=5_000).contains(&ms) {
        Ok(Duration::from_millis(ms))
    } else {
        Err("sampling interval must be 500..=5000 ms")
    }
}

pub fn history_capacity(interval: Duration) -> usize {
    (3_600_000u128 / interval.as_millis()) as usize
}

/// Wall-clock time in milliseconds since the Unix epoch.
pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A tick that panics is logged on the 1st, 2nd, 4th, 8th... panic in a row:
/// a tick that panics every second must not flood the log.
fn log_panic(consecutive: u64) -> bool {
    consecutive.is_power_of_two()
}

/// Text of a panic payload (`panic!` with a literal or a formatted message).
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

/// The sampling interval, shared between the shell (which changes it when the
/// setting changes) and the sampler thread (which reads it after every tick).
/// `set` wakes the sampler so a new interval applies without waiting for the
/// old one to run out. Clones share the same value.
#[derive(Clone)]
pub struct IntervalHandle(Arc<IntervalShared>);

struct IntervalShared {
    millis: AtomicU64,
    /// The sampler thread to wake, once one is running.
    sleeper: Mutex<Option<Thread>>,
}

impl IntervalHandle {
    pub fn new(interval: Duration) -> Self {
        Self(Arc::new(IntervalShared {
            millis: AtomicU64::new(Self::to_millis(interval)),
            sleeper: Mutex::new(None),
        }))
    }

    /// At least one millisecond: a zero interval would make the sampler spin.
    fn to_millis(interval: Duration) -> u64 {
        u64::try_from(interval.as_millis())
            .unwrap_or(u64::MAX)
            .max(1)
    }

    pub fn get(&self) -> Duration {
        Duration::from_millis(self.0.millis.load(Ordering::Acquire))
    }

    pub fn set(&self, interval: Duration) {
        self.0
            .millis
            .store(Self::to_millis(interval), Ordering::Release);
        if let Some(thread) = self
            .0
            .sleeper
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            thread.unpark();
        }
    }

    /// Called by the sampler thread, so `set` can wake it.
    fn register_sleeper(&self) {
        *self
            .0
            .sleeper
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(std::thread::current());
    }
}

pub struct Sampler {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Sampler {
    /// Starts ticking `engine` every `interval` (which may change while it
    /// runs), calling `on_tick` after each tick with the engine lock already
    /// released.
    pub fn spawn<F>(engine: Arc<Mutex<Engine>>, interval: IntervalHandle, on_tick: F) -> Self
    where
        F: FnMut(&TickOutput) + Send + 'static,
    {
        let tick = move |timestamp_ms, monotonic_ms| {
            engine
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .tick(timestamp_ms, monotonic_ms)
        };
        Self::spawn_ticker(interval, tick, on_tick)
    }

    /// The loop behind `spawn`, with the tick injectable for tests. A panic in
    /// `tick` must not end sampling (spec §8): it is caught, logged, that
    /// iteration publishes nothing and the loop goes on. A panic while the
    /// engine lock is held poisons the mutex; `spawn` keeps locking it through
    /// the poison, like the Tauri commands do.
    ///
    /// The interval is read again after every tick and while waiting. When it
    /// changes, the next tick is due one new interval after the start of the
    /// last tick, or right away if that moment has already passed: the new
    /// pace holds from the next tick on, with no burst of catch-up ticks.
    fn spawn_ticker<T, F>(interval: IntervalHandle, mut tick: T, mut on_tick: F) -> Self
    where
        T: FnMut(u64, u64) -> TickOutput + Send + 'static,
        F: FnMut(&TickOutput) + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("oma-sampler".into())
            .spawn(move || {
                interval.register_sleeper();
                let epoch = Instant::now();
                // When the tick that just ran was due (its start, without drift).
                let mut anchor = epoch;
                let mut panics = 0u64;
                while !stop_flag.load(Ordering::Acquire) {
                    let monotonic_ms = epoch.elapsed().as_millis() as u64;
                    match catch_unwind(AssertUnwindSafe(|| tick(unix_ms(), monotonic_ms))) {
                        Ok(output) => {
                            panics = 0;
                            on_tick(&output);
                        }
                        Err(payload) => {
                            panics += 1;
                            if log_panic(panics) {
                                tracing::error!(
                                    panic = panic_message(payload.as_ref()),
                                    consecutive = panics,
                                    "engine tick panicked"
                                );
                            }
                        }
                    }
                    let mut waiting_for = interval.get();
                    let mut deadline = next_deadline(anchor, Instant::now(), waiting_for);
                    while !stop_flag.load(Ordering::Acquire) {
                        let now = Instant::now();
                        let current = interval.get();
                        if current != waiting_for {
                            waiting_for = current;
                            deadline = (anchor + current).max(now);
                        }
                        if now >= deadline {
                            break;
                        }
                        std::thread::park_timeout(deadline - now);
                    }
                    anchor = deadline;
                }
            })
            .expect("failed to spawn the sampler thread");
        Self {
            stop,
            thread: Some(thread),
        }
    }

    /// Stops the loop and waits for the current tick to finish.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
    use crate::provider::{Inventory, Provider, ProviderError};
    use std::sync::mpsc;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn next_deadline_advances_by_one_interval() {
        let t0 = Instant::now();
        assert_eq!(next_deadline(t0, t0 + ms(10), ms(1_000)), t0 + ms(1_000));
    }

    #[test]
    fn next_deadline_skips_missed_ticks_after_sleep() {
        let t0 = Instant::now();
        assert_eq!(
            next_deadline(t0, t0 + ms(10_500), ms(1_000)),
            t0 + ms(11_500)
        );
    }

    #[test]
    fn next_deadline_on_exact_boundary_moves_forward() {
        let t0 = Instant::now();
        assert_eq!(next_deadline(t0, t0 + ms(3_000), ms(1_000)), t0 + ms(4_000));
    }

    struct Const;

    impl Provider for Const {
        fn name(&self) -> &'static str {
            "const"
        }

        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            Ok(Inventory {
                devices: vec![Device {
                    id: "d".into(),
                    kind: DeviceKind::Cpu,
                    name: "d".into(),
                    vendor: None,
                    properties: Default::default(),
                }],
                sensors: vec![Sensor::new(
                    "d",
                    SensorKind::Load,
                    "x",
                    Unit::Percent,
                    Label::new("t"),
                    Source::Mock,
                )],
            })
        }

        fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
            Ok(vec![Some(42.0)])
        }
    }

    #[test]
    fn configured_interval_retains_one_hour() {
        assert!(sample_interval(499).is_err());
        assert!(sample_interval(5_001).is_err());
        for (ms, capacity) in [(500, 7200), (1000, 3600), (5000, 720)] {
            assert_eq!(history_capacity(sample_interval(ms).unwrap()), capacity);
        }
    }

    fn output(seq: u64) -> TickOutput {
        TickOutput {
            snapshot: crate::model::Snapshot {
                revision: 1,
                seq,
                timestamp_ms: 0,
                values: Vec::new(),
            },
            schema: None,
            quality: Vec::new(),
            health: None,
            entries: Vec::new(),
            monotonic_ms: 0,
        }
    }

    #[test]
    fn a_panicking_tick_is_skipped_and_sampling_goes_on() {
        let mut calls = 0u64;
        let tick = move |_: u64, _: u64| {
            calls += 1;
            if calls == 2 {
                panic!("tick {calls} failed");
            }
            output(calls)
        };
        let (tx, rx) = mpsc::channel();
        let sampler = Sampler::spawn_ticker(IntervalHandle::new(ms(10)), tick, move |out| {
            let _ = tx.send(out.snapshot.seq);
        });
        let seqs: Vec<u64> = (0..3)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        sampler.stop();
        // The second tick panicked: nothing was published for it.
        assert_eq!(seqs, vec![1, 3, 4]);
    }

    #[test]
    fn a_poisoned_engine_keeps_sampling() {
        let engine = Arc::new(Mutex::new(Engine::new(vec![Box::new(Const)], 16)));
        let poisoner = engine.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the engine mutex");
        })
        .join();
        assert!(engine.is_poisoned());
        let (tx, rx) = mpsc::channel();
        let sampler = Sampler::spawn(engine, IntervalHandle::new(ms(10)), move |out| {
            let _ = tx.send(out.snapshot.values.clone());
        });
        let values = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        sampler.stop();
        assert_eq!(values, vec![Some(42.0)]);
    }

    #[test]
    fn repeated_panics_are_logged_with_exponential_spacing() {
        let logged: Vec<u64> = (1..=20).filter(|&n| log_panic(n)).collect();
        assert_eq!(logged, vec![1, 2, 4, 8, 16]);
    }

    #[test]
    fn panic_messages_are_extracted_from_both_payload_kinds() {
        let literal = catch_unwind(|| panic!("literal")).unwrap_err();
        assert_eq!(panic_message(literal.as_ref()), "literal");
        let formatted = catch_unwind(|| panic!("tick {}", 7)).unwrap_err();
        assert_eq!(panic_message(formatted.as_ref()), "tick 7");
        let other = catch_unwind(|| std::panic::panic_any(5u8)).unwrap_err();
        assert_eq!(panic_message(other.as_ref()), "non-string panic payload");
    }

    #[test]
    fn sampler_ticks_and_stops_promptly() {
        let engine = Arc::new(Mutex::new(Engine::new(vec![Box::new(Const)], 16)));
        let (tx, rx) = mpsc::channel();
        let sampler = Sampler::spawn(engine.clone(), IntervalHandle::new(ms(20)), move |out| {
            let _ = tx.send(out.snapshot.seq);
        });
        let seqs: Vec<u64> = (0..3)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        assert_eq!(seqs, vec![1, 2, 3]);
        let started = Instant::now();
        sampler.stop();
        assert!(started.elapsed() < ms(500));
        assert!(engine.lock().unwrap().history().len() >= 3);
    }

    #[test]
    fn interval_change_applies_on_next_tick() {
        let interval = IntervalHandle::new(ms(20));
        let (tx, rx) = mpsc::channel();
        let sampler = Sampler::spawn_ticker(
            interval.clone(),
            |_, _| output(0),
            move |_| {
                let _ = tx.send(Instant::now());
            },
        );
        for _ in 0..3 {
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        let changed_at = Instant::now();
        interval.set(ms(80));
        assert_eq!(interval.get(), ms(80));
        let mut ticks = Vec::new();
        while ticks.len() < 5 {
            ticks.push(rx.recv_timeout(Duration::from_secs(2)).unwrap());
        }
        sampler.stop();
        // A tick that was already running at the change may still be reported
        // (at the old pace, on its own); from the first tick after the change
        // on, ticks follow the new interval. The schedule is anchored, so a
        // tick reported late (a busy CI runner) is followed by a shorter gap:
        // check the mean spacing, which timers never make shorter than the
        // interval (70 ms leaves 10 ms of slack for clock granularity).
        let first_after = ticks.iter().position(|t| *t > changed_at).unwrap();
        let after = &ticks[first_after..];
        assert!(after.len() >= 3, "too few ticks after the change");
        let gaps = u32::try_from(after.len() - 1).unwrap();
        let mean = (after[after.len() - 1] - after[0]) / gaps;
        assert!(
            mean >= ms(70),
            "ticks only {mean:?} apart on average after the change"
        );
        for pair in after.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(gap < ms(1_500), "the new pace never took hold: {gap:?}");
        }
    }

    #[test]
    fn a_shorter_interval_wakes_the_sleeping_sampler() {
        let interval = IntervalHandle::new(Duration::from_secs(60));
        let (tx, rx) = mpsc::channel();
        let sampler = Sampler::spawn_ticker(
            interval.clone(),
            |_, _| output(0),
            move |_| {
                let _ = tx.send(Instant::now());
            },
        );
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        // The next tick is a minute away; the new interval is already due.
        interval.set(ms(20));
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the sampler did not wake up for the new interval");
        let started = Instant::now();
        sampler.stop();
        assert!(started.elapsed() < ms(500));
    }

    #[test]
    fn the_handle_reports_what_was_set() {
        let interval = IntervalHandle::new(ms(1_000));
        let other = interval.clone();
        assert_eq!(other.get(), ms(1_000));
        interval.set(ms(2_500));
        assert_eq!(other.get(), ms(2_500));
    }
}
