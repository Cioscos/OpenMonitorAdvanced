//! Merging a theme with its base and writing it back as normalized CSS.

use std::collections::BTreeMap;

use serde::Serialize;

use super::builtin::builtin;
use super::parse::ParsedTheme;
use super::variables::variables;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    Dark,
    Light,
}

/// `--color-scheme` of the theme; dark when absent.
pub fn scheme_of(t: &ParsedTheme) -> Scheme {
    match t.vars.get("--color-scheme").map(String::as_str) {
        Some("light") => Scheme::Light,
        _ => Scheme::Dark,
    }
}

/// The base theme of the scheme with the variables of `t` on top.
pub fn resolve(t: &ParsedTheme) -> BTreeMap<String, String> {
    let id = match scheme_of(t) {
        Scheme::Dark => "builtin-synthwave",
        Scheme::Light => "builtin-daylight",
    };
    let mut out = builtin(id)
        .expect("base themes are built in")
        .theme
        .vars
        .clone();
    out.extend(t.vars.iter().map(|(k, v)| (k.clone(), v.clone())));
    out
}

/// The theme as a file of the §2.1 format, variables in contract order.
pub fn to_css(t: &ParsedTheme) -> String {
    // The header is a comment: keep `*/` out of the values we put in it.
    let safe = |s: &str| s.replace("*/", "* /");
    let mut s = format!("/* @oma-theme {}", t.meta.version);
    if let Some(n) = &t.meta.name {
        s.push_str(&format!("\n   @name {}", safe(n)));
    }
    if let Some(a) = &t.meta.author {
        s.push_str(&format!("\n   @author {}", safe(a)));
    }
    s.push_str(" */\n:root {\n");
    for spec in variables() {
        if let Some(v) = t.vars.get(&spec.name) {
            s.push_str(&format!("  {}: {v};\n", spec.name));
        }
    }
    s.push_str("}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::super::contrast::contrast_warnings;
    use super::super::parse::{parse_theme, WarningCode};
    use super::*;

    fn fixture(name: &str) -> ParsedTheme {
        let path = format!(
            "{}/../../testdata/themes/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        parse_theme(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn resolve_fills_from_scheme_base() {
        let dark = resolve(&fixture("minimal.css"));
        assert_eq!(dark.len(), 22);
        assert_eq!(dark["--bg"], "#101820");
        assert_eq!(dark["--surface"], "#181126");
        let light = resolve(&fixture("light.css"));
        assert_eq!(light.len(), 22);
        assert_eq!(light["--surface-2"], "#eeeaf5");
        assert_eq!(light["--bg"], "#f7f7fa");
    }

    #[test]
    fn to_css_round_trips() {
        let t = fixture("full-dark.css");
        let back = parse_theme(to_css(&t).as_bytes()).unwrap();
        assert_eq!(back.vars, t.vars);
        assert_eq!(back.meta, t.meta);
        assert!(back.warnings.is_empty());
    }

    #[test]
    fn user_theme_gets_contrast_warnings() {
        let t = parse_theme(b":root { --bg: #000000; --text-muted: #3a3a3a; }").unwrap();
        let w = contrast_warnings(&resolve(&t));
        // Also fails on the inherited --surface; only --bg is asserted here.
        let on_bg: Vec<_> = w
            .iter()
            .filter(|x| x.detail.starts_with("--text-muted/--bg "))
            .collect();
        assert_eq!(on_bg.len(), 1, "{w:?}");
        assert_eq!(on_bg[0].code, WarningCode::LowContrast);
        assert_eq!(on_bg[0].line, 0);
    }
}
