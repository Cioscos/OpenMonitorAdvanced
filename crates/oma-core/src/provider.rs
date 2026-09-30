//! Contract between data sources and the engine.

use crate::model::{Device, Sensor};

/// Devices and sensors a provider exposes. `poll` values follow `sensors` order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Inventory {
    pub devices: Vec<Device>,
    pub sensors: Vec<Sensor>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProviderError {
    /// The hardware set changed (e.g. a disk was plugged in): call `discover` again.
    #[error("hardware configuration changed")]
    Rediscover,
    #[error("{0}")]
    Failed(String),
}

/// A source of sensor readings (PDH, a vendor SDK, the privileged service...).
pub trait Provider: Send {
    /// Short name used in logs.
    fn name(&self) -> &'static str;

    /// (Re)initialises the provider and lists its devices and sensors.
    fn discover(&mut self) -> Result<Inventory, ProviderError>;

    /// Reads current values, aligned with the sensors of the last `discover`.
    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError>;

    /// `true` when the last `poll` carried no new measurement: its values
    /// repeat the ones before (e.g. the service has not sent a new snapshot
    /// yet). The engine marks them `Quality::Held`. A cache the source
    /// declares valid within its TTL is not a repeat.
    fn repeated(&self) -> bool {
        false
    }
}
