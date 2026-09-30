//! The `log` section: CSV sensor log settings.

use std::ops::RangeInclusive;

use serde_json::{json, Value};

use crate::hotkey::parse_hotkey;

/// Accepted values of `log.everyTicks`, ascending: one row every N ticks.
pub const EVERY_TICKS: [u32; 6] = [1, 2, 5, 10, 30, 60];
/// Accepted values of `log.maxFileMb`.
pub const MAX_FILE_MB: RangeInclusive<u32> = 10..=2048;
/// Most sensors a `log.sensors` list may hold.
pub const MAX_LOG_SENSORS: usize = 4096;
/// Default of `log.hotkeyToggle`.
pub const DEFAULT_HOTKEY_TOGGLE: &str = "Ctrl+Alt+Shift+R";

#[derive(Debug, Clone, PartialEq)]
pub struct LogSettings {
    /// `None` = `Documents\OpenMonitor Advanced\logs`; otherwise an absolute path.
    pub folder: Option<String>,
    /// `None` = every sensor; otherwise sensor ids, unique and non-empty.
    pub sensors: Option<Vec<String>>,
    /// One row every this many ticks; one of [`EVERY_TICKS`].
    pub every_ticks: u32,
    /// Size at which a new part starts, in MiB; within [`MAX_FILE_MB`].
    pub max_file_mb: u32,
    /// Canonical hotkey text, or `None` for no hotkey.
    pub hotkey_toggle: Option<String>,
    /// Canonical hotkey text, or `None`; never equal to `hotkey_toggle`.
    pub hotkey_pause: Option<String>,
}

impl Default for LogSettings {
    fn default() -> Self {
        Self {
            folder: None,
            sensors: None,
            every_ticks: 1,
            max_file_mb: 100,
            hotkey_toggle: Some(DEFAULT_HOTKEY_TOGGLE.to_string()),
            hotkey_pause: None,
        }
    }
}

impl LogSettings {
    /// The JSON spelling; every key is always present.
    pub(super) fn encode(&self) -> Value {
        json!({
            "folder": self.folder,
            "sensors": self.sensors,
            "everyTicks": self.every_ticks,
            "maxFileMb": self.max_file_mb,
            "hotkeyToggle": self.hotkey_toggle,
            "hotkeyPause": self.hotkey_pause,
        })
    }
}

/// Whether `path` is `X:\…` (or `X:/…`) or `\\server\share…`. A string check,
/// so it holds on every platform and never touches the disk.
pub fn is_absolute_folder(path: &str) -> bool {
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return matches!(bytes[2], b'\\' | b'/');
    }
    match path.strip_prefix("\\\\") {
        Some(rest) => {
            let mut parts = rest.split('\\');
            parts.next().is_some_and(|server| !server.is_empty())
                && parts.next().is_some_and(|share| !share.is_empty())
        }
        None => false,
    }
}

/// The canonical spelling of a hotkey text, or `None` when it does not parse.
pub(super) fn canonical_hotkey(text: &str) -> Option<String> {
    parse_hotkey(text).ok().map(|hotkey| hotkey.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_defaults_match_the_spec() {
        let log = LogSettings::default();
        assert_eq!(log.folder, None);
        assert_eq!(log.sensors, None);
        assert_eq!(log.every_ticks, 1);
        assert_eq!(log.max_file_mb, 100);
        assert_eq!(log.hotkey_toggle.as_deref(), Some("Ctrl+Alt+Shift+R"));
        assert_eq!(log.hotkey_pause, None);
        assert_eq!(
            log.encode(),
            json!({"folder": null, "sensors": null, "everyTicks": 1, "maxFileMb": 100,
                   "hotkeyToggle": "Ctrl+Alt+Shift+R", "hotkeyPause": null})
        );
        assert!(EVERY_TICKS.contains(&log.every_ticks) && MAX_FILE_MB.contains(&log.max_file_mb));
        assert_eq!(
            canonical_hotkey(DEFAULT_HOTKEY_TOGGLE).as_deref(),
            Some(DEFAULT_HOTKEY_TOGGLE)
        );
    }

    #[test]
    fn absolute_folders() {
        for ok in [
            "C:\\logs",
            "d:/logs",
            "C:\\",
            "\\\\srv\\share",
            "\\\\srv\\share\\dir",
        ] {
            assert!(is_absolute_folder(ok), "{ok}");
        }
        for bad in [
            "",
            "logs",
            "C:",
            "C:logs",
            "\\logs",
            "\\\\srv",
            "\\\\srv\\",
            "\\\\\\share",
            "./x",
            "1:\\x",
        ] {
            assert!(!is_absolute_folder(bad), "{bad}");
        }
    }
}
