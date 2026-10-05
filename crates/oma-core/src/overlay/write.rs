//! Minimal profile JSON (spec 6.3): only what differs from the defaults the
//! deserializer would apply, so saved files stay short and readable.

use super::profile::*;
use serde_json::Value;

/// Keys written first, in this order, at every level; the rest follow alphabetically.
const KEY_ORDER: &[&str] = &[
    "format",
    "name",
    "anchor",
    "offset",
    "scale",
    "panel",
    "id",
    "x",
    "y",
    "w",
    "h",
    "rect",
    "z",
    "source",
    "stat",
    "kind",
    "style",
    "thresholds",
    "visibleIf",
    "blocks",
];

/// Block keys that are always written, even when equal to the dummy default.
const BLOCK_KEEP: &[&str] = &["id", "rect", "source", "kind"];

/// Drops from `v` every key equal to the same key in `def`, recursing into objects.
fn strip(v: &mut Value, def: &Value, keep: &[&str]) {
    let (Value::Object(m), Value::Object(d)) = (v, def) else {
        return;
    };
    m.retain(|k, val| {
        if keep.contains(&k.as_str()) {
            return true;
        }
        match d.get(k) {
            Some(dv) if dv == val => false,
            Some(dv) => {
                strip(val, dv, &[]);
                true
            }
            None => true,
        }
    });
}

fn to_value<T: serde::Serialize>(t: &T) -> Value {
    serde_json::to_value(t).expect("profile types serialize")
}

fn rank(k: &str) -> (usize, &str) {
    (
        KEY_ORDER.iter().position(|o| *o == k).unwrap_or(usize::MAX),
        k,
    )
}

/// Compact JSON writer with the `KEY_ORDER` key order (`Value` maps are sorted).
fn emit(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<_> = m.keys().collect();
            keys.sort_by_key(|k| rank(k));
            out.push('{');
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push(':');
                emit(&m[k], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, e) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                emit(e, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Compact JSON of `profile` with default values left out.
pub fn profile_to_json(profile: &Profile) -> String {
    let mut v = to_value(profile);
    let def = to_value(&Profile {
        format: profile.format,
        name: profile.name.clone(),
        anchor: Anchor::default(),
        offset: CellPoint::default(),
        scale: 1.0,
        panel: Panel::default(),
        blocks: Vec::new(),
    });
    strip(&mut v, &def, &["format", "name", "blocks"]);

    let block_def = to_value(&Block {
        id: String::new(),
        rect: CellRect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        },
        z: 0,
        source: Source::Text(String::new()),
        stat: Stat::default(),
        kind: Kind::Text,
        style: Style::default(),
        thresholds: Vec::new(),
        visible_if: None,
        panel: None,
    });
    let threshold_def = serde_json::json!({ "target": "value" });
    let panel_def = to_value(&Panel::default());
    let compare_def = serde_json::json!({ "stat": to_value(&Stat::default()) });
    if let Some(blocks) = v.get_mut("blocks").and_then(Value::as_array_mut) {
        for b in blocks {
            strip(b, &block_def, BLOCK_KEEP);
            if let Some(ts) = b.get_mut("thresholds").and_then(Value::as_array_mut) {
                ts.iter_mut().for_each(|t| strip(t, &threshold_def, &[]));
            }
            if let Some(p) = b.get_mut("panel") {
                strip(p, &panel_def, &[]);
            }
            if let Some(c) = b.get_mut("visibleIf") {
                strip(c, &compare_def, &[]);
            }
        }
    }
    let mut out = String::new();
    emit(&v, &mut out);
    out
}

/// `wanted` if free, else `wanted (2)`, `wanted (3)`...; case-insensitive.
/// A `wanted` already ending in ` (n)` continues from `n + 1`. The base is
/// shortened (on a character boundary) so the result fits
/// [`MAX_TEXT_BYTES`].
pub fn unique_name(existing: &[&str], wanted: &str) -> String {
    let taken = |n: &str| {
        existing
            .iter()
            .any(|e| e.to_lowercase() == n.to_lowercase())
    };
    if !taken(wanted) {
        return wanted.to_owned();
    }
    let (base, start) = split_suffix(wanted);
    (start..)
        .map(|n| {
            let suffix = format!(" ({n})");
            let mut end = base.len().min(MAX_TEXT_BYTES - suffix.len());
            while !base.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}{suffix}", &base[..end])
        })
        .find(|c| !taken(c))
        .expect("unbounded range")
}

