//! The game overlay's app side (M7). In M7b there is no overlay window yet:
//! [`target`] picks the game to follow and [`frames`] logs its frame metrics
//! when `OMA_FRAMES_DEBUG` asks for it.

pub mod frames;
pub mod target;
