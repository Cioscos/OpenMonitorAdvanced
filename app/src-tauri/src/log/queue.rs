//! Bounded queue between the sampler and the log writer, with control
//! barriers (L1) and conservative memory accounting (L2).
//!
//! Only rows count toward the limits and only rows can be refused: the
//! sampler pushes them with [`LogQueue::try_push_row`], which never waits.
//! Control messages always enter (while the queue is open) and stay ordered
//! after the rows already accepted, so a `Pause` or `Stop` is a barrier: the
//! writer handles every row accepted before it first.

use std::collections::{HashMap, VecDeque};
use std::mem::size_of;
use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{Duration, Instant};

use oma_core::csv::Layout;

use super::writer::WriteFailure;

/// Rows the queue holds at most.
pub const MAX_ROWS: usize = 64;
/// Accounted bytes (rows and the layouts they retain) the queue holds at most.
pub const MAX_QUEUE_BYTES: usize = 4 * 1024 * 1024;

/// One sampled row of a log session.
#[derive(Debug)]
pub struct Row {
    pub session: u64,
    pub layout: Arc<Layout>,
    pub timestamp_ms: u64,
    pub offset_minutes: i32,
    /// Raw values in column order ([`Layout::extract`]).
    pub values: Box<[Option<f64>]>,
}

/// Commands to the writer; each one is answered once on its `reply`.
#[derive(Debug)]
pub enum Control {
    /// Opens a session: folder, first file part, BOM and header.
    Start {
        session: u64,
        layout: Arc<Layout>,
        dir: PathBuf,
        /// File name stem, e.g. `oma-2026-09-29_14-03-12`.
        stem: String,
        limit_bytes: u64,
        reply: Reply,
    },
    /// Writes and flushes what the session has buffered.
    Pause {
        session: u64,
        reply: Reply,
    },
    Resume {
        session: u64,
        reply: Reply,
    },
    /// Writes, flushes and closes the session's file.
    Stop {
        session: u64,
        reply: Reply,
    },
}

/// Answer channel of a [`Control`] (capacity 1). The writer never waits on
/// it: a receiver that gave up is not an error.
pub type Reply = SyncSender<Result<(), WriteFailure>>;

/// The queue was closed: the control was not accepted and gets no reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueClosed;

#[derive(Debug)]
pub(crate) enum Item {
    Row(Row),
    Control(Control),
}

/// What [`LogQueue::pop`] got.
#[derive(Debug)]
pub(crate) enum Popped {
    Item(Item),
    TimedOut,
    /// Closed and fully drained: the writer exits.
    Closed,
}

pub struct LogQueue {
    state: Mutex<State>,
    ready: Condvar,
    max_rows: usize,
    max_bytes: usize,
}

#[derive(Default)]
struct State {
    items: VecDeque<Item>,
    rows: usize,
    /// Accounted bytes of the queued rows and of every layout they (or a
    /// queued `Start`) retain.
    bytes: usize,
    /// Retained layouts by `Arc` identity: each is counted once, until the
    /// last queued item holding it leaves.
    layouts: HashMap<usize, Retained>,
    closed: bool,
    /// The writer waits in `pop` (tests: a tick then cannot meet it on the lock).
    #[cfg(test)]
    parked: bool,
}

struct Retained {
    refs: usize,
    bytes: usize,
}

