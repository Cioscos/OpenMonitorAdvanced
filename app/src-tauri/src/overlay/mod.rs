//! The game overlay's app side (M7). In M7b there is no overlay window yet:
//! [`target`] picks the game to follow and [`frames`] logs its frame metrics
//! when `OMA_FRAMES_DEBUG` asks for it.

// Used by the overlay controller from C16 on.
#[allow(dead_code)]
pub mod forward;
pub mod frames;
// Used by the overlay controller from C16 on.
#[allow(dead_code)]
pub mod host;
// Used by the overlay controller from C16 on.
#[allow(dead_code)]
pub mod profiles;
pub mod target;
