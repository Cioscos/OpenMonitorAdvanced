//! Strict validation and merging of settings patches.

use serde_json::{Map, Value};

use super::decode::{decode_lenient, Diagnostic, DiagnosticKind, TYPE_ERROR};
use super::{encode, Settings};
use crate::rules::{is_builtin, validate_rules, RulesSettings};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchError {
    /// camelCase path of the first failing field.
    pub field: String,
    /// i18n key such as `settings.error.range`.
    pub key: &'static str,
}

impl PatchError {
    fn new(field: impl Into<String>, key: &'static str) -> Self {
        Self {
            field: field.into(),
            key,
        }
    }
}

/// Shape of the patchable part of the settings, used to reject unknown keys
/// and `null` on non-nullable fields before the real validation.
enum Node {
    Leaf {
        nullable: bool,
    },
    Object(&'static [(&'static str, Node)]),
    /// Free-form keys, each holding an object of this shape.
    Map(&'static [(&'static str, Node)]),
    /// Free-form map: its keys are never unknown.
    Free,
}

const fn leaf() -> Node {
    Node::Leaf { nullable: false }
}

const fn nullable() -> Node {
    Node::Leaf { nullable: true }
}

/// Sections that a patch may never touch.
const READ_ONLY: [&str; 3] = ["version", "migrations", "log"];

const SCHEMA: &[(&str, Node)] = &[
    (
        "general",
        Node::Object(&[
            ("language", leaf()),
            ("temperatureUnit", leaf()),
            ("throughputUnit", leaf()),
            ("intervalMs", leaf()),
            ("chartFps", leaf()),
            ("defaultView", leaf()),
        ]),
    ),
    (
        "tray",
        Node::Object(&[
            ("closeToTray", leaf()),
            ("autostart", leaf()),
            ("iconSensor", nullable()),
        ]),
    ),
    (
        "sources",
        Node::Object(&[
            (
                "vendorLibraries",
                Node::Object(&[
                    ("nvml", leaf()),
                    ("nvapi", leaf()),
                    ("adl", leaf()),
                    ("igcl", leaf()),
                ]),
            ),
            ("antiCheat", leaf()),
            (
                "serviceModules",
                Node::Object(&[
                    ("cpu", leaf()),
                    ("motherboard", leaf()),
                    ("memory", leaf()),
                    ("storage", leaf()),
                    ("controller", leaf()),
                    ("psu", leaf()),
                ]),
            ),
            ("smartDisabledDrives", leaf()),
        ]),
    ),
    (
        "advanced",
        Node::Object(&[
            ("section", nullable()),
            ("window", nullable()),
            ("series", Node::Free),
        ]),
    ),
    ("view", Node::Object(&[("last", nullable())])),
    (
        "rules",
        Node::Object(&[
            (
                "overrides",
                Node::Map(&[
                    ("enabled", leaf()),
                    ("warn", nullable()),
                    ("crit", nullable()),
                    ("hysteresis", leaf()),
                    ("notify", leaf()),
                ]),
            ),
            ("custom", leaf()),
        ]),
    ),
];

/// Whether the object at `path` is replaced whole instead of merged: the
/// fields of a rule override (R5).
fn replaced_whole(path: &[String]) -> bool {
    matches!(path, [rules, overrides, _, _] if rules == "rules" && overrides == "overrides")
}

