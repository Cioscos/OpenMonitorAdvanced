//! One persistent worker per provider. No timer and at most one in-flight request.
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::time::Instant;

use crate::engine::backoff_ms;
use crate::provider::{Inventory, Provider, ProviderError};

/// Consecutive `Rediscover` results retried on the very next tick; from the
/// next one on, the retry waits `backoff_ms(n - FREE_REDISCOVERS)`, so a
/// flapping provider (e.g. a GPU layer after a driver reset) cannot
/// rediscover every second forever.
const FREE_REDISCOVERS: u32 = 3;

pub(crate) struct Sample {
    pub inventory: Inventory,
    pub values: Vec<Option<f64>>,
    /// The provider reported that this poll carried no new measurement.
    pub repeated: bool,
}

pub(crate) struct Worker {
    tx: SyncSender<u64>,
    rx: Receiver<Sample>,
    pending: bool,
}

impl Worker {
    pub fn spawn(mut provider: Box<dyn Provider>) -> Self {
        let (tx, requests) = mpsc::sync_channel::<u64>(1);
        let (responses, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name(format!("oma-{}", provider.name()))
            .spawn(move || {
                let mut inventory = Inventory::default();
                let mut discover = true;
                let mut failures = 0u32;
                let mut rediscovers = 0u32;
                let mut retry_at = 0u64;
                while let Ok(now) = requests.recv() {
                    let mut values = vec![None; inventory.sensors.len()];
                    let mut repeated = false;
                    if now >= retry_at {
                        // A Rust panic is isolated; native DLL access violations are not catchable.
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            if discover {
                                inventory = provider.discover()?;
                                discover = false;
                            }
                            let polled = provider.poll()?;
                            if polled.len() != inventory.sensors.len() {
                                return Err(ProviderError::Failed("poll value count mismatch".into()));
                            }
                            Ok((polled, provider.repeated()))
                        }))
                        .unwrap_or_else(|_| Err(ProviderError::Failed("provider panicked".into())));
                        match result {
                            Ok((polled, was_repeated)) => {
                                values = polled;
                                repeated = was_repeated;
                                // Only a successful poll ends a failure or rediscovery streak.
                                failures = 0;
                                rediscovers = 0;
                            }
                            Err(ProviderError::Rediscover) => {
                                discover = true;
                                values = vec![None; inventory.sensors.len()];
                                rediscovers = rediscovers.saturating_add(1);
                                if rediscovers > FREE_REDISCOVERS {
                                    let extra = rediscovers - FREE_REDISCOVERS;
                                    retry_at = now.saturating_add(backoff_ms(extra));
                                    tracing::warn!(
                                        provider = provider.name(),
                                        rediscovers,
                                        "provider keeps requesting rediscovery"
                                    );
                                }
                            }
                            Err(err) => {
                                failures = failures.saturating_add(1);
                                retry_at = now.saturating_add(backoff_ms(failures));
                                discover = true;
                                values = vec![None; inventory.sensors.len()];
                                tracing::warn!(provider = provider.name(), %err, failures, "provider degraded");
                            }
                        }
                    }
                    if responses
                        .send(Sample {
                            inventory: inventory.clone(),
                            values,
                            repeated,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("provider worker");
        // Deliberately do not join a worker executing an uninterruptible Win32 call.
        // Dropping the channels ends an idle worker; blocked workers exit with the process.
        Self {
            tx,
            rx,
            pending: false,
        }
    }

    pub fn start(&mut self, monotonic_ms: u64) {
        if !self.pending && self.tx.try_send(monotonic_ms).is_ok() {
            self.pending = true;
        }
    }

    pub fn finish(&mut self, deadline: Instant) -> Option<Sample> {
        if !self.pending {
            return None;
        }
        match self
            .rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(sample) => {
                self.pending = false;
                Some(sample)
            }
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                self.pending = false;
                None
            }
        }
    }
}
