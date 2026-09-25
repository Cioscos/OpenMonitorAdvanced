//! Contract between the GPU provider and one data source (D3DKMT, DXGI, PDH, a vendor library).

use std::collections::{BTreeMap, BTreeSet};

use oma_core::model::Source;
use oma_core::provider::ProviderError;

use super::adapter::Adapter;
use super::field::GpuField;

/// Values of one adapter for one tick; a field missing from the map is unavailable.
pub(crate) type Readings = BTreeMap<GpuField, f64>;

pub(crate) trait GpuLayer: Send {
    fn source(&self) -> Source;

    /// Called on every discover. Returns, for each adapter (same order), the fields this layer
    /// provides for it. Never fails: an unusable layer/adapter returns empty sets.
    fn attach(&mut self, adapters: &[Adapter]) -> Vec<BTreeSet<GpuField>>;

    /// Values for the adapters of the last attach (same order, same length). A field missing from
    /// a map is unavailable this tick. Err(ProviderError::Rediscover) when the adapter binding is stale
    /// (e.g. D3DKMT handle invalid after a driver update / TDR); other errors are logged by the caller
    /// and that layer's values count as unavailable for this tick.
    fn sample(&mut self) -> Result<Vec<Readings>, ProviderError>;

    /// Experimental fields of this layer (spec §5.2: undocumented NVAPI calls).
    fn is_experimental(&self, _field: GpuField) -> bool {
        false
    }
}
