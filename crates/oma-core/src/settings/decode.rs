//! Tolerant decoding of the settings file.

use std::collections::{BTreeMap, HashSet};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

use super::log::{canonical_hotkey, is_absolute_folder, EVERY_TICKS, MAX_FILE_MB, MAX_LOG_SENSORS};
use super::{
    ChartFps, DefaultView, Language, LogSettings, Settings, TemperatureUnit, ThroughputUnit,
    ViewKind, INTERVAL_VALUES, WINDOW_VALUES,
};
use crate::rules::{
    is_builtin, nested, validate_override, validate_rules, CustomRules, Rule, RuleOverride,
    RulesSettings,
};

/// Serialized as `{"path": …, "kind": "invalidRule", "key": …}`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Diagnostic {
    /// camelCase path such as `general.intervalMs`.
    pub path: String,
    #[serde(flatten)]
    pub kind: DiagnosticKind,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DiagnosticKind {
    WrongType,
    UnknownVariant,
    Corrected {
        from: String,
        to: String,
    },
    MissingVersion,
    /// A custom rule or an override field left out of the rules; `key` is the
    /// i18n key of the reason (`settings.error.type` when it does not parse).
    /// The path is the whole rule (`rules.custom.2`) or the override field
    /// (`rules.overrides.gpu-temp.warn`).
    InvalidRule {
        key: &'static str,
    },
}