fn join(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

/// Rejects unknown keys and `null` on non-nullable fields.
fn check_shape(
    patch: &Map<String, Value>,
    schema: &[(&str, Node)],
    parent: &str,
) -> Result<(), PatchError> {
    for (key, value) in patch {
        let field = join(parent, key);
        let Some((_, node)) = schema.iter().find(|(name, _)| name == key) else {
            return Err(PatchError::new(field, "settings.error.unknownField"));
        };
        match node {
            Node::Leaf { nullable } => {
                if value.is_null() && !nullable {
                    return Err(PatchError::new(field, "settings.error.null"));
                }
            }
            Node::Free => {
                if value.is_null() {
                    return Err(PatchError::new(field, "settings.error.null"));
                }
            }
            Node::Object(children) => match value {
                Value::Null => return Err(PatchError::new(field, "settings.error.null")),
                Value::Object(map) => check_shape(map, children, &field)?,
                // A non-object here is caught as a wrong type by the decoder.
                _ => {}
            },
            Node::Map(children) => match value {
                Value::Null => return Err(PatchError::new(field, "settings.error.null")),
                Value::Object(map) => {
                    for (entry, value) in map {
                        let entry_field = join(&field, entry);
                        match value {
                            Value::Null => {
                                return Err(PatchError::new(entry_field, "settings.error.null"))
                            }
                            Value::Object(fields) => check_shape(fields, children, &entry_field)?,
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
        }
    }
    Ok(())
}

/// Objects merge recursively, everything else (arrays included) replaces, and
/// so do the fields of a rule override. `path` is where `base` sits.
fn merge(base: &mut Value, patch: &Value, path: &mut Vec<String>) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) if !replaced_whole(path) => {
            for (key, value) in patch {
                match base.get_mut(key) {
                    Some(existing) => {
                        path.push(key.clone());
                        merge(existing, value, path);
                        path.pop();
                    }
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, patch) => *base = patch.clone(),
    }
}

/// The rules section of a merged document, decoded strictly: the first value
/// that does not parse, or the first rule that fails [`validate_rules`], is the
/// error, with its full path.
fn strict_rules(merged: &Value, diagnostics: &[Diagnostic]) -> Result<RulesSettings, PatchError> {
    let section = merged.get("rules").cloned().unwrap_or(Value::Null);
    if let Some(overrides) = section.get("overrides").and_then(Value::as_object) {
        // The tolerant decoder drops these silently; a patch may not add one.
        if let Some(id) = overrides.keys().find(|id| !is_builtin(id)) {
            return Err(PatchError::new(
                format!("rules.overrides.{id}"),
                "rules.error.unknownRule",
            ));
        }
    }
    let rules = serde_json::from_value::<RulesSettings>(section).map_err(|_| {
        // The tolerant decoder located the value that does not parse.
        let path = diagnostics
            .iter()
            .find(|d| d.kind == (DiagnosticKind::InvalidRule { key: TYPE_ERROR }))
            .map_or("rules", |d| d.path.as_str());
        PatchError::new(path, TYPE_ERROR)
    })?;
    validate_rules(&rules).map_err(|(field, key)| PatchError::new(field, key))?;
    Ok(rules)
}

/// Applies `patch` to `current` and validates the whole result strictly: any
/// value the tolerant decoder would have had to fix is an error instead.
/// Pure: on error nothing is changed.
pub fn apply_patch(current: &Settings, patch: &Value) -> Result<Settings, PatchError> {
    let Some(patch_map) = patch.as_object() else {
        return Err(PatchError::new("", "settings.error.notObject"));
    };
    if let Some(key) = READ_ONLY.iter().find(|key| patch_map.contains_key(**key)) {
        return Err(PatchError::new(*key, "settings.error.readOnlyField"));
    }
    check_shape(patch_map, SCHEMA, "")?;

    let mut merged = encode(current);
    merge(&mut merged, patch, &mut Vec::new());
    let decoded = decode_lenient(&merged);
    // Rules are checked strictly below, with their full paths.
    let other = decoded
        .diagnostics
        .iter()
        .find(|d| !matches!(d.kind, DiagnosticKind::InvalidRule { .. }));
    if let Some(diagnostic) = other {
        let key = match diagnostic.kind {
            DiagnosticKind::Corrected { .. } => "settings.error.range",
            DiagnosticKind::WrongType
            | DiagnosticKind::UnknownVariant
            | DiagnosticKind::MissingVersion
            | DiagnosticKind::InvalidRule { .. } => TYPE_ERROR,
        };
        return Err(PatchError::new(diagnostic.path.clone(), key));
    }
    let rules = strict_rules(&merged, &decoded.diagnostics)?;
    Ok(Settings {
        rules,
        ..decoded.settings
    })
}

/// `current` without the override of the built-in rule `rule_id` ("Restore"
/// in the rules table); no override is not an error, an id that is not a
/// built-in rule is.
pub fn reset_rule_override(current: &Settings, rule_id: &str) -> Result<Settings, PatchError> {
    if !is_builtin(rule_id) {
        return Err(PatchError::new(
            format!("rules.overrides.{rule_id}"),
            "rules.error.unknownRule",
        ));
    }
    let mut next = current.clone();
    next.rules.overrides.remove(rule_id);
    Ok(next)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::test_support::everything_changed;
    use super::super::*;
    use super::READ_ONLY;

    fn err(field: &str, key: &'static str) -> PatchError {
        PatchError {
            field: field.into(),
            key,
        }
    }

    #[test]
    fn patch_merges_objects_and_replaces_arrays() {
        let base = Settings::default();
        let next = apply_patch(&base, &json!({"general": {"intervalMs": 2000}})).unwrap();
        let mut want = base.clone();
        want.general.interval_ms = 2000;
        assert_eq!(next, want);

        let next = apply_patch(&base, &json!({"sources": {"smartDisabledDrives": ["a"]}})).unwrap();
        assert_eq!(next.sources.smart_disabled_drives, vec!["a".to_string()]);
        let next = apply_patch(&next, &json!({"sources": {"smartDisabledDrives": []}})).unwrap();
        assert!(next.sources.smart_disabled_drives.is_empty());

        let start = everything_changed();
        let next =
            apply_patch(&start, &json!({"advanced": {"series": {"gpu/x": ["id"]}}})).unwrap();
        let mut want = start.clone();
        want.advanced
            .series
            .insert("gpu/x".into(), vec!["id".into()]);
        assert_eq!(next, want);
    }

    #[test]
    fn empty_patch_is_a_no_op() {
        let start = everything_changed();
        assert_eq!(apply_patch(&start, &json!({})).unwrap(), start);
    }

    #[test]
    fn patch_rejects_invalid_values() {
        let base = Settings::default();
        let cases = [
            (
                json!({"general": {"intervalMs": 700}}),
                err("general.intervalMs", "settings.error.range"),
            ),
            (
                json!({"general": {"chartFps": 45}}),
                err("general.chartFps", "settings.error.range"),
            ),
            (
                json!({"advanced": {"window": 100}}),
                err("advanced.window", "settings.error.range"),
            ),
            (
                json!({"general": {"language": "de"}}),
                err("general.language", "settings.error.type"),
            ),
            (
                json!({"general": {"intervalMs": "fast"}}),
                err("general.intervalMs", "settings.error.type"),
            ),
            (
                json!({"tray": {"closeToTray": null}}),
                err("tray.closeToTray", "settings.error.null"),
            ),
            (
                json!({"general": null}),
                err("general", "settings.error.null"),
            ),
            (
                json!({"nope": 1}),
                err("nope", "settings.error.unknownField"),
            ),
            (
                json!({"general": {"nope": 1}}),
                err("general.nope", "settings.error.unknownField"),
            ),
            (
                json!({"sources": {"serviceModules": {"gpu": true}}}),
                err("sources.serviceModules.gpu", "settings.error.unknownField"),
            ),
            (
                json!({"version": 2}),
                err("version", "settings.error.readOnlyField"),
            ),
            (
                json!({"migrations": {"webviewV1": true}}),
                err("migrations", "settings.error.readOnlyField"),
            ),
            (json!({"rules": null}), err("rules", "settings.error.null")),
            (
                json!({"rules": {"overrides": {"gpu-temp": null}}}),
                err("rules.overrides.gpu-temp", "settings.error.null"),
            ),
            (
                json!({"rules": {"overrides": {"gpu-temp": {"enabled": null}}}}),
                err("rules.overrides.gpu-temp.enabled", "settings.error.null"),
            ),
            (
                json!({"rules": {"overrides": {"gpu-temp": {"target": {}}}}}),
                err(
                    "rules.overrides.gpu-temp.target",
                    "settings.error.unknownField",
                ),
            ),
            (
                json!({"rules": {"custom": null}}),
                err("rules.custom", "settings.error.null"),
            ),
            (
                json!({"rules": {"extra": 1}}),
                err("rules.extra", "settings.error.unknownField"),
            ),
            (
                json!({"log": {}}),
                err("log", "settings.error.readOnlyField"),
            ),
            (json!([]), err("", "settings.error.notObject")),
            (json!("x"), err("", "settings.error.notObject")),
        ];
        for (patch, want) in cases {
            assert_eq!(apply_patch(&base, &patch), Err(want), "patch {patch}");
        }
    }

    #[test]
    fn nullable_fields_accept_null() {
        let start = everything_changed();
        let patch = json!({
            "tray": {"iconSensor": null},
            "advanced": {"section": null, "window": null},
            "view": {"last": null}
        });
        let next = apply_patch(&start, &patch).unwrap();
        assert_eq!(next.tray.icon_sensor, None);
        assert_eq!(next.advanced.section, None);
        assert_eq!(next.advanced.window, None);
        assert_eq!(next.view.last, None);
        assert_eq!(next.general, start.general);
    }

    #[test]
    fn series_keys_are_free_form() {
        let next = apply_patch(
            &Settings::default(),
            &json!({"advanced": {"series": {"any.key/with-stuff": ["a", "b"]}}}),
        )
        .unwrap();
        assert_eq!(next.advanced.series.len(), 1);
        let bad = apply_patch(
            &Settings::default(),
            &json!({"advanced": {"series": {"k": [1]}}}),
        );
        assert_eq!(bad, Err(err("advanced.series", "settings.error.type")));
    }

    #[test]
    fn failed_patch_changes_nothing() {
        let current = everything_changed();
        let snapshot = current.clone();
        let result = apply_patch(&current, &json!({"general": {"intervalMs": 700}}));
        assert_eq!(
            result,
            Err(err("general.intervalMs", "settings.error.range"))
        );
        assert_eq!(current, snapshot);
    }

    const CUSTOM_ID: &str = "custom-00000000-0000-4000-8000-000000000001";

    fn custom_rule(warn: f64, crit: f64, warn_duration: u32) -> serde_json::Value {
        json!({
            "id": CUSTOM_ID,
            "target": {"sensor": "cpu/0/temperature/package"},
            "unit": "celsius",
            "condition": "above",
            "warn": {"threshold": {"fixed": warn}, "durationS": warn_duration},
            "crit": {"threshold": {"fixed": crit}, "durationS": 0}
        })
    }

    #[test]
    fn rules_patch_replaces_level_objects() {
        let property_warn = json!({
            "threshold": {"property": "tjMaxC", "offset": -10.0, "fallback": 80.0},
            "durationS": 30
        });
        let start = apply_patch(
            &Settings::default(),
            &json!({"rules": {"overrides": {"gpu-temp": {"warn": property_warn, "enabled": false}}}}),
        )
        .unwrap();
        let next = apply_patch(
            &start,
            &json!({"rules": {"overrides": {"gpu-temp": {
                "warn": {"threshold": {"fixed": 85}, "durationS": 20}
            }}}}),
        )
        .unwrap();
        let over = &encode(&next)["rules"]["overrides"]["gpu-temp"];
        // The level object was replaced whole: no key of the property threshold is left.
        assert_eq!(
            over,
            &json!({"enabled": false, "warn": {"threshold": {"fixed": 85.0}, "durationS": 20}})
        );

        // `null` switches a level off; the other override fields stay.
        let off = apply_patch(
            &next,
            &json!({"rules": {"overrides": {"gpu-temp": {"crit": null}}}}),
        )
        .unwrap();
        let over = &off.rules.overrides["gpu-temp"];
        assert_eq!(over.crit, Some(None));
        assert_eq!(over.enabled, Some(false));
    }

    #[test]
    fn rules_patch_is_validated() {
        let current = Settings::default();
        let cases = [
            (
                json!({"rules": {"custom": [custom_rule(90.0, 80.0, 0)]}}),
                err("rules.custom.0.crit", "rules.error.order"),
            ),
            (
                json!({"rules": {"custom": [custom_rule(80.0, 90.0, 601)]}}),
                err("rules.custom.0.warn.durationS", "rules.error.duration"),
            ),
            (
                json!({"rules": {"custom": [{"id": CUSTOM_ID, "warn": "alta"}]}}),
                err("rules.custom.0", "settings.error.type"),
            ),
            (
                json!({"rules": {"custom": [custom_rule(80.0, 90.0, 0), custom_rule(80.0, 90.0, 0)]}}),
                err("rules.custom.1.id", "rules.error.duplicateId"),
            ),
            (
                json!({"rules": {"overrides": {"gpu-temp": {"warn": {"threshold": {"fixed": 95}, "durationS": 0}}}}}),
                err("rules.overrides.gpu-temp.crit", "rules.error.order"),
            ),
            (
                json!({"rules": {"overrides": {"gpu-temp": {"warn": {"threshold": {"fixed": "x"}, "durationS": 0}}}}}),
                err("rules.overrides.gpu-temp.warn", "settings.error.type"),
            ),
            (
                json!({"rules": {"overrides": {"no-such-rule": {"enabled": false}}}}),
                err("rules.overrides.no-such-rule", "rules.error.unknownRule"),
            ),
            (
                json!({"rules": {"custom": {}}}),
                err("rules.custom", "settings.error.type"),
            ),
        ];
        for (patch, want) in cases {
            assert_eq!(apply_patch(&current, &patch), Err(want), "patch {patch}");
        }
        assert_eq!(current, Settings::default());

        let ok = apply_patch(
            &current,
            &json!({"rules": {"custom": [custom_rule(80.0, 90.0, 0)]}}),
        )
        .unwrap();
        assert_eq!(ok.rules.custom.len(), 1);
        assert_eq!(ok.rules.custom[0].id, CUSTOM_ID);
    }

    #[test]
    fn reset_rule_override_removes_only_that_entry() {
        let start = everything_changed();
        assert!(start.rules.overrides.contains_key("gpu-temp"));
        let next = reset_rule_override(&start, "gpu-temp").unwrap();
        assert!(!next.rules.overrides.contains_key("gpu-temp"));
        assert_eq!(next.rules.custom, start.rules.custom);
        // No entry: nothing changes and it is not an error.
        assert_eq!(reset_rule_override(&next, "gpu-temp").unwrap(), next);
        assert_eq!(
            reset_rule_override(&start, "no-such-rule"),
            Err(err(
                "rules.overrides.no-such-rule",
                "rules.error.unknownRule"
            ))
        );
    }

    #[test]
    fn patch_schema_covers_every_encoded_field() {
        // Ties the field names of `encode`, `decode_lenient` and the patch
        // schema together: a field added to one and not the others fails here.
        let mut patch = encode(&everything_changed());
        for key in READ_ONLY {
            patch.as_object_mut().unwrap().remove(key);
        }
        let applied = apply_patch(&Settings::default(), &patch)
            .unwrap_or_else(|e| panic!("the schema rejects an encoded field: {e:?}"));
        let mut round = encode(&applied);
        for key in READ_ONLY {
            round.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(round, patch);
    }
}
