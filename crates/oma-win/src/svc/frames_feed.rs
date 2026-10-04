//! Frame data received from the sensor service (protocol v4), written by the
//! link thread and drained by the app's frame consumer a few times a second.
//!
//! The state (the engine's status and the list of presenting processes) is
//! the latest received and stays until the next one or a disconnection; the
//! frame batches are a queue that [`FramesFeed::drain`] empties. The queue
//! holds at most [`MAX_QUEUED_BATCHES`] (6.4 s at the service's 100 ms
//! flush): a consumer that stops draining costs no more than that, and the
//! oldest batches go first, their frames counted in the `dropped` of the
//! oldest batch kept.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use oma_ipc::{FrameBatch, FramesStatus, PresentingProcesses};

/// Most batches the feed keeps between two drains.
pub const MAX_QUEUED_BATCHES: usize = 64;

/// What the link hands to the feed.
#[derive(Debug, Clone, PartialEq)]
pub enum FramesEvent {
    Status(FramesStatus),
    Processes(PresentingProcesses),
    Batch(FrameBatch),
    /// The link lost the service: everything received so far is stale.
    Disconnected,
}

/// What a drain returns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FramesUpdate {
    /// The engine's latest status, kept until the next one or a disconnection.
    pub status: Option<FramesStatus>,
    /// The latest list of presenting processes, kept like `status`.
    pub processes: Option<PresentingProcesses>,
    /// The batches received since the previous drain, oldest first.
    pub batches: Vec<FrameBatch>,
    /// The service has sent frame data (any of the above) since the link
    /// last lost it.
    pub connected: bool,
}

#[derive(Default)]
struct Inner {
    status: Option<FramesStatus>,
    processes: Option<PresentingProcesses>,
    batches: VecDeque<FrameBatch>,
    connected: bool,
}

/// Frame data from the service; cheap to clone (shared state).
#[derive(Clone, Default)]
pub struct FramesFeed(Arc<Mutex<Inner>>);

impl FramesFeed {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes one event from the link.
    pub fn apply(&self, event: FramesEvent) {
        let mut inner = self.lock();
        match event {
            FramesEvent::Status(status) => inner.status = Some(status),
            FramesEvent::Processes(list) => inner.processes = Some(list),
            FramesEvent::Batch(batch) => {
                inner.batches.push_back(batch);
                if inner.batches.len() > MAX_QUEUED_BATCHES {
                    if let Some(old) = inner.batches.pop_front() {
                        let lost = u32::try_from(old.frames.len())
                            .unwrap_or(u32::MAX)
                            .saturating_add(old.dropped);
                        if let Some(next) = inner.batches.front_mut() {
                            next.dropped = next.dropped.saturating_add(lost);
                        }
                    }
                }
            }
            FramesEvent::Disconnected => {
                *inner = Inner::default();
                return;
            }
        }
        inner.connected = true;
    }

    /// The current state and the batches queued since the last drain (which
    /// leaves the queue empty).
    pub fn drain(&self) -> FramesUpdate {
        let mut inner = self.lock();
        FramesUpdate {
            status: inner.status.clone(),
            processes: inner.processes.clone(),
            batches: inner.batches.drain(..).collect(),
            connected: inner.connected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(pid: u32, frames: usize, dropped: u32) -> FrameBatch {
        let frame = oma_ipc::WireFrame {
            qpc: 1,
            swapchain: 2,
            frame_type: "app".to_owned(),
            displayed: true,
            ms_between_presents: 4.0,
            ms_between_display_change: Some(4.0),
            ms_until_displayed: Some(1.0),
            ms_app_frametime: Some(4.0),
            ms_pc_latency: None,
            ms_gpu_busy: None,
            pcl_frame_id: None,
        };
        FrameBatch {
            pid,
            frames: vec![frame; frames],
            dropped,
        }
    }

    fn running() -> FramesStatus {
        FramesStatus {
            state: oma_ipc::frames_state::RUNNING.to_owned(),
            detail: None,
            presentmon_version: Some("2.6.0".to_owned()),
        }
    }

    #[test]
    fn drain_returns_and_clears_batches() {
        let feed = FramesFeed::default();
        assert_eq!(feed.drain(), FramesUpdate::default());

        let processes = PresentingProcesses {
            at_qpc: 5,
            processes: Vec::new(),
        };
        feed.apply(FramesEvent::Status(running()));
        feed.apply(FramesEvent::Processes(processes.clone()));
        feed.apply(FramesEvent::Batch(batch(7, 2, 0)));
        feed.apply(FramesEvent::Batch(batch(7, 1, 1)));

        let update = feed.clone().drain();
        assert_eq!(update.batches, vec![batch(7, 2, 0), batch(7, 1, 1)]);
        assert_eq!(update.status, Some(running()));
        assert_eq!(update.processes, Some(processes.clone()));
        assert!(update.connected);

        // The batches are gone, the state stays.
        let again = feed.drain();
        assert!(again.batches.is_empty());
        assert_eq!(again.status, Some(running()));
        assert_eq!(again.processes, Some(processes));
        assert!(again.connected);
    }

    #[test]
    fn feed_keeps_at_most_sixty_four_batches() {
        let feed = FramesFeed::default();
        for pid in 0..70 {
            feed.apply(FramesEvent::Batch(batch(pid, 3, 1)));
        }
        let batches = feed.drain().batches;
        assert_eq!(batches.len(), MAX_QUEUED_BATCHES);
        assert_eq!(MAX_QUEUED_BATCHES, 64);
        // The six oldest went; their 6 × (3 + 1) frames count as dropped.
        assert_eq!(batches[0].pid, 6);
        assert_eq!(batches[0].dropped, 1 + 6 * 4);
        assert_eq!(batches[63].pid, 69);
        assert_eq!(batches[63].dropped, 1);
    }

    #[test]
    fn disconnect_clears_status() {
        let feed = FramesFeed::default();
        feed.apply(FramesEvent::Status(running()));
        feed.apply(FramesEvent::Processes(PresentingProcesses {
            at_qpc: 5,
            processes: Vec::new(),
        }));
        feed.apply(FramesEvent::Batch(batch(7, 2, 0)));
        feed.apply(FramesEvent::Disconnected);
        assert_eq!(feed.drain(), FramesUpdate::default());

        // A new status after the reconnection reads as connected again.
        feed.apply(FramesEvent::Status(running()));
        let update = feed.drain();
        assert!(update.connected);
        assert_eq!(update.status, Some(running()));
    }
}
