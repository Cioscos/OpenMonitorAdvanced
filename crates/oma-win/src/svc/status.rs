//! The service status shared between the connection thread, which writes it,
//! and the shell, which reads it on its sampler callback and emits an event
//! when the version changes (plan decision D7, like `GpuProcessTable`).

use std::sync::{Arc, Mutex, PoisonError};

pub use oma_ipc::{ServiceDetail, ServiceState, ServiceStatus};

/// Status before the connection thread has said anything.
const INITIAL: ServiceStatus = ServiceStatus::new(ServiceState::Starting, None);

/// Latest service status with a version that changes only when the status
/// does; cheap to clone (shared state).
#[derive(Clone)]
pub struct ServiceStatusTable(Arc<Mutex<(u64, ServiceStatus)>>);

impl Default for ServiceStatusTable {
    fn default() -> Self {
        Self(Arc::new(Mutex::new((0, INITIAL))))
    }
}

impl ServiceStatusTable {
    /// The version and the status.
    pub fn get(&self) -> (u64, ServiceStatus) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the status; the version bumps only when it differs.
    pub fn set(&self, status: ServiceStatus) {
        let mut entry = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if entry.1 != status {
            *entry = (entry.0 + 1, status);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_version_bumps_only_on_change() {
        let table = ServiceStatusTable::default();
        let (v0, initial) = table.get();
        assert_eq!(initial, INITIAL);

        let connected = ServiceStatus::new(ServiceState::Connected, None);
        table.set(connected);
        let (v1, status) = table.get();
        assert_eq!(status, connected);
        assert!(v1 > v0);

        table.set(connected);
        table.set(connected);
        assert_eq!(table.get(), (v1, connected));

        let clone = table.clone();
        clone.set(ServiceStatus::new(
            ServiceState::Unreachable,
            Some(ServiceDetail::Disconnected),
        ));
        let (v2, status) = table.get();
        assert!(v2 > v1);
        assert_eq!(status.detail, Some(ServiceDetail::Disconnected));
    }
}
