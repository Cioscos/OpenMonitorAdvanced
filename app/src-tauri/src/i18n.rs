//! Rust-side translations. The UI catalogs (`app/src/lib/i18n/*.json`) are the
//! single source of strings; they are embedded here and parsed once, so the tray
//! (which outlives the webview) reads exactly what the UI shows.

use std::collections::HashMap;
use std::sync::OnceLock;

use oma_core::model::Label;
use oma_core::settings::Language;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    It,
}

/// Every catalog key the Rust code looks up by name (sensor labels are dynamic);
/// the test below checks each one exists in both catalogs.
#[cfg(test)]
pub const RUST_KEYS: &[&str] = &[
    "tray.open",
    "tray.viewSimple",
    "tray.viewAdvanced",
    "tray.antiCheat",
    "tray.quit",
    "tray.tooltip.cpu",
    "tray.tooltip.gpu",
    "tray.tooltip.ram",
    "flag.on",
    "flag.off",
    "rule.cpu-temp.name",
    "rule.cpu-temp.message",
    "rule.cpu-throttle.name",
    "rule.cpu-throttle.message",
    "rule.gpu-temp.name",
    "rule.gpu-temp.message",
    "rule.gpu-hotspot.name",
    "rule.gpu-hotspot.message",
    "rule.gpu-mem-temp.name",
    "rule.gpu-mem-temp.message",
    "rule.gpu-throttle.name",
    "rule.gpu-throttle.message",
    "rule.disk-temp.name",
    "rule.disk-temp.message",
    "rule.disk-wear.name",
    "rule.disk-wear.message",
    "rule.disk-critical.name",
    "rule.disk-critical.message",
    "rule.volume-used.name",
    "rule.volume-used.message",
    "rule.ram-used.name",
    "rule.ram-used.message",
    "rule.battery-low.name",
    "rule.battery-low.message",
    "rule.custom.above",
    "rule.custom.below",
    "rule.custom.flag",
    "health.problems",
    "health.allClear",
    "health.partial",
    "health.unavailableValue",
];

type Catalog = HashMap<String, String>;

/// `[en, it]`, parsed on first use.
fn catalogs() -> &'static [Catalog; 2] {
    static CATALOGS: OnceLock<[Catalog; 2]> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        let parse = |json: &str| serde_json::from_str(json).expect("embedded UI catalog");
        [
            parse(include_str!("../../src/lib/i18n/en.json")),
            parse(include_str!("../../src/lib/i18n/it.json")),
        ]
    })
}

/// The language to use: the setting, or with `System` the OS locale (`it`,
/// `it-IT`, `it_CH`... give Italian, anything else English).
pub fn resolve(language: Language, system_locale: &str) -> Lang {
    match language {
        Language::En => Lang::En,
        Language::It => Lang::It,
        Language::System => {
            let base = system_locale.split(['-', '_']).next().unwrap_or("");
            if base.eq_ignore_ascii_case("it") {
                Lang::It
            } else {
                Lang::En
            }
        }
    }
}

/// Looks `key` up in `lang`, then in English, then falls back to the key itself.
/// `{name}` placeholders are replaced like the UI does; unknown ones stay as written.
pub fn t(lang: Lang, key: &str, params: &[(&str, &str)]) -> String {
    let [en, it] = catalogs();
    let catalog = if lang == Lang::It { it } else { en };
    let template = catalog
        .get(key)
        .or_else(|| en.get(key))
        .map_or(key, String::as_str);
    interpolate(template, params)
}

fn interpolate(template: &str, params: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let name_len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        let name = &after[..name_len];
        let closed = name_len > 0 && after[name_len..].starts_with('}');
        match params.iter().find(|(n, _)| closed && *n == name) {
            Some((_, value)) => {
                out.push_str(value);
                rest = &after[name_len + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The text of a sensor label: catalog key `sensor.<key>` with `{arg}`.
pub fn sensor_label(lang: Lang, label: &Label) -> String {
    let key = format!("sensor.{}", label.key);
    match label.arg.as_deref() {
        Some(arg) => t(lang, &key, &[("arg", arg)]),
        None => t(lang, &key, &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_follows_settings_then_system() {
        assert_eq!(resolve(Language::It, "en-US"), Lang::It);
        assert_eq!(resolve(Language::System, "it-IT"), Lang::It);
        assert_eq!(resolve(Language::System, "it_CH"), Lang::It);
        assert_eq!(resolve(Language::System, "it"), Lang::It);
        assert_eq!(resolve(Language::System, "de-DE"), Lang::En);
        assert_eq!(resolve(Language::System, ""), Lang::En);
        assert_eq!(resolve(Language::En, "it-IT"), Lang::En);
    }

    #[test]
    fn t_falls_back_to_english_then_key() {
        assert_eq!(t(Lang::It, "tray.quit", &[]), "Esci");
        assert_eq!(t(Lang::En, "tray.quit", &[]), "Quit");
        assert_eq!(t(Lang::It, "no.such.key", &[]), "no.such.key");
    }

    #[test]
    fn t_replaces_params() {
        assert_eq!(
            t(Lang::En, "sensor.cpu.load.thread", &[("arg", "3")]),
            "Thread 3 load"
        );
        // Unknown placeholders are left as they are, like the UI does.
        assert_eq!(
            t(Lang::En, "sensor.cpu.load.thread", &[]),
            "Thread {arg} load"
        );
    }

    #[test]
    fn sensor_label_uses_the_catalog_and_arg() {
        let label = Label::with_arg("cpu.load.thread", "3");
        assert_eq!(sensor_label(Lang::En, &label), "Thread 3 load");
        assert_eq!(sensor_label(Lang::It, &label), "Carico thread 3");
        assert_eq!(
            sensor_label(Lang::En, &Label::new("cpu.load.total")),
            "Total load"
        );
    }

    #[test]
    fn rust_keys_exist_in_both_catalogs() {
        for lang in [Lang::En, Lang::It] {
            for key in RUST_KEYS {
                assert_ne!(t(lang, key, &[]), *key, "{key} missing for {lang:?}");
            }
        }
        // Distinct texts in the two languages prove the Italian catalog is read.
        assert_eq!(t(Lang::It, "tray.viewAdvanced", &[]), "Vista Avanzata");
        assert_eq!(t(Lang::En, "tray.viewSimple", &[]), "Simple view");
    }

    #[test]
    fn every_default_rule_has_a_name_and_a_message() {
        for rule in oma_core::rules::default_rules() {
            for part in ["name", "message"] {
                let key = format!("rule.{}.{part}", rule.id);
                assert!(RUST_KEYS.contains(&key.as_str()), "{key} not in RUST_KEYS");
            }
        }
        let value = [("device", "RTX 4080"), ("value", "92 °C")];
        assert_eq!(
            t(Lang::It, "rule.gpu-temp.message", &value),
            "RTX 4080 surriscaldata (92 °C)"
        );
        assert_eq!(
            t(Lang::En, "rule.gpu-temp.message", &value),
            "RTX 4080 overheating (92 °C)"
        );
    }
}
