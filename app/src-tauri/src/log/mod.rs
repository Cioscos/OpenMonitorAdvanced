//! CSV sensor log: the bounded queue between the sampler and the writer
//! thread, the writer itself, the filesystem it writes through, the session
//! coordinator and its Tauri commands.

use serde::Serialize;

pub mod commands;
pub mod fs;
pub mod queue;
pub mod session;
pub mod writer;

pub use session::{LogService, CLOSE_TIMEOUT};

/// What became of one global hotkey of the log (filled by the hotkey
/// manager; until then both are unset).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatus {
    /// The combination the settings ask for.
    pub requested: Option<String>,
    /// The combination registered now (the previous one after a failed change).
    pub effective: Option<String>,
    pub state: HotkeyState,
    /// i18n key of the failure.
    pub reason: Option<String>,
}

impl Default for HotkeyStatus {
    fn default() -> Self {
        Self {
            requested: None,
            effective: None,
            state: HotkeyState::Unset,
            reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HotkeyState {
    // Built by the hotkey manager (Task 7).
    #[allow(dead_code)]
    Active,
    Unset,
    // Built by the hotkey manager (Task 7).
    #[allow(dead_code)]
    Failed,
}

/// The two hotkeys of the log, as `LogStatus.hotkeys`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct HotkeyStatuses {
    pub toggle: HotkeyStatus,
    pub pause: HotkeyStatus,
}
