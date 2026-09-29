//! Strict validation and merging of settings patches.

use serde_json::{Map, Value};

use super::decode::{decode_lenient, DiagnosticKind};
use super::{encode, Settings};

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
const READ_ONLY: [&str; 4] = ["version", "migrations", "rules", "log"];

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
];

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
        }
    }
    Ok(())
}

/// Objects merge recursively, everything else (arrays included) replaces.
fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                match base.get_mut(key) {
                    Some(existing) => merge(existing, value),
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, patch) => *base = patch.clone(),
    }
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
    merge(&mut merged, patch);
    let decoded = decode_lenient(&merged);
    if let Some(diagnostic) = decoded.diagnostics.into_iter().next() {
        let key = match diagnostic.kind {
            DiagnosticKind::Corrected { .. } => "settings.error.range",
            DiagnosticKind::WrongType
            | DiagnosticKind::UnknownVariant
            | DiagnosticKind::MissingVersion => "settings.error.type",
        };
        return Err(PatchError::new(diagnostic.path, key));
    }
    Ok(decoded.settings)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::test_support::everything_changed;
    use super::super::*;

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
            (
                json!({"rules": {}}),
                err("rules", "settings.error.readOnlyField"),
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
}
