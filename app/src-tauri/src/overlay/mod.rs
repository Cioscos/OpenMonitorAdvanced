//! The game overlay's app side (M7): [`target`] picks the game to follow,
//! [`controller`] decides what the frame engine and the overlay do,
//! [`forward`] builds the messages for `oma-overlay.exe`, [`host`] runs it,
//! [`profiles`] reads the profile catalog and [`frames`] formats the
//! `OMA_FRAMES_DEBUG` line.

// Wired to its thread from C16 on; until then only the diagnostics use it.
#[cfg(windows)]
#[allow(dead_code)]
pub mod controller;
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
