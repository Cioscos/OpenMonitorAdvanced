//! Portable settings model: types, defaults, tolerant decoding of the settings
//! file and strict validation of patches. Reading and writing the file itself
//! lives in the shell (`app/src-tauri`).

mod decode;
pub mod log;
pub mod overlay;
mod patch;
pub mod performance;

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::rules::RulesSettings;

pub use log::LogSettings;
pub use overlay::OverlaySettings;
pub use performance::PerformanceSettings;

pub use decode::{decode_lenient, Decoded, Diagnostic, DiagnosticKind, VersionStatus};
pub use patch::{apply_patch, reset_rule_override, PatchError};

/// Format version written to and understood from the settings file.
pub const SETTINGS_VERSION: u32 = 1;

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),+ }

        impl $name {
            /// The JSON spelling of this variant.
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            /// Parses the JSON spelling; `None` for an unknown variant.
            pub fn parse(text: &str) -> Option<Self> {
                match text { $($text => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

string_enum!(
    /// UI language.
    Language { System => "system", En => "en", It => "it" }
);
string_enum!(
    /// Temperature unit shown in the UI.
    TemperatureUnit { C => "c", F => "f" }
);
string_enum!(
    /// Network throughput unit shown in the UI.
    ThroughputUnit { Bits => "bits", Bytes => "bytes" }
);
string_enum!(
    /// View opened at startup.
    DefaultView { Simple => "simple", Advanced => "advanced", Last => "last" }
);
string_enum!(
    /// What the overlay window follows: the game window or its monitor.
    Attach { Window => "window", Monitor => "monitor" }
);
string_enum!(
    /// A concrete view.
    ViewKind { Simple => "simple", Advanced => "advanced" }
);

impl Default for Language {
    fn default() -> Self {
        Self::System
    }
}
impl Default for TemperatureUnit {
    fn default() -> Self {
        Self::C
    }
}
impl Default for ThroughputUnit {
    fn default() -> Self {
        Self::Bits
    }
}
impl Default for Attach {
    fn default() -> Self {
        Self::Window
    }
}
impl Default for DefaultView {
    fn default() -> Self {
        Self::Last
    }
}

/// Chart frame rate; serialized as the JSON number 60, 30 or 15.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChartFps {
    #[default]
    Fps60,
    Fps30,
    Fps15,
}

impl ChartFps {
    /// Accepted values, ascending.
    pub const VALUES: [u32; 3] = [15, 30, 60];

    pub fn as_u32(self) -> u32 {
        match self {
            Self::Fps60 => 60,
            Self::Fps30 => 30,
            Self::Fps15 => 15,
        }
    }

    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            60 => Some(Self::Fps60),
            30 => Some(Self::Fps30),
            15 => Some(Self::Fps15),
            _ => None,
        }
    }
}

/// Accepted values of `general.intervalMs`, ascending (500 to 5000, step 500).
pub const INTERVAL_VALUES: [u32; 10] = [500, 1000, 1500, 2000, 2500, 3000, 3500, 4000, 4500, 5000];
/// Accepted values of `advanced.window`, in seconds, ascending.
pub const WINDOW_VALUES: [u32; 4] = [60, 300, 1800, 3600];

#[derive(Clone, Debug, PartialEq)]
pub struct General {
    pub language: Language,
    pub temperature_unit: TemperatureUnit,
    pub throughput_unit: ThroughputUnit,
    pub interval_ms: u32,
    pub chart_fps: ChartFps,
    pub default_view: DefaultView,
}

impl Default for General {
    fn default() -> Self {
        Self {
            language: Language::default(),
            temperature_unit: TemperatureUnit::default(),
            throughput_unit: ThroughputUnit::default(),
            interval_ms: 1000,
            chart_fps: ChartFps::default(),
            default_view: DefaultView::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tray {
    pub close_to_tray: bool,
    pub autostart: bool,
    /// `None` = automatic; otherwise a sensor id.
    pub icon_sensor: Option<String>,
}

impl Default for Tray {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            autostart: false,
            icon_sensor: None,
        }
    }
}

/// Update check preferences. The check is opt-in: nothing touches the network by default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Updates {
    pub check_automatically: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VendorLibraries {
    pub nvml: bool,
    pub nvapi: bool,
    pub adl: bool,
    pub igcl: bool,
}

impl Default for VendorLibraries {
    fn default() -> Self {
        Self {
            nvml: true,
            nvapi: true,
            adl: true,
            igcl: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServiceModules {
    pub cpu: bool,
    pub motherboard: bool,
    pub memory: bool,
    pub storage: bool,
    pub controller: bool,
    pub psu: bool,
}

impl Default for ServiceModules {
    fn default() -> Self {
        Self {
            cpu: true,
            motherboard: true,
            memory: true,
            storage: true,
            controller: true,
            psu: true,
        }
    }
}

impl ServiceModules {
    /// Names of the modules that are turned off, in declaration order.
    pub fn disabled(&self) -> Vec<&'static str> {
        [
            ("cpu", self.cpu),
            ("motherboard", self.motherboard),
            ("memory", self.memory),
            ("storage", self.storage),
            ("controller", self.controller),
            ("psu", self.psu),
        ]
        .into_iter()
        .filter(|(_, enabled)| !enabled)
        .map(|(name, _)| name)
        .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sources {
    pub vendor_libraries: VendorLibraries,
    pub anti_cheat: bool,
    pub service_modules: ServiceModules,
    pub smart_disabled_drives: Vec<String>,
    /// Core ids of the disks whose SMART is off by default and that the user
    /// switched on.
    pub smart_enabled_drives: Vec<String>,
}

/// Advanced view state. `None` means "never set", which is distinct from a
/// default value (spec §2.4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AdvancedState {
    pub section: Option<String>,
    pub window: Option<u32>,
    /// Section id -> sensor ids.
    pub series: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewState {
    pub last: Option<ViewKind>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Migrations {
    pub service_v1: bool,
    pub webview_v1: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub version: u32,
    pub general: General,
    pub tray: Tray,
    pub updates: Updates,
    pub sources: Sources,
    pub advanced: AdvancedState,
    pub view: ViewState,
    /// Overrides of the built-in rules and the custom rules; always passes
    /// [`crate::rules::validate_rules`].
    pub rules: RulesSettings,
    pub log: LogSettings,
    pub overlay: OverlaySettings,
    pub performance: PerformanceSettings,
    pub migrations: Migrations,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            general: General::default(),
            tray: Tray::default(),
            updates: Updates::default(),
            sources: Sources::default(),
            advanced: AdvancedState::default(),
            view: ViewState::default(),
            rules: RulesSettings::default(),
            log: LogSettings::default(),
            overlay: OverlaySettings::default(),
            performance: PerformanceSettings::default(),
            migrations: Migrations::default(),
        }
    }
}

/// Encodes settings as camelCase JSON. Unset `advanced.section`,
/// `advanced.window` and `view.last` are omitted; `tray.iconSensor` is `null`
/// when automatic.
pub fn encode(settings: &Settings) -> Value {
    let mut advanced = Map::new();
    if let Some(section) = &settings.advanced.section {
        advanced.insert("section".into(), json!(section));
    }
    if let Some(window) = settings.advanced.window {
        advanced.insert("window".into(), json!(window));
    }
    advanced.insert("series".into(), json!(settings.advanced.series));

    let mut view = Map::new();
    if let Some(last) = settings.view.last {
        view.insert("last".into(), json!(last.as_str()));
    }

    let s = &settings.sources;
    json!({
        "version": settings.version,
        "general": {
            "language": settings.general.language.as_str(),
            "temperatureUnit": settings.general.temperature_unit.as_str(),
            "throughputUnit": settings.general.throughput_unit.as_str(),
            "intervalMs": settings.general.interval_ms,
            "chartFps": settings.general.chart_fps.as_u32(),
            "defaultView": settings.general.default_view.as_str(),
        },
        "tray": {
            "closeToTray": settings.tray.close_to_tray,
            "autostart": settings.tray.autostart,
            "iconSensor": settings.tray.icon_sensor,
        },
        "updates": {
            "checkAutomatically": settings.updates.check_automatically,
        },
        "sources": {
            "vendorLibraries": {
                "nvml": s.vendor_libraries.nvml,
                "nvapi": s.vendor_libraries.nvapi,
                "adl": s.vendor_libraries.adl,
                "igcl": s.vendor_libraries.igcl,
            },
            "antiCheat": s.anti_cheat,
            "serviceModules": {
                "cpu": s.service_modules.cpu,
                "motherboard": s.service_modules.motherboard,
                "memory": s.service_modules.memory,
                "storage": s.service_modules.storage,
                "controller": s.service_modules.controller,
                "psu": s.service_modules.psu,
            },
            "smartDisabledDrives": s.smart_disabled_drives,
            "smartEnabledDrives": s.smart_enabled_drives,
        },
        "advanced": advanced,
        "view": view,
        "rules": serde_json::to_value(&settings.rules).unwrap_or_else(|_| json!({})),
        "log": settings.log.encode(),
        "overlay": settings.overlay.encode(),
        "performance": settings.performance.encode(),
        "migrations": {
            "serviceV1": settings.migrations.service_v1,
            "webviewV1": settings.migrations.webview_v1,
        },
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A value with every field different from the default.
    pub fn everything_changed() -> Settings {
        Settings {
            version: 1,
            general: General {
                language: Language::It,
                temperature_unit: TemperatureUnit::F,
                throughput_unit: ThroughputUnit::Bytes,
                interval_ms: 2500,
                chart_fps: ChartFps::Fps15,
                default_view: DefaultView::Advanced,
            },
            tray: Tray {
                close_to_tray: false,
                autostart: true,
                icon_sensor: Some("gpu0/temperature/core".into()),
            },
            updates: Updates {
                check_automatically: true,
            },
            sources: Sources {
                vendor_libraries: VendorLibraries {
                    nvml: false,
                    nvapi: false,
                    adl: false,
                    igcl: false,
                },
                anti_cheat: true,
                service_modules: ServiceModules {
                    cpu: false,
                    motherboard: false,
                    memory: false,
                    storage: false,
                    controller: false,
                    psu: false,
                },
                smart_disabled_drives: vec!["disk/0".into(), "disk/1".into()],
                smart_enabled_drives: vec!["disk/2".into()],
            },
            advanced: AdvancedState {
                section: Some("gpu".into()),
                window: Some(1800),
                series: BTreeMap::from([
                    ("gpu".into(), vec!["gpu0/load/core".into()]),
                    ("cpu".into(), vec![]),
                ]),
            },
            view: ViewState {
                last: Some(ViewKind::Advanced),
            },
            rules: serde_json::from_value(json!({
                "overrides": {
                    "gpu-temp": {"enabled": false, "crit": null},
                    "ram-used": {
                        "warn": {"threshold": {"fixed": 85.0}, "durationS": 5},
                        "hysteresis": {"amount": 1.0, "durationS": 2},
                        "notify": {"warn": true, "crit": false}
                    }
                },
                "custom": [{
                    "id": "custom-00000000-0000-4000-8000-000000000001",
                    "target": {"sensor": "cpu/0/temperature/package"},
                    "unit": "celsius",
                    "condition": "above",
                    "warn": {"threshold": {"fixed": 70.0}, "durationS": 0},
                    "crit": null,
                    "hysteresis": {"amount": 3.0, "durationS": 10},
                    "enabled": true,
                    "notify": {"warn": false, "crit": true}
                }]
            }))
            .expect("valid rules"),
            log: LogSettings {
                folder: Some("D:\\logs".into()),
                sensors: Some(vec![
                    "cpu/0/load/total".into(),
                    "gpu0/temperature/core".into(),
                ]),
                every_ticks: 10,
                max_file_mb: 512,
                hotkey_toggle: Some("Ctrl+Shift+F9".into()),
                hotkey_pause: Some("Ctrl+Alt+P".into()),
            },
            overlay: OverlaySettings {
                enabled: true,
                chart_fps: ChartFps::Fps15,
                text_hz: 4,
                hide_from_capture: true,
                attach: Attach::Monitor,
                track_pc_latency: true,
                track_gpu: true,
                default_profile: "builtin-full".into(),
                game_profiles: BTreeMap::from([
                    (
                        "game.exe".into(),
                        "00000000-0000-4000-8000-000000000002".into(),
                    ),
                    ("other.exe".into(), "builtin-bar".into()),
                ]),
                blocked_games: vec!["launcher.exe".into(), "browser.exe".into()],
                hotkey_toggle: Some("Ctrl+Alt+F1".into()),
                hotkey_next_profile: Some("Ctrl+Alt+F2".into()),
                hotkey_benchmark: Some("Ctrl+Alt+F3".into()),
                editor_bounds: Some(overlay::WindowBounds {
                    x: -1280,
                    y: 40,
                    width: 1400,
                    height: 900,
                }),
            },
            performance: PerformanceSettings {
                thermal_stop: false,
                cpu_stop_c: Some(88),
                gpu_stop_c: 85,
                stop_on_first_error: Some(true),
                ram_share_percent: 40,
                risk_notice_seen: true,
                disk_stop_c: Some(75),
                disk_folder: Some("E:\\Tests".into()),
                community_table: false,
            },
            migrations: Migrations {
                service_v1: true,
                webview_v1: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::everything_changed;
    use super::*;

    #[test]
    fn defaults_match_the_spec() {
        let expected = json!({
            "version": 1,
            "general": {"language": "system", "temperatureUnit": "c", "throughputUnit": "bits",
                        "intervalMs": 1000, "chartFps": 60, "defaultView": "last"},
            "tray": {"closeToTray": true, "autostart": false, "iconSensor": null},
            "updates": {"checkAutomatically": false},
            "sources": {
                "vendorLibraries": {"nvml": true, "nvapi": true, "adl": true, "igcl": true},
                "antiCheat": false,
                "serviceModules": {"cpu": true, "motherboard": true, "memory": true,
                                   "storage": true, "controller": true, "psu": true},
                "smartDisabledDrives": [],
                "smartEnabledDrives": []
            },
            "advanced": {"series": {}},
            "view": {},
            "rules": {"overrides": {}, "custom": []},
            "log": {"folder": null, "sensors": null, "everyTicks": 1, "maxFileMb": 100,
                    "hotkeyToggle": "Ctrl+Alt+Shift+R", "hotkeyPause": null},
            "overlay": {"enabled": false, "chartFps": 30, "textHz": 2, "hideFromCapture": false,
                        "attach": "window", "trackPcLatency": false, "trackGpu": false,
                        "defaultProfile": "builtin-gaming", "gameProfiles": {},
                        "blockedGames": [], "hotkeyToggle": null, "hotkeyNextProfile": null,
                        "hotkeyBenchmark": null, "editorBounds": null},
            "performance": {"thermalStop": true, "cpuStopC": null, "gpuStopC": 90,
                            "stopOnFirstError": null, "ramSharePercent": 70, "riskNoticeSeen": false, "diskStopC": null,
                            "diskFolder": null, "communityTable": true},
            "migrations": {"serviceV1": false, "webviewV1": false}
        });
        assert_eq!(encode(&Settings::default()), expected);
    }

    #[test]
    fn round_trip_is_stable() {
        for settings in [Settings::default(), everything_changed()] {
            let decoded = decode_lenient(&encode(&settings));
            assert_eq!(decoded.settings, settings);
            assert!(decoded.diagnostics.is_empty(), "{:?}", decoded.diagnostics);
            assert_eq!(decoded.version, VersionStatus::Current);
        }
    }

    #[test]
    fn smart_enabled_drives_round_trips() {
        let mut settings = Settings::default();
        settings.sources.smart_enabled_drives = vec!["storage/device-aaa".into()];
        let encoded = encode(&settings);
        assert_eq!(
            encoded["sources"]["smartEnabledDrives"],
            json!(["storage/device-aaa"])
        );
        let decoded = decode_lenient(&encoded);
        assert_eq!(decoded.settings, settings);
        assert!(decoded.diagnostics.is_empty(), "{:?}", decoded.diagnostics);

        let patched = apply_patch(
            &Settings::default(),
            &json!({"sources": {"smartEnabledDrives": ["storage/device-aaa"]}}),
        )
        .unwrap();
        assert_eq!(patched, settings);
        let cleared =
            apply_patch(&patched, &json!({"sources": {"smartEnabledDrives": []}})).unwrap();
        assert!(cleared.sources.smart_enabled_drives.is_empty());
    }

    #[test]
    fn disabled_modules_are_listed_in_declaration_order() {
        assert!(ServiceModules::default().disabled().is_empty());
        let modules = ServiceModules {
            psu: false,
            cpu: false,
            storage: false,
            ..ServiceModules::default()
        };
        assert_eq!(modules.disabled(), vec!["cpu", "storage", "psu"]);
    }
}
