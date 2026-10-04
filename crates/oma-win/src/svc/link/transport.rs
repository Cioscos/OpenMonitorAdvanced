use std::sync::mpsc::Sender;
use std::sync::Arc;

use oma_ipc::Message;

use super::super::pipe::{CloseReason, ConnectError, PipeClient, PipeEvent, PipeReader};
use super::Input;

/// An open connection to the sensor pipe.
pub trait Connection: Send {
    /// PID of the process serving the pipe, read on this connection.
    fn server_pid(&self) -> Option<u32>;
    fn send(&mut self, msg: &Message) -> std::io::Result<()>;
}

/// Where a connection delivers what it receives: the link thread's queue,
/// tagged with the connection's id so the thread can discard what arrives
/// late from a connection it has already dropped.
///
/// Delivery never blocks. The queue is not bounded: the thread drains it
/// between short effects, and the service sends one snapshot per interval.
#[derive(Clone)]
pub struct LinkSink {
    pub(super) id: u64,
    pub(super) tx: Sender<Input>,
}

impl LinkSink {
    #[cfg(test)]
    pub(super) fn new(id: u64, tx: Sender<Input>) -> Self {
        Self { id, tx }
    }

    /// Hands over a message; `false` when the link is gone.
    pub fn message(&self, msg: Message) -> bool {
        self.tx.send(Input::Message(self.id, msg)).is_ok()
    }

    /// Reports that the connection ended. Sent at most once per connection.
    pub fn closed(&self, reason: CloseReason) {
        let _ = self.tx.send(Input::Closed(self.id, reason));
    }
}

/// Opens a [`Connection`] to the named pipe; its reader delivers to `sink`.
pub type Connector =
    Arc<dyn Fn(&str, LinkSink) -> Result<Box<dyn Connection>, ConnectError> + Send + Sync>;

/// The real connector: a [`PipeClient`] whose reader thread forwards
/// messages and the close to the link's queue.
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
