//! Tolerant decoding of the settings file.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::{
    ChartFps, DefaultView, Language, Settings, TemperatureUnit, ThroughputUnit, ViewKind,
    INTERVAL_VALUES, WINDOW_VALUES,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    /// camelCase path such as `general.intervalMs`.
    pub path: String,
    pub kind: DiagnosticKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DiagnosticKind {
    WrongType,
    UnknownVariant,
    Corrected { from: String, to: String },
    MissingVersion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionStatus {
    Current,
    Future(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub settings: Settings,
    pub version: VersionStatus,
    pub diagnostics: Vec<Diagnostic>,
}

type Obj = Map<String, Value>;

/// Decodes a settings document field by field, so a broken field never drags
/// the rest along. Missing keys take defaults silently, unknown keys are
/// ignored; every other deviation produces a [`Diagnostic`].
pub fn decode_lenient(value: &Value) -> Decoded {
    let mut reader = Reader::default();
    let mut settings = Settings::default();
    let mut version = VersionStatus::Current;

    let Some(root) = value.as_object() else {
        reader.push(String::new(), DiagnosticKind::WrongType);
        return Decoded {
            settings,
            version,
            diagnostics: reader.diagnostics,
        };
    };

    match root.get("version") {
        None => reader.push("version".into(), DiagnosticKind::MissingVersion),
        Some(v) => match v.as_u64() {
            Some(1) => {}
            Some(n) if n > 1 => {
                version = VersionStatus::Future(u32::try_from(n).unwrap_or(u32::MAX))
            }
            _ => reader.push("version".into(), DiagnosticKind::WrongType),
        },
    }

    let general = reader.section(root, "", "general");
    let g = &mut settings.general;
    if let Some(v) = reader.variant(&general, "general", "language", false, Language::parse) {
        g.language = v;
    }
    if let Some(v) = reader.variant(
        &general,
        "general",
        "temperatureUnit",
        false,
        TemperatureUnit::parse,
    ) {
        g.temperature_unit = v;
    }
    if let Some(v) = reader.variant(
        &general,
        "general",
        "throughputUnit",
        false,
        ThroughputUnit::parse,
    ) {
        g.throughput_unit = v;
    }
    if let Some(v) = reader.choice(&general, "general", "intervalMs", false, &INTERVAL_VALUES) {
        g.interval_ms = v;
    }
    if let Some(v) = reader.choice(&general, "general", "chartFps", false, &ChartFps::VALUES) {
        g.chart_fps = ChartFps::from_u32(v).unwrap_or_default();
    }
    if let Some(v) = reader.variant(
        &general,
        "general",
        "defaultView",
        false,
        DefaultView::parse,
    ) {
        g.default_view = v;
    }

    let tray = reader.section(root, "", "tray");
    let t = &mut settings.tray;
    t.close_to_tray = reader.boolean(&tray, "tray", "closeToTray", t.close_to_tray);
    t.autostart = reader.boolean(&tray, "tray", "autostart", t.autostart);
    t.icon_sensor = reader.text(&tray, "tray", "iconSensor");

    let sources = reader.section(root, "", "sources");
    let vendors = reader.section(&sources, "sources", "vendorLibraries");
    let v = &mut settings.sources.vendor_libraries;
    let path = "sources.vendorLibraries";
    v.nvml = reader.boolean(&vendors, path, "nvml", v.nvml);
    v.nvapi = reader.boolean(&vendors, path, "nvapi", v.nvapi);
    v.adl = reader.boolean(&vendors, path, "adl", v.adl);
    v.igcl = reader.boolean(&vendors, path, "igcl", v.igcl);
    settings.sources.anti_cheat = reader.boolean(&sources, "sources", "antiCheat", false);
    let modules = reader.section(&sources, "sources", "serviceModules");
    let m = &mut settings.sources.service_modules;
    let path = "sources.serviceModules";
    m.cpu = reader.boolean(&modules, path, "cpu", m.cpu);
    m.motherboard = reader.boolean(&modules, path, "motherboard", m.motherboard);
    m.memory = reader.boolean(&modules, path, "memory", m.memory);
    m.storage = reader.boolean(&modules, path, "storage", m.storage);
    m.controller = reader.boolean(&modules, path, "controller", m.controller);
    m.psu = reader.boolean(&modules, path, "psu", m.psu);
    if let Some(list) = reader.string_list(&sources, "sources", "smartDisabledDrives") {
        settings.sources.smart_disabled_drives = list;
    }

    let advanced = reader.section(root, "", "advanced");
    settings.advanced.section = reader.text(&advanced, "advanced", "section");
    settings.advanced.window = reader.choice(&advanced, "advanced", "window", true, &WINDOW_VALUES);
    settings.advanced.series = reader.series(&advanced);

    let view = reader.section(root, "", "view");
    settings.view.last = reader.variant(&view, "view", "last", true, ViewKind::parse);

    if let Some(rules) = reader.opaque_object(root, "rules") {
        settings.rules = rules;
    }
    if let Some(log) = reader.opaque_object(root, "log") {
        settings.log = log;
    }

    let migrations = reader.section(root, "", "migrations");
    let m = &mut settings.migrations;
    m.service_v1 = reader.boolean(&migrations, "migrations", "serviceV1", false);
    m.webview_v1 = reader.boolean(&migrations, "migrations", "webviewV1", false);

    Decoded {
        settings,
        version,
        diagnostics: reader.diagnostics,
    }
}

#[derive(Default)]
struct Reader {
    diagnostics: Vec<Diagnostic>,
}

fn join(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

/// The value under `key`, or `None` when it is missing (or `null` and the
/// field is nullable).
fn lookup<'a>(obj: &'a Obj, key: &str, nullable: bool) -> Option<&'a Value> {
    match obj.get(key) {
        None => None,
        Some(Value::Null) if nullable => None,
        Some(v) => Some(v),
    }
}

/// Nearest accepted value; on a tie the larger one wins.
fn snap(x: f64, valid: &[u32]) -> u32 {
    let mut best = valid[0];
    let mut best_distance = f64::INFINITY;
    for &candidate in valid {
        let distance = (x - f64::from(candidate)).abs();
        if distance <= best_distance {
            best = candidate;
            best_distance = distance;
        }
    }
    best
}

impl Reader {
    fn push(&mut self, path: String, kind: DiagnosticKind) {
        self.diagnostics.push(Diagnostic { path, kind });
    }

    /// A nested object; missing means all-defaults, any other type is a
    /// diagnostic and also means all-defaults.
    fn section(&mut self, parent: &Obj, parent_path: &str, key: &str) -> Obj {
        match parent.get(key) {
            None => Obj::new(),
            Some(Value::Object(map)) => map.clone(),
            Some(_) => {
                self.push(join(parent_path, key), DiagnosticKind::WrongType);
                Obj::new()
            }
        }
    }

    fn boolean(&mut self, obj: &Obj, parent: &str, key: &str, default: bool) -> bool {
        match lookup(obj, key, false) {
            None => default,
            Some(Value::Bool(b)) => *b,
            Some(_) => {
                self.push(join(parent, key), DiagnosticKind::WrongType);
                default
            }
        }
    }

    /// A string enum; `None` when missing, `null` or invalid (the caller keeps
    /// its default, which for nullable fields is `None`).
    fn variant<T>(
        &mut self,
        obj: &Obj,
        parent: &str,
        key: &str,
        nullable: bool,
        parse: fn(&str) -> Option<T>,
    ) -> Option<T> {
        match lookup(obj, key, nullable)? {
            Value::String(text) => {
                let parsed = parse(text);
                if parsed.is_none() {
                    self.push(join(parent, key), DiagnosticKind::UnknownVariant);
                }
                parsed
            }
            _ => {
                self.push(join(parent, key), DiagnosticKind::WrongType);
                None
            }
        }
    }

    /// A number restricted to `valid`; off-set numbers are snapped with a
    /// `Corrected` diagnostic.
    fn choice(
        &mut self,
        obj: &Obj,
        parent: &str,
        key: &str,
        nullable: bool,
        valid: &[u32],
    ) -> Option<u32> {
        match lookup(obj, key, nullable)? {
            Value::Number(n) => {
                let x = n.as_f64()?;
                let snapped = snap(x, valid);
                if x != f64::from(snapped) {
                    self.push(
                        join(parent, key),
                        DiagnosticKind::Corrected {
                            from: n.to_string(),
                            to: snapped.to_string(),
                        },
                    );
                }
                Some(snapped)
            }
            _ => {
                self.push(join(parent, key), DiagnosticKind::WrongType);
                None
            }
        }
    }

    /// An optional string (`null` and missing both mean unset).
    fn text(&mut self, obj: &Obj, parent: &str, key: &str) -> Option<String> {
        match lookup(obj, key, true)? {
            Value::String(s) => Some(s.clone()),
            _ => {
                self.push(join(parent, key), DiagnosticKind::WrongType);
                None
            }
        }
    }

    fn string_list(&mut self, obj: &Obj, parent: &str, key: &str) -> Option<Vec<String>> {
        let items = match lookup(obj, key, false)? {
            Value::Array(items) => items,
            _ => {
                self.push(join(parent, key), DiagnosticKind::WrongType);
                return None;
            }
        };
        let strings: Option<Vec<String>> = items
            .iter()
            .map(|item| item.as_str().map(str::to_string))
            .collect();
        if strings.is_none() {
            self.push(join(parent, key), DiagnosticKind::WrongType);
        }
        strings
    }

    /// `advanced.series`: any key maps to a list of strings; malformed entries
    /// are dropped one by one.
    fn series(&mut self, advanced: &Obj) -> BTreeMap<String, Vec<String>> {
        let path = "advanced.series";
        let mut series = BTreeMap::new();
        match advanced.get("series") {
            None => {}
            Some(Value::Object(map)) => {
                for (id, value) in map {
                    let list: Option<Vec<String>> = value.as_array().and_then(|items| {
                        items
                            .iter()
                            .map(|item| item.as_str().map(str::to_string))
                            .collect()
                    });
                    match list {
                        Some(list) => {
                            series.insert(id.clone(), list);
                        }
                        None => self.push(path.into(), DiagnosticKind::WrongType),
                    }
                }
            }
            Some(_) => self.push(path.into(), DiagnosticKind::WrongType),
        }
        series
    }

    /// `rules` / `log`: kept verbatim when objects.
    fn opaque_object(&mut self, root: &Obj, key: &str) -> Option<Value> {
        match root.get(key)? {
            v @ Value::Object(_) => Some(v.clone()),
            _ => {
                self.push(key.into(), DiagnosticKind::WrongType);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::test_support::everything_changed;
    use super::super::*;

    fn decode(value: serde_json::Value) -> Decoded {
        decode_lenient(&value)
    }

    fn corrected(path: &str, from: &str, to: &str) -> Diagnostic {
        Diagnostic {
            path: path.into(),
            kind: DiagnosticKind::Corrected {
                from: from.into(),
                to: to.into(),
            },
        }
    }

    #[test]
    fn missing_keys_take_defaults_silently() {
        let d = decode(json!({"version": 1}));
        assert_eq!(d.settings, Settings::default());
        assert!(d.diagnostics.is_empty());
        assert_eq!(d.version, VersionStatus::Current);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let d = decode(json!({"version": 1, "foo": 1, "general": {"bar": true}}));
        assert_eq!(d.settings, Settings::default());
        assert!(d.diagnostics.is_empty());
    }

    #[test]
    fn interval_is_snapped_to_the_nearest_step() {
        for (given, want) in [(700, 500), (800, 1000), (750, 1000), (9000, 5000), (0, 500)] {
            let d = decode(json!({"version": 1, "general": {"intervalMs": given}}));
            assert_eq!(d.settings.general.interval_ms, want, "from {given}");
            assert_eq!(
                d.diagnostics,
                vec![corrected(
                    "general.intervalMs",
                    &given.to_string(),
                    &want.to_string()
                )]
            );
        }
        let d = decode(json!({"version": 1, "general": {"intervalMs": 3500}}));
        assert_eq!(d.settings.general.interval_ms, 3500);
        assert!(d.diagnostics.is_empty());
    }

    #[test]
    fn fps_and_window_are_snapped() {
        let d = decode(json!({"version": 1, "general": {"chartFps": 45}}));
        assert_eq!(d.settings.general.chart_fps, ChartFps::Fps60);
        assert_eq!(
            d.diagnostics,
            vec![corrected("general.chartFps", "45", "60")]
        );

        let d = decode(json!({"version": 1, "general": {"chartFps": 20}}));
        assert_eq!(d.settings.general.chart_fps, ChartFps::Fps15);

        let d = decode(json!({"version": 1, "advanced": {"window": 100}}));
        assert_eq!(d.settings.advanced.window, Some(60));
        assert_eq!(
            d.diagnostics,
            vec![corrected("advanced.window", "100", "60")]
        );

        let d = decode(json!({"version": 1, "advanced": {"window": 1000}}));
        assert_eq!(d.settings.advanced.window, Some(300));

        let d = decode(json!({"version": 1, "advanced": {"window": 1800}}));
        assert_eq!(d.settings.advanced.window, Some(1800));
        assert!(d.diagnostics.is_empty());
    }

    #[test]
    fn wrong_types_fall_back_per_field() {
        let d = decode(json!({
            "version": 1,
            "general": {"intervalMs": "fast", "language": "it"},
            "tray": {"closeToTray": "yes", "autostart": true}
        }));
        assert_eq!(d.settings.general.interval_ms, 1000);
        assert!(d.settings.tray.close_to_tray);
        assert_eq!(d.settings.general.language, Language::It);
        assert!(d.settings.tray.autostart);
        assert_eq!(
            d.diagnostics,
            vec![
                Diagnostic {
                    path: "general.intervalMs".into(),
                    kind: DiagnosticKind::WrongType
                },
                Diagnostic {
                    path: "tray.closeToTray".into(),
                    kind: DiagnosticKind::WrongType
                },
            ]
        );
    }

    #[test]
    fn wrong_section_types_fall_back_as_a_whole() {
        let d =
            decode(json!({"version": 1, "general": 5, "sources": {"smartDisabledDrives": [1]}}));
        assert_eq!(d.settings, Settings::default());
        let paths: Vec<_> = d.diagnostics.iter().map(|x| x.path.as_str()).collect();
        assert_eq!(paths, vec!["general", "sources.smartDisabledDrives"]);
        assert!(d
            .diagnostics
            .iter()
            .all(|x| x.kind == DiagnosticKind::WrongType));
    }

    #[test]
    fn unknown_enum_variant_falls_back() {
        let d = decode(json!({"version": 1, "general": {"language": "de"}}));
        assert_eq!(d.settings.general.language, Language::System);
        assert_eq!(
            d.diagnostics,
            vec![Diagnostic {
                path: "general.language".into(),
                kind: DiagnosticKind::UnknownVariant
            }]
        );
    }

    #[test]
    fn future_version_decodes_what_it_can() {
        let d = decode(json!({"version": 7, "general": {"intervalMs": 2000}}));
        assert_eq!(d.version, VersionStatus::Future(7));
        assert_eq!(d.settings.general.interval_ms, 2000);
    }

    #[test]
    fn missing_version_is_treated_as_1() {
        let d = decode(json!({"general": {"intervalMs": 2000}}));
        assert_eq!(d.version, VersionStatus::Current);
        assert_eq!(d.settings.version, 1);
        assert_eq!(d.settings.general.interval_ms, 2000);
        assert_eq!(
            d.diagnostics,
            vec![Diagnostic {
                path: "version".into(),
                kind: DiagnosticKind::MissingVersion
            }]
        );
    }

    #[test]
    fn non_object_root_yields_defaults() {
        let d = decode(json!([1, 2]));
        assert_eq!(d.settings, Settings::default());
        assert_eq!(d.diagnostics[0].kind, DiagnosticKind::WrongType);
    }

    #[test]
    fn rules_and_log_are_preserved_when_objects() {
        let want = everything_changed();
        let d = decode(json!({"version": 1, "rules": want.rules, "log": want.log}));
        assert_eq!(d.settings.rules, want.rules);
        assert_eq!(d.settings.log, want.log);

        let d = decode(json!({"version": 1, "rules": 3, "log": []}));
        assert_eq!(d.settings.rules, Settings::default().rules);
        assert_eq!(d.settings.log, Settings::default().log);
        assert_eq!(d.diagnostics.len(), 2);
    }

    #[test]
    fn advanced_and_view_distinguish_absent_from_set() {
        let d = decode(json!({
            "version": 1,
            "advanced": {"section": null, "series": {"a": ["x"], "b": 3}},
            "view": {"last": "advanced"}
        }));
        assert_eq!(d.settings.advanced.section, None);
        assert_eq!(d.settings.advanced.window, None);
        assert_eq!(d.settings.advanced.series.len(), 1);
        assert_eq!(d.settings.view.last, Some(ViewKind::Advanced));
        assert_eq!(d.diagnostics.len(), 1);
    }
}
