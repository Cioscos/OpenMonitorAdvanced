//! Platform-independent core of OpenMonitor Advanced: data model, providers,
//! engine, history and sampling loop.

pub mod csv;
pub mod engine;
pub mod history;
pub mod hotkey;
pub mod merge;
pub mod model;
pub mod provider;
pub mod rate;
pub mod rules;
pub mod sampler;
pub mod sanitize;
pub mod settings;
pub mod stats;
pub mod updates;
mod worker;
