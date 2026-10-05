//! Overlay profile model, validation and pixel geometry. Pure: no Windows
//! code, so the controller and the renderer share one definition.

pub mod geometry;
pub mod profile;

pub use geometry::{cell_px, footprint, place, Foreground, PxRect, WindowGeometry};
pub use profile::*;
