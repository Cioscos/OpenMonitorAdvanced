use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::ThreadId;
use std::time::Duration;

use oma_ipc::Message;

use super::super::pipe::{CloseReason, ConnectError, PipeClient, PipeEvent, PipeReader};
use super::Input;

/// An open connection to the sensor pipe.
pub trait Connection: Send {
    /// PID of the process serving the pipe, read on this connection.
    fn server_pid(&self) -> Option<u32>;
    fn send(&mut self, msg: &Message) -> std::io::Result<()>;
}

/// How often a reader waiting for room in the link's queue looks again.
const FULL_QUEUE_POLL: Duration = Duration::from_millis(2);

/// Where a connection delivers what it receives: the link thread's queue,
/// tagged with the connection's id so the thread can discard what arrives
/// late from a connection it has already dropped.
///
/// The queue is bounded ([`LINK_QUEUE_CAPACITY`](super::LINK_QUEUE_CAPACITY)),
/// so a link thread stuck in a long call (a pipe write, the SCM) cannot make
/// it grow without limit. When it is full:
/// - a `Snapshot` is dropped (the next interval brings a new one); the
///   drops are counted and logged once per episode;
/// - any other message and the close wait for room, on the reader's own
///   thread. The wait ends when the link drops this connection or goes away,
///   so the link thread may join the reader while the queue is full. On the
///   link thread itself (a connector that reports at once) nothing waits:
///   what does not fit is dropped and logged.
#[derive(Clone)]
pub struct LinkSink {
    pub(super) id: u64,
    pub(super) tx: SyncSender<Input>,
    pub(super) shared: Arc<SinkShared>,
}

/// State of a connection's sink, shared with the link thread.
pub(super) struct SinkShared {
    /// The thread that drains the queue: it must never wait for room.
    link_thread: ThreadId,
    /// Set by the link before it drops the connection: a reader waiting for
    /// room gives up, so joining the reader cannot deadlock.
    cancelled: AtomicBool,
    /// Inside an episode of dropped snapshots.
    dropping: AtomicBool,
    /// Snapshots dropped on this connection.
    dropped: AtomicU64,
}

impl SinkShared {
    /// For a sink whose queue the current thread drains.
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            link_thread: std::thread::current().id(),
            cancelled: AtomicBool::new(false),
            dropping: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
        })
    }

    /// The link is dropping the connection: stop waiting for room.
    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl LinkSink {
    #[cfg(test)]
    pub(super) fn new(id: u64, tx: SyncSender<Input>) -> Self {
        Self {
            id,
            tx,
            shared: SinkShared::new(),
        }
    }

    /// Hands over a message; `false` when the link is gone (or is dropping
    /// this connection). A snapshot that finds the queue full is dropped
    /// and still counts as delivered.
    pub fn message(&self, msg: Message) -> bool {
        if matches!(msg, Message::Snapshot(_)) {
            return self.offer_snapshot(msg);
        }
        self.deliver(Input::Message(self.id, msg))
    }

    /// Reports that the connection ended. Sent at most once per connection.
    pub fn closed(&self, reason: CloseReason) {
        let _ = self.deliver(Input::Closed(self.id, reason));
    }

    /// Snapshots dropped so far on this connection.
    #[cfg(test)]
    pub(super) fn dropped_snapshots(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    fn offer_snapshot(&self, msg: Message) -> bool {
        let shared = &self.shared;
        match self.tx.try_send(Input::Message(self.id, msg)) {
            Ok(()) => {
                if shared.dropping.swap(false, Ordering::Relaxed) {
                    let total = shared.dropped.load(Ordering::Relaxed);
                    tracing::info!(
                        "sensor service link caught up; {total} snapshots dropped on this connection"
                    );
                }
                true
            }
            Err(TrySendError::Full(_)) => {
                shared.dropped.fetch_add(1, Ordering::Relaxed);
                if !shared.dropping.swap(true, Ordering::Relaxed) {
                    tracing::warn!("sensor service link is busy; dropping snapshots");
                }
                true
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// Puts `input` on the queue, waiting for room unless on the link thread.
    fn deliver(&self, mut input: Input) -> bool {
        let on_link_thread = std::thread::current().id() == self.shared.link_thread;
        loop {
            match self.tx.try_send(input) {
                Ok(()) => return true,
                Err(TrySendError::Disconnected(_)) => return false,
                Err(TrySendError::Full(_)) if on_link_thread => {
                    // Waiting here would wait for this very thread.
                    tracing::warn!("sensor service link queue full; event dropped");
                    return false;
                }
                Err(TrySendError::Full(back)) => {
                    if self.shared.cancelled.load(Ordering::Acquire) {
                        return false;
                    }
                    input = back;
                    std::thread::sleep(FULL_QUEUE_POLL);
                }
            }
        }
    }
}

/// Opens a [`Connection`] to the named pipe; its reader delivers to `sink`.
pub type Connector =
    Arc<dyn Fn(&str, LinkSink) -> Result<Box<dyn Connection>, ConnectError> + Send + Sync>;

/// The real connector: a [`PipeClient`] whose reader thread forwards
/// messages and the close to the link's queue. That thread is the one that
/// waits for room (see [`LinkSink`]); the link cancels the wait before it
/// drops the connection and so joins the reader.
pub fn pipe_connector() -> Connector {
    Arc::new(|pipe_name: &str, sink: LinkSink| {
        let client = PipeClient::connect(pipe_name)?;
        let reader = client.start_reader_with(move |event| match event {
            PipeEvent::Message(msg) => sink.message(msg),
            PipeEvent::Closed(reason) => {
                sink.closed(reason);
                true
            }
        });
        Ok(Box::new(PipeConnection {
            reader: Some(reader),
            client,
        }) as Box<dyn Connection>)
    })
}

/// [`PipeClient`] and its reader as a [`Connection`]. Dropping it stops the
/// reader and then releases the client: the kernel handle closes only when
/// both are gone.
struct PipeConnection {
    reader: Option<PipeReader>,
    client: PipeClient,
}

impl Connection for PipeConnection {
    fn server_pid(&self) -> Option<u32> {
        self.client.server_pid()
    }

    fn send(&mut self, msg: &Message) -> std::io::Result<()> {
        self.client.send(msg)
    }
}

impl Drop for PipeConnection {
    fn drop(&mut self) {
        if let Some(reader) = self.reader.take() {
            reader.stop();
        }
    }
}