impl LogQueue {
    pub fn new(max_rows: usize, max_bytes: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            ready: Condvar::new(),
            max_rows,
            max_bytes,
        })
    }

    /// Never waits: lock contention, a full queue or a closed one drop the
    /// row (it is handed back).
    pub fn try_push_row(&self, row: Row) -> Result<(), Row> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(row),
        };
        if state.closed || state.rows >= self.max_rows {
            return Err(row);
        }
        let cost = row_cost(&row);
        let needed = cost.saturating_add(state.layout_cost(&row.layout));
        if state.bytes.saturating_add(needed) > self.max_bytes {
            return Err(row);
        }
        state.retain(&row.layout);
        state.bytes += cost;
        state.rows += 1;
        state.items.push_back(Item::Row(row));
        drop(state);
        self.ready.notify_one();
        Ok(())
    }

    /// Always accepted while the queue is open, ordered after the rows
    /// already accepted. A `Start` whose layout alone exceeds the byte budget
    /// is refused: it is answered at once with an error and never queued.
    pub fn push_control(&self, control: Control) -> Result<(), QueueClosed> {
        let mut state = self.lock();
        if state.closed {
            return Err(QueueClosed);
        }
        if let Control::Start { layout, .. } = &control {
            if layout.retained_bytes() > self.max_bytes {
                drop(state);
                if let Control::Start { reply, .. } = control {
                    // The coordinator may already have given up waiting.
                    let _ = reply.try_send(Err(WriteFailure::Other(format!(
                        "the column layout exceeds the {} byte log queue budget",
                        self.max_bytes
                    ))));
                }
                return Ok(());
            }
            state.retain(layout);
        }
        state.items.push_back(Item::Control(control));
        drop(state);
        self.ready.notify_one();
        Ok(())
    }

    /// Refuses everything from now on; the writer exits after draining.
    pub fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all();
    }

    /// Next item, waiting at most `timeout` (`None`: until one arrives or
    /// the queue is closed).
    pub(crate) fn pop(&self, timeout: Option<Duration>) -> Popped {
        // A timeout too large for `Instant` waits without one.
        let deadline = timeout.and_then(|timeout| Instant::now().checked_add(timeout));
        let mut state = self.lock();
        loop {
            if let Some(item) = state.take() {
                return Popped::Item(item);
            }
            if state.closed {
                return Popped::Closed;
            }
            #[cfg(test)]
            {
                state.parked = true;
            }
            state = match deadline {
                None => self
                    .ready
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner),
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Popped::TimedOut;
                    }
                    self.ready
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
            };
            #[cfg(test)]
            {
                state.parked = false;
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Items (rows and controls) waiting for the writer.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().items.len()
    }

    /// The writer waits for items and none is queued.
    #[cfg(test)]
    pub(crate) fn writer_parked(&self) -> bool {
        let state = self.lock();
        state.parked && state.items.is_empty()
    }
}

impl State {
    /// Bytes `layout` adds when it is not retained yet.
    fn layout_cost(&self, layout: &Arc<Layout>) -> usize {
        if self.layouts.contains_key(&identity(layout)) {
            0
        } else {
            layout.retained_bytes()
        }
    }

    fn retain(&mut self, layout: &Arc<Layout>) {
        let retained = self
            .layouts
            .entry(identity(layout))
            .or_insert_with(|| Retained {
                refs: 0,
                bytes: layout.retained_bytes(),
            });
        if retained.refs == 0 {
            self.bytes += retained.bytes;
        }
        retained.refs += 1;
    }

    fn release(&mut self, layout: &Arc<Layout>) {
        let key = identity(layout);
        if let Some(retained) = self.layouts.get_mut(&key) {
            retained.refs -= 1;
            if retained.refs == 0 {
                self.bytes -= retained.bytes;
                self.layouts.remove(&key);
            }
        }
    }

    fn take(&mut self) -> Option<Item> {
        let item = self.items.pop_front()?;
        match &item {
            Item::Row(row) => {
                self.rows -= 1;
                self.bytes -= row_cost(row);
                self.release(&row.layout);
            }
            Item::Control(Control::Start { layout, .. }) => self.release(layout),
            Item::Control(_) => {}
        }
        Some(item)
    }
}

/// A layout's identity while an `Arc` to it is queued.
fn identity(layout: &Arc<Layout>) -> usize {
    Arc::as_ptr(layout).addr()
}