/// i18n key of a value that does not parse.
pub(super) const TYPE_ERROR: &str = "settings.error.type";

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

    if let Some(rules) = root.get("rules") {
        settings.rules = reader.rules(rules);
    }
    settings.log = reader.log(root);

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

    /// The rules section, keeping every valid part: an invalid custom rule is
    /// left out whole, an invalid override field alone. Overrides for rules
    /// that do not exist (any more) are dropped silently (spec M5 §3.2). The
    /// result always passes [`validate_rules`].
    fn rules(&mut self, value: &Value) -> RulesSettings {
        let mut rules = RulesSettings::default();
        let Value::Object(section) = value else {
            self.push("rules".into(), DiagnosticKind::WrongType);
            return rules;
        };
        match section.get("overrides") {
            None => {}
            Some(Value::Object(overrides)) => {
                for (id, over) in overrides {
                    if let Some(over) = self.rule_override(id, over) {
                        rules.overrides.insert(id.clone(), over);
                    }
                }
            }
            Some(_) => self.push("rules.overrides".into(), DiagnosticKind::WrongType),
        }
        match section.get("custom") {
            None => {}
            Some(Value::Array(items)) => {
                let mut admitted = CustomRules::default();
                for (i, item) in items.iter().enumerate() {
                    let path = format!("rules.custom.{i}");
                    let Ok(rule) = serde_json::from_value::<Rule>(item.clone()) else {
                        self.push(path, DiagnosticKind::InvalidRule { key: TYPE_ERROR });
                        continue;
                    };
                    match admitted.admit(&rule) {
                        Ok(()) => rules.custom.push(rule),
                        Err(e) => self.push(path, DiagnosticKind::InvalidRule { key: e.key }),
                    }
                }
            }
            Some(_) => self.push("rules.custom".into(), DiagnosticKind::WrongType),
        }
        debug_assert_eq!(validate_rules(&rules), Ok(()));
        rules
    }

    /// One override, field by field; `None` for an unknown rule or when
    /// nothing valid is left of a non-object.
    fn rule_override(&mut self, id: &str, value: &Value) -> Option<RuleOverride> {
        let path = format!("rules.overrides.{id}");
        // An unknown id is ignored, whatever it holds.
        if !is_builtin(id) {
            return None;
        }
        let Value::Object(fields) = value else {
            self.push(path, DiagnosticKind::InvalidRule { key: TYPE_ERROR });
            return None;
        };
        // Each field on its own; `null` is a value only for the levels.
        let mut over = RuleOverride {
            enabled: self.override_field(&path, "enabled", fields),
            warn: self.override_field(&path, "warn", fields),
            crit: self.override_field(&path, "crit", fields),
            hysteresis: self.override_field(&path, "hysteresis", fields),
            notify: self.override_field(&path, "notify", fields),
        };

        // Fields valid alone may still make the rule invalid together; drop
        // the fields the error points at until the rule is valid.
        while let Err(e) = validate_override(id, &over) {
            let group = e.key == "rules.error.order" || e.key == "rules.error.noLevel";
            let mut dropped = Vec::new();
            if (group || e.field.starts_with("warn")) && over.warn.take().is_some() {
                dropped.push("warn");
            }
            if (group || e.field.starts_with("crit")) && over.crit.take().is_some() {
                dropped.push("crit");
            }
            if e.field.starts_with("hysteresis") && over.hysteresis.take().is_some() {
                dropped.push("hysteresis");
            }
            if dropped.is_empty() {
                // Not caused by an override field: the built-in rule itself
                // is broken, which its own tests rule out. Keep nothing.
                debug_assert!(false, "built-in rule {id} is invalid: {e:?}");
                return None;
            }
            for name in dropped {
                self.push(
                    nested(&path, name),
                    DiagnosticKind::InvalidRule { key: e.key },
                );
            }
        }
        Some(over)
    }

    /// The override field `name` when present and it parses, else a diagnostic.
    fn override_field<T: DeserializeOwned>(
        &mut self,
        path: &str,
        name: &str,
        fields: &Obj,
    ) -> Option<T> {
        let parsed = serde_json::from_value::<T>(fields.get(name)?.clone());
        if parsed.is_err() {
            self.push(
                nested(path, name),
                DiagnosticKind::InvalidRule { key: TYPE_ERROR },
            );
        }
        parsed.ok()
    }

    /// The `log` section: each field on its own, values outside the rules
    /// brought inside them (spec M5c L11) with a `Corrected` diagnostic.
    fn log(&mut self, root: &Obj) -> LogSettings {
        let section = self.section(root, "", "log");
        let mut log = LogSettings::default();

        match lookup(&section, "folder", true) {
            None => {}
            Some(Value::String(text)) if is_absolute_folder(text) => {
                log.folder = Some(text.clone());
            }
            Some(Value::String(text)) => self.push(
                "log.folder".into(),
                DiagnosticKind::Corrected {
                    from: text.clone(),
                    to: "null".into(),
                },
            ),
            Some(_) => self.push("log.folder".into(), DiagnosticKind::WrongType),
        }

        log.sensors = self.log_sensors(&section);

        if let Some(v) = self.choice(&section, "log", "everyTicks", false, &EVERY_TICKS) {
            log.every_ticks = v;
        }
        match lookup(&section, "maxFileMb", false) {
            None => {}
            Some(Value::Number(n)) => {
                let x = n.as_f64().unwrap_or(f64::from(log.max_file_mb));
                let (min, max) = (*MAX_FILE_MB.start(), *MAX_FILE_MB.end());
                let clamped = x.clamp(f64::from(min), f64::from(max)).round() as u32;
                if x != f64::from(clamped) {
                    self.push(
                        "log.maxFileMb".into(),
                        DiagnosticKind::Corrected {
                            from: n.to_string(),
                            to: clamped.to_string(),
                        },
                    );
                }
                log.max_file_mb = clamped;
            }
            Some(_) => self.push("log.maxFileMb".into(), DiagnosticKind::WrongType),
        }

        (log.hotkey_toggle, _) = self.hotkey(&section, "hotkeyToggle", log.hotkey_toggle.clone());
        let (pause, pause_text) = self.hotkey(&section, "hotkeyPause", None);
        log.hotkey_pause = pause;
        if log.hotkey_pause.is_some() && log.hotkey_pause == log.hotkey_toggle {
            self.push(
                "log.hotkeyPause".into(),
                DiagnosticKind::Corrected {
                    from: pause_text.unwrap_or_default(),
                    to: "null".into(),
                },
            );
            log.hotkey_pause = None;
        }
        log
    }

    /// `log.sensors`: `null` or missing = every sensor; empty and repeated ids
    /// are dropped and the list is cut at [`MAX_LOG_SENSORS`].
    fn log_sensors(&mut self, section: &Obj) -> Option<Vec<String>> {
        let items = match lookup(section, "sensors", true)? {
            Value::Array(items) => items,
            _ => {
                self.push("log.sensors".into(), DiagnosticKind::WrongType);
                return None;
            }
        };
        let Some(ids) = items
            .iter()
            .map(|item| item.as_str())
            .collect::<Option<Vec<&str>>>()
        else {
            self.push("log.sensors".into(), DiagnosticKind::WrongType);
            return None;
        };
        let mut seen = HashSet::new();
        let kept: Vec<String> = ids
            .iter()
            .filter(|id| !id.is_empty() && seen.insert(**id))
            .take(MAX_LOG_SENSORS)
            .map(|id| id.to_string())
            .collect();
        if kept.len() != ids.len() {
            self.push(
                "log.sensors".into(),
                DiagnosticKind::Corrected {
                    from: format!("{} entries", ids.len()),
                    to: format!("{} entries", kept.len()),
                },
            );
        }
        Some(kept)
    }

    /// A hotkey field in canonical form, with the text as written. Missing
    /// keeps `default`, `null` switches it off, unreadable text is dropped
    /// with a diagnostic.
    fn hotkey(
        &mut self,
        section: &Obj,
        key: &str,
        default: Option<String>,
    ) -> (Option<String>, Option<String>) {
        let path = join("log", key);
        match section.get(key) {
            None => (default, None),
            Some(Value::Null) => (None, None),
            Some(Value::String(text)) => match canonical_hotkey(text) {
                Some(canonical) => (Some(canonical), Some(text.clone())),
                None => {
                    self.push(
                        path,
                        DiagnosticKind::Corrected {
                            from: text.clone(),
                            to: "null".into(),
                        },
                    );
                    (None, None)
                }
            },
            Some(_) => {
                self.push(path, DiagnosticKind::WrongType);
                (default, None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::super::test_support::everything_changed;
    use super::super::*;
    use crate::rules::{
        effective_rules, validate_rules, Notify, Rule, RuleOverride, RulesSettings,
    };

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
        let rules = serde_json::to_value(&want.rules).unwrap();
        let d = decode(json!({"version": 1, "rules": rules, "log": encode(&want)["log"]}));
        assert_eq!(d.settings.rules, want.rules);
        assert_eq!(d.settings.log, want.log);
        assert!(d.diagnostics.is_empty(), "{:?}", d.diagnostics);

        let d = decode(json!({"version": 1, "rules": 3, "log": []}));
        assert_eq!(d.settings.rules, Settings::default().rules);
        assert_eq!(d.settings.log, Settings::default().log);
        assert_eq!(d.diagnostics.len(), 2);
    }

    #[test]
    fn log_round_trips() {
        let want = everything_changed();
        let d = decode(encode(&want));
        assert_eq!(d.settings.log, want.log);
        assert!(d.diagnostics.is_empty(), "{:?}", d.diagnostics);
        // Missing keys take the defaults; an explicit null switches a hotkey off.
        let d = decode(json!({"version": 1, "log": {}}));
        assert_eq!(d.settings.log, LogSettings::default());
        assert!(d.diagnostics.is_empty());
        let d = decode(json!({"version": 1, "log": {"hotkeyToggle": null}}));
        assert_eq!(d.settings.log.hotkey_toggle, None);
        assert!(d.diagnostics.is_empty());
        // A readable hotkey is stored canonical, without a diagnostic.
        let d = decode(json!({"version": 1, "log": {"hotkeyToggle": "shift + alt + p"}}));
        assert_eq!(d.settings.log.hotkey_toggle.as_deref(), Some("Alt+Shift+P"));
        assert!(d.diagnostics.is_empty());
    }

    #[test]
    fn lenient_log_corrects_hand_edits() {
        let d = decode(json!({"version": 1, "log": {
            "everyTicks": 7,
            "maxFileMb": 5,
            "folder": "logs",
            "hotkeyToggle": "Ctrl+Alt+R",
            "hotkeyPause": "alt+ctrl+r",
            "sensors": ["a", "", "a"]
        }}));
        let log = &d.settings.log;
        assert_eq!(log.every_ticks, 5);
        assert_eq!(log.max_file_mb, 10);
        assert_eq!(log.folder, None);
        assert_eq!(log.hotkey_toggle.as_deref(), Some("Ctrl+Alt+R"));
        assert_eq!(log.hotkey_pause, None);
        assert_eq!(log.sensors, Some(vec!["a".to_string()]));
        assert_eq!(
            d.diagnostics,
            vec![
                corrected("log.folder", "logs", "null"),
                corrected("log.sensors", "3 entries", "1 entries"),
                corrected("log.everyTicks", "7", "5"),
                corrected("log.maxFileMb", "5", "10"),
                corrected("log.hotkeyPause", "alt+ctrl+r", "null"),
            ]
        );

        let d = decode(json!({"version": 1, "log": {
            "maxFileMb": 5000, "everyTicks": 100, "hotkeyToggle": "Ctrl+R", "hotkeyPause": "nonsense"
        }}));
        assert_eq!(d.settings.log.max_file_mb, 2048);
        assert_eq!(d.settings.log.every_ticks, 60);
        assert_eq!(d.settings.log.hotkey_toggle, None);
        assert_eq!(d.settings.log.hotkey_pause, None);
        assert_eq!(d.diagnostics.len(), 4);

        let many: Vec<String> = (0..4100).map(|i| format!("s{i}")).collect();
        let d = decode(json!({"version": 1, "log": {"sensors": many}}));
        assert_eq!(d.settings.log.sensors.as_ref().map(Vec::len), Some(4096));
        assert_eq!(d.diagnostics.len(), 1);

        // A value of the wrong type falls back to the default, with a diagnostic.
        let d = decode(json!({"version": 1, "log": {
            "everyTicks": "1", "folder": 3, "sensors": [1], "hotkeyToggle": true, "maxFileMb": null
        }}));
        assert_eq!(d.settings.log, LogSettings::default());
        assert_eq!(d.diagnostics.len(), 5);
        assert!(d
            .diagnostics
            .iter()
            .all(|x| x.kind == DiagnosticKind::WrongType));
    }

    const ID_A: &str = "custom-00000000-0000-4000-8000-00000000000a";
    const ID_B: &str = "custom-00000000-0000-4000-8000-00000000000b";

    /// A custom rule on a CPU temperature with only a warning level.
    fn custom_rule(id: &str, threshold: serde_json::Value) -> serde_json::Value {
        json!({
            "id": id,
            "target": {"sensor": "cpu/0/temperature/package"},
            "unit": "celsius",
            "condition": "above",
            "warn": {"threshold": threshold, "durationS": 0},
            "crit": null
        })
    }

    fn invalid(path: &str, key: &'static str) -> Diagnostic {
        Diagnostic {
            path: path.into(),
            kind: DiagnosticKind::InvalidRule { key },
        }
    }

    #[test]
    fn invalid_rules_are_excluded_and_reported() {
        let first_a = custom_rule(ID_A, json!({"fixed": 70}));
        let d = decode(json!({
            "version": 1,
            "rules": {
                "overrides": {
                    "gpu-temp": {
                        "warn": {"threshold": {"fixed": "x"}, "durationS": 30},
                        "enabled": false
                    }
                },
                "custom": [
                    first_a,
                    custom_rule(ID_B, json!("alta")),
                    custom_rule(ID_A, json!({"fixed": 75}))
                ]
            }
        }));
        let rules = &d.settings.rules;
        assert_eq!(
            rules.custom,
            vec![serde_json::from_value::<Rule>(first_a).unwrap()]
        );
        assert_eq!(
            rules.overrides,
            BTreeMap::from([(
                "gpu-temp".to_string(),
                RuleOverride {
                    enabled: Some(false),
                    ..RuleOverride::default()
                }
            )])
        );
        assert_eq!(
            d.diagnostics,
            vec![
                invalid("rules.overrides.gpu-temp.warn", "settings.error.type"),
                invalid("rules.custom.1", "settings.error.type"),
                invalid("rules.custom.2", "rules.error.duplicateId"),
            ]
        );
        assert_eq!(validate_rules(rules), Ok(()));
    }

    #[test]
    fn a_duplicate_id_keeps_the_first_valid_rule() {
        // The first rule with the id is invalid, so the second one is kept.
        let mut broken = custom_rule(ID_A, json!({"fixed": 70}));
        broken["warn"]["durationS"] = json!(601);
        let d = decode(json!({"version": 1, "rules": {"custom": [
            broken,
            custom_rule(ID_A, json!({"fixed": 75})),
        ]}}));
        assert_eq!(d.settings.rules.custom.len(), 1);
        assert_eq!(
            d.settings.rules.custom[0].warn.as_ref().unwrap().duration_s,
            0
        );
        assert_eq!(
            d.diagnostics,
            vec![invalid("rules.custom.0", "rules.error.duration")]
        );
    }

    #[test]
    fn cross_field_invalid_overrides_restore_default_levels() {
        let d = decode(json!({"version": 1, "rules": {"overrides": {
            // Warning 95 above the default critical 90.
            "gpu-temp": {
                "warn": {"threshold": {"fixed": 95}, "durationS": 30},
                "enabled": false,
                "notify": {"warn": true, "crit": true}
            },
            // Both levels off.
            "gpu-hotspot": {"warn": null, "crit": null, "enabled": false}
        }}}));
        let overrides = &d.settings.rules.overrides;
        assert_eq!(
            overrides["gpu-temp"],
            RuleOverride {
                enabled: Some(false),
                notify: Some(Notify {
                    warn: true,
                    crit: true
                }),
                ..RuleOverride::default()
            }
        );
        assert_eq!(
            overrides["gpu-hotspot"],
            RuleOverride {
                enabled: Some(false),
                ..RuleOverride::default()
            }
        );
        let effective = effective_rules(&d.settings.rules);
        let defaults = crate::rules::default_rules();
        for id in ["gpu-temp", "gpu-hotspot"] {
            let rule = effective.iter().find(|r| r.id == id).unwrap();
            let default = defaults.iter().find(|r| r.id == id).unwrap();
            assert_eq!((&rule.warn, &rule.crit), (&default.warn, &default.crit));
            assert!(!rule.enabled);
        }
        // Map order is not part of the contract.
        let mut diagnostics = d.diagnostics.clone();
        diagnostics.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            diagnostics,
            vec![
                invalid("rules.overrides.gpu-hotspot.crit", "rules.error.noLevel"),
                invalid("rules.overrides.gpu-hotspot.warn", "rules.error.noLevel"),
                invalid("rules.overrides.gpu-temp.warn", "rules.error.order"),
            ]
        );
        assert_eq!(validate_rules(&d.settings.rules), Ok(()));
    }

    #[test]
    fn single_invalid_override_fields_are_dropped_alone() {
        let d = decode(json!({"version": 1, "rules": {"overrides": {
            "gpu-temp": {
                "crit": {"threshold": {"fixed": 99}, "durationS": 601},
                "hysteresis": {"amount": -1, "durationS": 0},
                "notify": "loud",
                "enabled": false
            },
            "ram-used": 7
        }}}));
        assert_eq!(
            d.settings.rules.overrides,
            BTreeMap::from([(
                "gpu-temp".to_string(),
                RuleOverride {
                    enabled: Some(false),
                    ..RuleOverride::default()
                }
            )])
        );
        assert_eq!(
            d.diagnostics,
            vec![
                invalid("rules.overrides.gpu-temp.notify", "settings.error.type"),
                invalid("rules.overrides.gpu-temp.crit", "rules.error.duration"),
                invalid(
                    "rules.overrides.gpu-temp.hysteresis",
                    "rules.error.hysteresis"
                ),
                invalid("rules.overrides.ram-used", "settings.error.type"),
            ]
        );
    }

    #[test]
    fn overrides_for_unknown_rules_are_dropped_silently() {
        let d = decode(json!({"version": 1, "rules": {"overrides": {
            "no-such-rule": {"enabled": false},
            "gpu-temp": {"enabled": false}
        }}}));
        assert_eq!(
            d.settings.rules.overrides.keys().collect::<Vec<_>>(),
            ["gpu-temp"]
        );
        assert!(d.diagnostics.is_empty(), "{:?}", d.diagnostics);
        assert_eq!(validate_rules(&d.settings.rules), Ok(()));
    }

    #[test]
    fn too_many_custom_rules_are_truncated_on_load() {
        let id = |n: usize| format!("custom-00000000-0000-4000-8000-{n:012x}");
        let mut custom = vec![custom_rule(&id(9999), json!({"fixed": f64::NAN}))];
        // `json!` turns NaN into null: the first rule is invalid.
        custom.extend((0..300).map(|n| custom_rule(&id(n), json!({"fixed": 70}))));
        let d = decode(json!({"version": 1, "rules": {"custom": custom}}));
        let kept = &d.settings.rules.custom;
        assert_eq!(kept.len(), crate::rules::MAX_CUSTOM_RULES);
        assert_eq!(kept[0].id, id(0));
        assert_eq!(kept.last().unwrap().id, id(255));
        let mut want = vec![invalid("rules.custom.0", "settings.error.type")];
        want.extend(
            (257..=300).map(|i| invalid(&format!("rules.custom.{i}"), "rules.error.tooMany")),
        );
        assert_eq!(d.diagnostics, want);
        assert_eq!(validate_rules(&d.settings.rules), Ok(()));
    }

    #[test]
    fn wrong_rules_containers_fall_back() {
        let d = decode(json!({"version": 1, "rules": {"overrides": [], "custom": {}}}));
        assert_eq!(d.settings.rules, RulesSettings::default());
        assert_eq!(
            d.diagnostics,
            vec![
                Diagnostic {
                    path: "rules.overrides".into(),
                    kind: DiagnosticKind::WrongType
                },
                Diagnostic {
                    path: "rules.custom".into(),
                    kind: DiagnosticKind::WrongType
                },
            ]
        );
    }

    #[test]
    fn diagnostics_serialize_with_a_kind_tag() {
        let value = serde_json::to_value(vec![
            invalid("rules.custom.1", "settings.error.type"),
            corrected("general.intervalMs", "700", "500"),
            Diagnostic {
                path: "version".into(),
                kind: DiagnosticKind::MissingVersion,
            },
        ])
        .unwrap();
        assert_eq!(
            value,
            json!([
                {"path": "rules.custom.1", "kind": "invalidRule", "key": "settings.error.type"},
                {"path": "general.intervalMs", "kind": "corrected", "from": "700", "to": "500"},
                {"path": "version", "kind": "missingVersion"}
            ])
        );
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
