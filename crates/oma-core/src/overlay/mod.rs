//! Overlay profile model, validation and pixel geometry. Pure: no Windows
//! code, so the controller and the renderer share one definition.

pub mod eval;
pub mod geometry;
pub mod profile;
pub mod templates;

pub use eval::{fg_active, is_visible, threshold_color, StatRing};
pub use geometry::{cell_px, footprint, place, Foreground, PxRect, WindowGeometry};
pub use profile::*;
pub use templates::{builtin_profile, BuiltinId};