/// Conservative accounting of one queued row (L2): the row, its deque slot
/// and the backing store of its values.
fn row_cost(row: &Row) -> usize {
    size_of::<Row>() + size_of::<Item>() + size_of::<Option<f64>>() * row.values.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::csv::{Column, Conversion};
    use std::sync::mpsc::{sync_channel, Receiver};

    fn layout(label: &str) -> Arc<Layout> {
        Arc::new(Layout {
            columns: vec![Column {
                sensor_id: "cpu/load/total".into(),
                index: 0,
                device: "CPU".into(),
                label: label.into(),
                unit: "%",
                conversion: Conversion::None,
            }],
        })
    }

    fn row(layout: &Arc<Layout>, values: usize) -> Row {
        Row {
            session: 1,
            layout: layout.clone(),
            timestamp_ms: 0,
            offset_minutes: 0,
            values: vec![Some(1.0); values].into_boxed_slice(),
        }
    }

    fn reply() -> (Reply, Receiver<Result<(), WriteFailure>>) {
        sync_channel(1)
    }

    fn pause(session: u64) -> Control {
        Control::Pause {
            session,
            reply: reply().0,
        }
    }

    fn start(layout: &Arc<Layout>) -> (Control, Receiver<Result<(), WriteFailure>>) {
        let (tx, rx) = reply();
        let control = Control::Start {
            session: 1,
            layout: layout.clone(),
            dir: PathBuf::from("logs"),
            stem: "oma-2026-09-29_14-03-12".into(),
            limit_bytes: 1 << 20,
            reply: tx,
        };
        (control, rx)
    }

    fn pop_now(queue: &LogQueue) -> Popped {
        queue.pop(Some(Duration::ZERO))
    }

    fn accounted(queue: &LogQueue) -> usize {
        queue.lock().bytes
    }

    #[test]
    fn queue_drops_rows_beyond_the_limits_but_never_controls() {
        let l = layout("Load");
        // Row limit.
        let queue = LogQueue::new(2, MAX_QUEUE_BYTES);
        assert!(queue.try_push_row(row(&l, 1)).is_ok());
        assert!(queue.try_push_row(row(&l, 1)).is_ok());
        assert!(queue.try_push_row(row(&l, 1)).is_err());
        for session in 0..5 {
            assert_eq!(queue.push_control(pause(session)), Ok(()));
        }
        assert!(queue.try_push_row(row(&l, 1)).is_err());
        // Order: the two rows, then the five controls.
        for _ in 0..2 {
            assert!(matches!(pop_now(&queue), Popped::Item(Item::Row(_))));
        }
        for expected in 0..5 {
            match pop_now(&queue) {
                Popped::Item(Item::Control(Control::Pause { session, .. })) => {
                    assert_eq!(session, expected)
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(matches!(pop_now(&queue), Popped::TimedOut));

        // Byte limit: a row with many values does not fit, a small one does.
        let budget = l.retained_bytes() + row_cost(&row(&l, 4)) + row_cost(&row(&l, 4));
        let queue = LogQueue::new(MAX_ROWS, budget);
        assert!(queue.try_push_row(row(&l, 4)).is_ok());
        assert!(queue.try_push_row(row(&l, 100)).is_err());
        assert!(queue.try_push_row(row(&l, 4)).is_ok());
        assert!(queue.try_push_row(row(&l, 1)).is_err());
        assert_eq!(queue.push_control(pause(1)), Ok(()));
    }

    #[test]
    fn queue_counts_shared_layout_until_last_row_leaves() {
        let shared = layout("Load");
        let queue = LogQueue::new(MAX_ROWS, MAX_QUEUE_BYTES);
        let cost = row_cost(&row(&shared, 3));
        assert!(cost >= size_of::<Row>() + size_of::<Item>() + 3 * size_of::<Option<f64>>());

        queue.try_push_row(row(&shared, 3)).unwrap();
        assert_eq!(accounted(&queue), shared.retained_bytes() + cost);
        queue.try_push_row(row(&shared, 3)).unwrap();
        assert_eq!(accounted(&queue), shared.retained_bytes() + 2 * cost);
        // A queued Start retains the layout too, but counts it only once.
        let (control, _rx) = start(&shared);
        queue.push_control(control).unwrap();
        assert_eq!(accounted(&queue), shared.retained_bytes() + 2 * cost);
        // A different Arc is another retained layout.
        let other = layout("Load");
        queue.try_push_row(row(&other, 3)).unwrap();
        assert_eq!(
            accounted(&queue),
            shared.retained_bytes() + other.retained_bytes() + 3 * cost
        );

        assert!(matches!(pop_now(&queue), Popped::Item(Item::Row(_))));
        assert_eq!(
            accounted(&queue),
            shared.retained_bytes() + other.retained_bytes() + 2 * cost
        );
        assert!(matches!(pop_now(&queue), Popped::Item(Item::Row(_))));
        // The Start still holds the shared layout.
        assert_eq!(
            accounted(&queue),
            shared.retained_bytes() + other.retained_bytes() + cost
        );
        assert!(matches!(pop_now(&queue), Popped::Item(Item::Control(_))));
        assert_eq!(accounted(&queue), other.retained_bytes() + cost);
        assert!(matches!(pop_now(&queue), Popped::Item(Item::Row(_))));
        assert_eq!(accounted(&queue), 0);
        assert!(queue.lock().layouts.is_empty());
    }

    #[test]
    fn queue_rejects_oversized_layout() {
        // Long labels: the layout alone exceeds the budget.
        let huge = layout(&"é".repeat(4096));
        let budget = 4096;
        assert!(huge.retained_bytes() > budget);
        let queue = LogQueue::new(MAX_ROWS, budget);

        assert!(queue.try_push_row(row(&huge, 1)).is_err());
        let (control, rx) = start(&huge);
        assert_eq!(queue.push_control(control), Ok(()));
        // Answered at once, never queued.
        assert!(matches!(rx.try_recv(), Ok(Err(WriteFailure::Other(_)))));
        assert!(matches!(pop_now(&queue), Popped::TimedOut));
        assert_eq!(accounted(&queue), 0);

        // A small layout still fits.
        assert!(queue.try_push_row(row(&layout("Load"), 1)).is_ok());
    }

    #[test]
    fn queue_try_push_drops_on_lock_contention() {
        let l = layout("Load");
        let queue = LogQueue::new(MAX_ROWS, MAX_QUEUE_BYTES);
        let held = queue.lock();
        std::thread::scope(|scope| {
            let pushed = scope
                .spawn(|| queue.try_push_row(row(&l, 1)).is_ok())
                .join()
                .unwrap();
            assert!(!pushed, "a contended lock drops the row instead of waiting");
        });
        drop(held);
        assert!(queue.try_push_row(row(&l, 1)).is_ok());
    }

    #[test]
    fn closed_queue_rejects_controls() {
        let l = layout("Load");
        let queue = LogQueue::new(MAX_ROWS, MAX_QUEUE_BYTES);
        queue.try_push_row(row(&l, 1)).unwrap();
        queue.push_control(pause(7)).unwrap();
        queue.close();

        assert_eq!(queue.push_control(pause(8)), Err(QueueClosed));
        assert!(queue.try_push_row(row(&l, 1)).is_err());
        // What was accepted is still drained, then the writer is told to exit.
        assert!(matches!(queue.pop(None), Popped::Item(Item::Row(_))));
        assert!(matches!(
            queue.pop(None),
            Popped::Item(Item::Control(Control::Pause { session: 7, .. }))
        ));
        assert!(matches!(queue.pop(None), Popped::Closed));
    }

    #[test]
    fn close_wakes_a_waiting_writer() {
        let queue = LogQueue::new(MAX_ROWS, MAX_QUEUE_BYTES);
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| matches!(queue.pop(None), Popped::Closed));
            queue.close();
            assert!(waiter.join().unwrap());
        });
    }

    #[test]
    fn push_wakes_a_waiting_writer() {
        let queue = LogQueue::new(MAX_ROWS, MAX_QUEUE_BYTES);
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| matches!(queue.pop(None), Popped::Item(_)));
            queue.push_control(pause(1)).unwrap();
            assert!(waiter.join().unwrap());
        });
    }
}