/// `("Gaming", 3)` for `"Gaming (2)"`, `("Gaming", 2)` for `"Gaming"`.
fn split_suffix(s: &str) -> (&str, u32) {
    s.strip_suffix(')')
        .and_then(|r| r.rsplit_once(" ("))
        .and_then(|(b, n)| Some((b, n.parse::<u32>().ok()?.checked_add(1)?)))
        .unwrap_or((s, 2))
}

/// `b1`, `b2`... the first id not in `existing`.
pub fn new_block_id(existing: &[&str]) -> String {
    (1..)
        .map(|n| format!("b{n}"))
        .find(|c| !existing.contains(&c.as_str()))
        .expect("unbounded range")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::{builtin_profile, parse_profile, BuiltinId};
    use serde_json::json;

    fn parse(v: Value) -> Profile {
        parse_profile(&v.to_string()).unwrap()
    }

    fn block(extra: Value) -> Value {
        let mut b = json!({"id":"a","rect":{"x":0,"y":0,"w":10,"h":2},
            "source":{"frames":"fps-displayed"},"kind":"text"});
        b.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        b
    }

    #[test]
    fn minimal_json_round_trips_every_builtin() {
        let schema = serde_json::from_str(include_str!(
            "../../tests/fixtures/this-machine-schema.json"
        ))
        .unwrap();
        for id in [
            BuiltinId::Gaming,
            BuiltinId::Full,
            BuiltinId::MinimalFps,
            BuiltinId::Bar,
        ] {
            let p = builtin_profile(id, &schema);
            assert_eq!(parse_profile(&profile_to_json(&p)), Ok(p), "{id:?}");
        }
    }

    #[test]
    fn minimal_json_omits_defaults() {
        let p = parse(json!({"format":1,"name":"x","blocks":[block(json!({}))]}));
        assert_eq!(
            profile_to_json(&p),
            r#"{"format":1,"name":"x","blocks":[{"id":"a","rect":{"x":0,"y":0,"w":10,"h":2},"source":{"frames":"fps-displayed"},"kind":"text"}]}"#
        );
    }

    #[test]
    fn minimal_json_keeps_changed_nested_fields() {
        let p = parse(
            json!({"format":1,"name":"x","blocks":[block(json!({"style":{"valueStyle":{"size":18.0}}}))]}),
        );
        let v: Value = serde_json::from_str(&profile_to_json(&p)).unwrap();
        assert_eq!(
            v["blocks"][0]["style"],
            json!({"valueStyle": {"size": 18.0}})
        );
        assert_eq!(parse_profile(&v.to_string()), Ok(p));
    }

    #[test]
    fn minimal_json_keeps_non_default_thresholds_in_order() {
        let th = json!([
            {"op":">","value":80.0,"color":"#ff0000"},
            {"op":"<","value":10.0,"color":"#00ff00","target":"panel"}
        ]);
        let p = parse(json!({"format":1,"name":"x","blocks":[block(json!({"thresholds":th}))]}));
        let v: Value = serde_json::from_str(&profile_to_json(&p)).unwrap();
        let t = &v["blocks"][0]["thresholds"];
        assert!(t[0].get("target").is_none());
        assert_eq!(t[1]["target"], "panel");
        assert_eq!(t[0]["value"], 80.0);
        assert_eq!(parse_profile(&v.to_string()), Ok(p));
    }

    #[test]
    fn unique_name_appends_the_first_free_number() {
        assert_eq!(
            unique_name(&["Gaming", "gaming (2)"], "Gaming"),
            "Gaming (3)"
        );
        assert_eq!(unique_name(&["Gaming"], "Nuovo"), "Nuovo");
        assert_eq!(unique_name(&["Gaming (2)"], "Gaming (2)"), "Gaming (3)");
    }

    #[test]
    fn unique_name_fits_the_name_limit() {
        // Two-byte characters: the cut must fall on a character boundary.
        let long = "é".repeat(MAX_TEXT_BYTES / 2);
        let name = unique_name(&[long.as_str()], &long);
        assert!(name.len() <= MAX_TEXT_BYTES, "{}", name.len());
        assert!(name.ends_with("é (2)"));
    }

    #[test]
    fn new_block_id_skips_used_ids() {
        assert_eq!(new_block_id(&[]), "b1");
        assert_eq!(new_block_id(&["b1", "b3", "x"]), "b2");
    }
}
