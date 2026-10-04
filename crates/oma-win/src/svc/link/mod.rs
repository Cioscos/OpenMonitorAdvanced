//! The app's link to the sensor service: one thread that starts the service
//! once at launch, connects to its pipe, checks who serves it, subscribes,
//! and hands schema and snapshots to the [`SvcFeed`]; or, in anti-cheat
//! compatible mode, stops the service and keeps away from it (spec §2.2, §6).
//!
//! The rules live in [`Machine::decide`], a pure function of the current
//! state, the event and the time, which returns the effects to run (SCM
//! calls, connection, feed updates). The thread only waits for events and
//! runs effects, so the rules are tested without threads or clocks.
//!
//! The thread blocks on one queue until the next event: commands from the
//! shell and, for the open connection, the messages and the close that its
//! reader forwards through a [`LinkSink`] all arrive on it. Between events it
//! sleeps until the machine's deadline (or for good when there is none), so a
//! connected link costs no timer wake-ups.
//!
//! The queue holds at most [`LINK_QUEUE_CAPACITY`] inputs, so a thread stuck
//! in a long call cannot make it grow without limit. With the queue full, a
//! command fails at once with [`LinkBusy`] (the shell's main thread and its
//! store listeners never wait for the link), a snapshot is dropped, and the
//! connection's reader, a thread of its own, waits for room for anything
//! else (see [`LinkSink`]).
//!
//! Limits, by design:
//! - The anti-cheat preference is per user and only restrains this app: it
//!   stops the service once, verifies the stop, and then leaves the service
//!   alone. If another user, an administrator or another client starts it
//!   again, the app does not fight back with a loop of STOPs (on a PC with
//!   several users logged on, one can already stop the service for the
//!   others: the accepted multi-user limit of spec §2.2).
//! - After `Incompatible` or `PidMismatch` the link does not reconnect on its
//!   own: every connection would reset the service's idle timer, so a service
//!   that cannot be served would never idle out. It keeps asking the SCM at
//!   each `retry` and connects again only on a `Start` command or when the SCM
//!   shows the service running as something new (a restart, a new process).
//! - The reconnect loop never starts the service: only the launch probe, a
//!   `Start` command and leaving anti-cheat mode call `start()`. The launch
//!   probe starts the service only when its first conclusive answer is
//!   `Stopped` (ruling R21): a service found missing, refused, on its way
//!   down or in any other state at launch may be in the hands of an
//!   installer or an administrator, and a later `Stopped` is only reported.
//! - [`ServiceLink::shutdown`] interrupts every wait of the thread (Hello,
//!   first sample, stream, retry, STOP verification) at once, but not a call
//!   already running: a pipe write can block for up to 2 s (the client's
//!   write timeout) and an SCM call for as long as the SCM takes. `shutdown`
//!   waits for the thread at most [`JOIN_WAIT`]; past that it detaches it,
//!   and the thread exits by itself as soon as the call returns, without
//!   touching the connection again.

use std::collections::VecDeque;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oma_ipc::Message;

use super::feed::SvcFeed;
use super::pipe::CloseReason;
use super::scm::ServiceControl;
use super::status::ServiceStatusTable;

mod machine;
mod transport;

#[cfg(test)]
mod tests;

pub use machine::{validate_schema, LinkCommand, LinkSettings};
use machine::{Effect, Event, Machine};
use transport::SinkShared;
pub use transport::{pipe_connector, Connection, Connector, LinkSink};

/// How many inputs the link thread's queue holds: commands, shutdown, and
/// what the connection's reader forwards.
pub const LINK_QUEUE_CAPACITY: usize = 256;

/// [`ServiceLink::send`] found the link's queue full: the command was not
/// delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkBusy;

impl std::fmt::Display for LinkBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the sensor service link is busy")
    }
}

impl std::error::Error for LinkBusy {}

/// Longest [`ServiceLink::shutdown`] waits for the thread before detaching it.
pub const JOIN_WAIT: Duration = Duration::from_millis(500);

/// What reaches the link thread on its one queue.
enum Input {
    Command(LinkCommand),
    Shutdown,
    /// A message of connection `.0`.
    Message(u64, Message),
    /// Connection `.0` ended.
    Closed(u64, CloseReason),
}

/// Ids of connections, unique across links, so that an event tagged with
/// the id of a dropped connection can be told from the current one's.
static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

