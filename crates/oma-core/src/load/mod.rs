//! Stress-test catalogue, profiles and plan builder (pure; no Windows code). The plan
//! types themselves live in `oma-ipc::load`.

mod catalog;
mod plan;

pub use catalog::catalog_json;
pub use plan::{
    build_plan, core_order, presets, ram_budget, BuildError, BuildInput, Component, Custom,
    ModeEdit, Objective, Preset, RetryCore, StartRequest, ThreadChoice,
};
