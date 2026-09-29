//! The service status shared between the connection thread, which writes it,
//! and the shell, which reads it on its sampler callback and emits an event
//! when the version changes (plan decision D7, like `GpuProcessTable`).

use std::sync::{Arc, Mutex, PoisonError};

pub use oma_ipc::{ServiceDetail, ServiceState, ServiceStatus};

/// Called with each new status, on the thread that set it and outside the
/// table's lock.
pub type StatusObserver = Box<dyn Fn(&ServiceStatus) + Send + Sync>;

/// Status before the connection thread has said anything.
fn initial() -> ServiceStatus {
    ServiceStatus::new(ServiceState::Starting, None)
}

struct Inner {
    state: Mutex<(u64, ServiceStatus)>,
    observers: Mutex<Vec<Arc<StatusObserver>>>,
    /// `Hello.service_version` of the last service that answered; outside the
    /// status, so it never causes an `oma:service` event.
    service_version: Mutex<Option<String>>,
}

/// Latest service status with a version that changes only when the status
/// does; cheap to clone (shared state).
#[derive(Clone)]
pub struct ServiceStatusTable(Arc<Inner>);

impl Default for ServiceStatusTable {
    fn default() -> Self {
        Self(Arc::new(Inner {
            state: Mutex::new((0, initial())),
            observers: Mutex::new(Vec::new()),
            service_version: Mutex::new(None),
        }))
    }
}

impl ServiceStatusTable {
    /// The version and the status.
    pub fn get(&self) -> (u64, ServiceStatus) {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The version alone: what a caller that polls compares before it pays
    /// for a [`get`](Self::get).
    pub fn version(&self) -> u64 {
        self.0
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .0
    }

    /// Replaces the status; the version bumps, and the observers hear it,
    /// only when it differs (`true`). The status is copied only in that case.
    pub fn set(&self, status: &ServiceStatus) -> bool {
        {
            let mut entry = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            if entry.1 == *status {
                return false;
            }
            *entry = (entry.0 + 1, status.clone());
        }
        let observers = self
            .0
            .observers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for observer in observers {
            observer(status);
        }
        true
    }

    /// The version the last service that said `Hello` reported, if any.
    pub fn service_version(&self) -> Option<String> {
        self.0
            .service_version
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replaces the service version (copied only when it differs).
    pub fn set_service_version(&self, version: Option<&str>) {
        let mut current = self
            .0
            .service_version
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if current.as_deref() != version {
            *current = version.map(str::to_owned);
        }
    }

    /// Registers an observer for every later change. It runs on the thread
    /// that changes the status (the link thread), so it must be quick and
    /// never wait for the link.
    pub fn subscribe(&self, observer: StatusObserver) {
        self.0
            .observers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::new(observer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_version_is_kept_apart_from_the_status() {
        let table = ServiceStatusTable::default();
        assert_eq!(table.service_version(), None);
        let (v0, _) = table.get();
        table.set_service_version(Some("1.2.3"));
        assert_eq!(table.service_version().as_deref(), Some("1.2.3"));
        assert_eq!(table.get().0, v0, "no status change, no event");
        table.set_service_version(None);
        assert_eq!(table.service_version(), None);
    }

    #[test]
    fn status_version_bumps_only_on_change() {
        let table = ServiceStatusTable::default();
        let (v0, initial) = table.get();
        assert_eq!(initial, super::initial());

        let connected = ServiceStatus::new(ServiceState::Connected, None);
        table.set(&connected);
        let (v1, status) = table.get();
        assert_eq!(status, connected);
        assert!(v1 > v0);

        table.set(&connected);
        table.set(&connected);
        assert_eq!(table.get(), (v1, connected.clone()));

        let clone = table.clone();
        clone.set(&ServiceStatus::new(
            ServiceState::Unreachable,
            Some(ServiceDetail::Disconnected),
        ));
        let (v2, status) = table.get();
        assert!(v2 > v1);
        assert_eq!(status.detail, Some(ServiceDetail::Disconnected));
    }

    #[test]
    fn version_is_readable_without_a_copy() {
        let table = ServiceStatusTable::default();
        assert_eq!(table.version(), 0);
        table.set(&ServiceStatus::new(ServiceState::Connected, None));
        assert_eq!(table.version(), table.get().0);
        assert_eq!(table.version(), 1);
    }

    #[test]
    fn extras_count_as_a_change() {
        let table = ServiceStatusTable::default();
        let mut connected = ServiceStatus::new(ServiceState::Connected, None);
        table.set(&connected);
        let (v1, _) = table.get();

        connected.pawn_io = Some(oma_ipc::PawnIoStatus::Missing);
        table.set(&connected);
        let (v2, status) = table.get();
        assert!(v2 > v1, "PawnIO alone bumps the version");
        assert_eq!(status.pawn_io, Some(oma_ipc::PawnIoStatus::Missing));

        connected.sources = Some(oma_ipc::ServiceSources {
            active_modules: vec!["cpu".to_owned()],
            requested_disabled_modules: Vec::new(),
            smart_disabled_drives: Vec::new(),
            reconfiguration: oma_ipc::Reconfiguration::Pending,
            smart_blocked_by: Vec::new(),
        });
        table.set(&connected);
        assert!(table.get().0 > v2, "sources alone bump the version");
    }

    #[test]
    fn observers_hear_each_change_once_outside_the_lock() {
        let table = ServiceStatusTable::default();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&heard);
        let reader = table.clone();
        table.subscribe(Box::new(move |status| {
            // Reading the table from an observer must not deadlock.
            let _ = reader.get();
            sink.lock().unwrap().push(status.state);
        }));

        let connected = ServiceStatus::new(ServiceState::Connected, None);
        table.set(&connected);
        table.set(&connected);
        table.set(&ServiceStatus::new(ServiceState::Unreachable, None));
        assert_eq!(
            *heard.lock().unwrap(),
            vec![ServiceState::Connected, ServiceState::Unreachable]
        );
    }
}