/// The link thread: waits for events and runs the machine's effects.
struct Driver {
    machine: Machine,
    control: Arc<dyn ServiceControl>,
    connector: Connector,
    status: ServiceStatusTable,
    feed: SvcFeed,
    /// The one queue: commands, shutdown, and what the connection's reader
    /// forwards.
    inbox: Receiver<Input>,
    /// A sender on `inbox`, for the sinks of new connections.
    sender: SyncSender<Input>,
    stop: Arc<AtomicBool>,
    conn: Option<Box<dyn Connection>>,
    /// The sink state of the current connection, cancelled before the
    /// connection is dropped.
    conn_sink: Option<Arc<SinkShared>>,
    /// Id of the current connection (the one `conn` holds, or that is being
    /// opened): events tagged with another id are discarded.
    conn_id: Option<u64>,
    /// Events of the current connection that arrived while the machine was
    /// not reading it (for instance the server's `Hello` during the PID
    /// check), in order.
    unread: VecDeque<Event>,
    /// Results of effects, handled before any new wait.
    pending: VecDeque<Event>,
    /// How many times the thread went to sleep on `inbox`.
    #[cfg(test)]
    waits: Arc<AtomicUsize>,
}

impl Driver {
    fn run(mut self) {
        let effects = self.machine.launch(Instant::now());
        self.apply(effects);
        while !self.stopping() {
            let event = match self.pending.pop_front() {
                Some(event) => event,
                None => match self.next_event() {
                    Some(event) => event,
                    None => break,
                },
            };
            let effects = self.machine.decide(event, Instant::now());
            self.apply(effects);
        }
        // Stops the pipe reader and closes the handle.
        self.drop_connection();
    }

    /// Drops the connection, if any, first releasing a reader that waits
    /// for room in the queue: dropping joins the reader, and the queue
    /// would never drain while this thread waits for it.
    fn drop_connection(&mut self) {
        if let Some(shared) = self.conn_sink.take() {
            shared.cancel();
        }
        self.conn = None;
    }

    fn stopping(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    /// The next command, message, close or deadline; `None` on shutdown.
    /// Sleeps on the queue until the machine's deadline, or until something
    /// arrives when there is no deadline.
    fn next_event(&mut self) -> Option<Event> {
        loop {
            if self.stopping() {
                return None;
            }
            if self.machine.reads_connection() {
                if let Some(event) = self.unread.pop_front() {
                    return Some(event);
                }
            }
            // What is already queued comes before a deadline that falls due
            // at the same time: a command is not delayed by a retry.
            let input = match self.inbox.try_recv() {
                Ok(input) => input,
                Err(TryRecvError::Disconnected) => return None,
                Err(TryRecvError::Empty) => {
                    let left = match self.machine.deadline {
                        Some(deadline) => {
                            let now = Instant::now();
                            if now >= deadline {
                                return Some(Event::Timer);
                            }
                            Some(deadline - now)
                        }
                        None => None,
                    };
                    #[cfg(test)]
                    self.waits.fetch_add(1, Ordering::Relaxed);
                    match left {
                        Some(left) => match self.inbox.recv_timeout(left) {
                            Ok(input) => input,
                            Err(RecvTimeoutError::Timeout) => continue,
                            Err(RecvTimeoutError::Disconnected) => return None,
                        },
                        None => self.inbox.recv().ok()?,
                    }
                }
            };
            match input {
                Input::Command(command) => return Some(Event::Command(command)),
                Input::Shutdown => return None,
                Input::Message(id, msg) if self.conn_id == Some(id) => {
                    self.unread.push_back(Event::Message(msg));
                }
                Input::Closed(id, reason) if self.conn_id == Some(id) => {
                    self.unread.push_back(Event::Closed(reason));
                }
                // A late event of a connection that is already gone.
                Input::Message(..) | Input::Closed(..) => {}
            }
        }
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Query => self.pending.push_back(Event::Queried(self.control.query())),
                Effect::Start => self.pending.push_back(Event::Started(self.control.start())),
                Effect::Stop => self.pending.push_back(Event::StopSent(self.control.stop())),
                Effect::Connect => {
                    let id = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
                    self.conn_id = Some(id);
                    self.unread.clear();
                    // A previous connection's reader stops waiting for room.
                    if let Some(old) = self.conn_sink.take() {
                        old.cancel();
                    }
                    let shared = SinkShared::new();
                    let sink = LinkSink {
                        id,
                        tx: self.sender.clone(),
                        shared: Arc::clone(&shared),
                    };
                    let result = match (self.connector)(&self.machine.settings.pipe_name, sink) {
                        Ok(conn) => {
                            let pid = conn.server_pid();
                            self.conn = Some(conn);
                            self.conn_sink = Some(shared);
                            Ok(pid)
                        }
                        Err(e) => {
                            shared.cancel();
                            self.conn_id = None;
                            Err(e)
                        }
                    };
                    self.pending.push_back(Event::Connected(result));
                }
                Effect::Send(msg) => {
                    // A write can block for seconds: never start one after shutdown.
                    let ok = !self.stopping()
                        && match self.conn.as_mut() {
                            Some(conn) => match conn.send(&msg) {
                                Ok(()) => true,
                                Err(e) => {
                                    tracing::info!("cannot write to the sensor pipe: {e}");
                                    false
                                }
                            },
                            None => false,
                        };
                    self.pending.push_back(Event::Sent(ok));
                }
                Effect::Close => {
                    self.drop_connection();
                    self.conn_id = None;
                    self.unread.clear();
                }
                Effect::ClearFeed => self.feed.clear(),
                Effect::SetInterval(interval) => self.feed.set_interval(interval),
                Effect::SetRequest(request) => self.feed.set_request(request),
                Effect::SetSchema(schema) => self.feed.set_schema(schema),
                Effect::SetSnapshot(snapshot) => self.feed.set_snapshot(snapshot, Instant::now()),
            }
        }
        // Before the status, so a reader woken by the status sees the version.
        self.status
            .set_service_version(self.machine.service_version.as_deref());
        if self.status.set(&self.machine.status) {
            tracing::info!("sensor service status: {:?}", self.machine.status);
        }
    }
}

