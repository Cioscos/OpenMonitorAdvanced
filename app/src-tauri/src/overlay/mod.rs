//! The game overlay's app side (M7): [`target`] picks the game to follow,
//! [`controller`] decides what the frame engine and the overlay do,
//! [`runner`] runs it on the `oma-overlay-ctl` thread with the service link,
//! the foreground watcher and the sampler, [`forward`] builds the messages
//! for `oma-overlay.exe`, [`host`] runs it, [`profiles`] reads the profile
//! catalog, [`store`] writes the editor's profile files and [`frames`]
//! formats the `OMA_FRAMES_DEBUG` line.

#[allow(dead_code)] // wired into the controller in D12
pub mod benchmark;
#[cfg(windows)]
pub mod controller;
pub mod editor;
#[cfg(windows)]
pub mod editor_feed;
pub mod forward;
pub mod frames;
pub mod host;
pub mod profiles;
#[cfg(windows)]
pub mod runner;
pub mod store;
pub mod target;

/// The overlay's Tauri commands off Windows, where there is no overlay.
#[cfg(not(windows))]
pub mod runner {
    #[tauri::command]
    pub fn get_overlay_status() -> Option<()> {
        None
    }

    #[tauri::command]
    pub fn overlay_retry() {}

    #[tauri::command]
    pub fn overlay_reload_profiles() {}

    #[tauri::command]
    pub fn set_overlay_hidden(hidden: bool) {
        let _ = hidden;
    }

    #[tauri::command]
    pub fn overlay_preview(json: Option<String>) -> Result<(), super::editor::CommandError> {
        let _ = json;
        Ok(())
    }

    #[tauri::command]
    pub fn overlay_editor_profile(json: Option<String>) -> Result<(), super::editor::CommandError> {
        let _ = json;
        Ok(())
    }

    #[tauri::command]
    pub fn overlay_use_now(id: String) -> Result<(), super::editor::CommandError> {
        let _ = id;
        Ok(())
    }
}
