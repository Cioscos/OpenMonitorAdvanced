//! The `overlay` section: in-game overlay settings.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::{Attach, ChartFps};
use crate::overlay::templates::BuiltinId;

/// Accepted values of `overlay.textHz`, ascending.
pub const TEXT_HZ: [u32; 2] = [2, 4];
/// Accepted values of `overlay.chartFps`, ascending.
pub const CHART_FPS: [u32; 3] = ChartFps::VALUES;
/// Most entries a `blockedGames` list or a `gameProfiles` map may hold.
pub const MAX_GAMES: usize = 256;
/// Longest executable name, in bytes.
pub const MAX_EXE_BYTES: usize = 260;
/// Smallest stored editor window, in physical pixels.
pub const EDITOR_MIN_WIDTH: u32 = 1100;
pub const EDITOR_MIN_HEIGHT: u32 = 700;
/// Largest coordinate or size of the stored editor window.
const MAX_COORDINATE: i64 = 32_768;
/// Default of `overlay.defaultProfile`: the built-in «Gaming» template.
pub const DEFAULT_PROFILE: &str = "builtin-gaming";

#[derive(Debug, Clone, PartialEq)]
pub struct OverlaySettings {
    pub enabled: bool,
    pub chart_fps: ChartFps,
    /// Text refresh rate; one of [`TEXT_HZ`].
    pub text_hz: u32,
    pub hide_from_capture: bool,
    pub attach: Attach,
    pub track_pc_latency: bool,
    pub track_gpu: bool,
    /// A known `builtin-*` id or a lowercase UUID. An id that no longer exists
    /// is not a settings error: it is resolved when the profile is used.
    pub default_profile: String,
    /// Lowercase executable name -> profile id (same rule as `default_profile`).
    pub game_profiles: BTreeMap<String, String>,
    /// Lowercase executable names, unique.
    pub blocked_games: Vec<String>,
    /// Canonical hotkey text, or `None`.
    pub hotkey_toggle: Option<String>,
    pub hotkey_next_profile: Option<String>,
    pub hotkey_benchmark: Option<String>,
    /// Where the editor window was last closed (DD12), `None` to center it.
    pub editor_bounds: Option<WindowBounds>,
}

/// A window's outer position and inner size, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowBounds {
    /// At least 1100×700 and nothing beyond ±32768.
    pub fn is_valid(&self) -> bool {
        let within = |v: i64| (-MAX_COORDINATE..=MAX_COORDINATE).contains(&v);
        self.width >= EDITOR_MIN_WIDTH
            && self.height >= EDITOR_MIN_HEIGHT
            && [
                self.x.into(),
                self.y.into(),
                self.width.into(),
                self.height.into(),
            ]
            .into_iter()
            .all(within)
    }

    /// From its JSON spelling; `None` unless every field is an integer in
    /// range, there are exactly these four and [`Self::is_valid`] holds.
    pub fn from_json(value: &Value) -> Option<Self> {
        let obj = value.as_object().filter(|o| o.len() == 4)?;
        let int = |key: &str| obj.get(key)?.as_i64();
        let bounds = Self {
            x: i32::try_from(int("x")?).ok()?,
            y: i32::try_from(int("y")?).ok()?,
            width: u32::try_from(int("width")?).ok()?,
            height: u32::try_from(int("height")?).ok()?,
        };
        bounds.is_valid().then_some(bounds)
    }

    fn to_json(self) -> Value {
        json!({"x": self.x, "y": self.y, "width": self.width, "height": self.height})
    }
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            chart_fps: ChartFps::Fps30,
            text_hz: 2,
            hide_from_capture: false,
            attach: Attach::Window,
            track_pc_latency: false,
            track_gpu: false,
            default_profile: DEFAULT_PROFILE.to_string(),
            game_profiles: BTreeMap::new(),
            blocked_games: Vec::new(),
            hotkey_toggle: None,
            hotkey_next_profile: None,
            hotkey_benchmark: None,
            editor_bounds: None,
        }
    }
}

impl OverlaySettings {
    /// The JSON spelling; every key is always present.
    pub(super) fn encode(&self) -> Value {
        json!({
            "enabled": self.enabled,
            "chartFps": self.chart_fps.as_u32(),
            "textHz": self.text_hz,
            "hideFromCapture": self.hide_from_capture,
            "attach": self.attach.as_str(),
            "trackPcLatency": self.track_pc_latency,
            "trackGpu": self.track_gpu,
            "defaultProfile": self.default_profile,
            "gameProfiles": self.game_profiles,
            "blockedGames": self.blocked_games,
            "hotkeyToggle": self.hotkey_toggle,
            "hotkeyNextProfile": self.hotkey_next_profile,
            "hotkeyBenchmark": self.hotkey_benchmark,
            "editorBounds": self.editor_bounds.map(WindowBounds::to_json),
        })
    }
}

/// The lowercase spelling of an executable name, or `None` when it is not one:
/// it must end in `.exe` after a non-empty stem, hold no `\`, `/` or `:`, and
/// fit in [`MAX_EXE_BYTES`].
pub fn normalize_exe(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let stem = lower.strip_suffix(".exe")?;
    let valid =
        !stem.is_empty() && lower.len() <= MAX_EXE_BYTES && !lower.contains(['\\', '/', ':']);
    valid.then_some(lower)
}

/// Whether `id` is a known built-in profile id or a lowercase `8-4-4-4-12`
/// UUID.
pub fn is_profile_id(id: &str) -> bool {
    if BuiltinId::parse(id).is_some() {
        return true;
    }
    let groups: Vec<&str> = id.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(group, len)| {
            group.len() == len
                && group
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_names() {
        assert_eq!(normalize_exe("Game.EXE").as_deref(), Some("game.exe"));
        for bad in [
            "",
            ".exe",
            "game",
            "game.exe.txt",
            "C:\\g.exe",
            "a/b.exe",
            "x:y.exe",
        ] {
            assert_eq!(normalize_exe(bad), None, "{bad}");
        }
        let long = format!("{}.exe", "a".repeat(MAX_EXE_BYTES - 4));
        assert_eq!(normalize_exe(&long).as_deref(), Some(long.as_str()));
        assert_eq!(normalize_exe(&format!("a{long}")), None);
    }

    #[test]
    fn profile_ids() {
        for ok in [
            "builtin-gaming",
            "builtin-minimal-fps",
            "00000000-0000-4000-8000-000000000002",
        ] {
            assert!(is_profile_id(ok), "{ok}");
        }
        for bad in [
            "",
            "builtin-nope",
            "gaming",
            "00000000-0000-4000-8000-00000000000G",
            "00000000-0000-4000-8000-00000000000A",
            "00000000-0000-4000-8000-0000000000002",
        ] {
            assert!(!is_profile_id(bad), "{bad}");
        }
    }
}