/// The running link: a command channel and the thread.
pub struct ServiceLink {
    commands: SyncSender<Input>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    #[cfg(test)]
    waits: Arc<AtomicUsize>,
}

impl ServiceLink {
    /// Sets the initial status (`AntiCheat` + `Stopping`, or `Starting`) and
    /// starts the thread.
    pub fn spawn(
        control: Arc<dyn ServiceControl>,
        connector: Connector,
        settings: LinkSettings,
        anti_cheat: bool,
        status: ServiceStatusTable,
        feed: SvcFeed,
    ) -> Self {
        feed.set_request(settings.sources.clone());
        let machine = Machine::new(settings, anti_cheat);
        status.set(&machine.status);
        let (commands, inbox) = mpsc::sync_channel(LINK_QUEUE_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let waits = Arc::new(AtomicUsize::new(0));
        let driver = Driver {
            machine,
            control,
            connector,
            status,
            feed,
            inbox,
            sender: commands.clone(),
            stop: Arc::clone(&stop),
            conn: None,
            conn_sink: None,
            conn_id: None,
            unread: VecDeque::new(),
            pending: VecDeque::new(),
            #[cfg(test)]
            waits: Arc::clone(&waits),
        };
        let thread = std::thread::Builder::new()
            .name("oma-service-link".to_owned())
            .spawn(move || driver.run());
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(e) => {
                tracing::error!("cannot start the sensor service link: {e}");
                None
            }
        };
        Self {
            commands,
            stop,
            thread,
            #[cfg(test)]
            waits,
        }
    }

    /// Queues `command` for the thread. Never blocks: with the queue full
    /// it fails at once with [`LinkBusy`]. A link whose thread is gone takes
    /// the command and drops it, as before.
    pub fn send(&self, command: LinkCommand) -> Result<(), LinkBusy> {
        match self.commands.try_send(Input::Command(command)) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => Ok(()),
            Err(TrySendError::Full(_)) => Err(LinkBusy),
        }
    }

    /// How many times the thread has gone to sleep waiting for an event.
    #[cfg(test)]
    fn waits(&self) -> usize {
        self.waits.load(Ordering::Relaxed)
    }

    /// Tells the thread to stop. The thread holds a sender of its own queue,
    /// so it would not notice the link being dropped otherwise. With the
    /// queue full the wake-up is not queued, and not needed: the thread does
    /// not sleep on a full queue, and it checks `stop` before each input.
    fn signal_stop(&self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.commands.try_send(Input::Shutdown);
    }

    /// Stops the thread, closing any connection. Returns within
    /// [`JOIN_WAIT`] (plus a few milliseconds): a thread still inside a
    /// blocking call by then is detached (see the module notes).
    pub fn shutdown(mut self) {
        self.signal_stop();
        let Some(thread) = self.thread.take() else {
            return;
        };
        let end = Instant::now() + JOIN_WAIT;
        while !thread.is_finished() {
            if Instant::now() >= end {
                tracing::warn!("sensor service link still busy at shutdown; detaching it");
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = thread.join();
    }
}

impl Drop for ServiceLink {
    /// A link dropped without [`shutdown`](Self::shutdown) still stops its
    /// thread (which is then left to exit by itself).
    fn drop(&mut self) {
        self.signal_stop();
    }
}
