//! The four built-in themes: files in the user format, included in the binary.

use std::sync::OnceLock;

use super::parse::{parse_theme, ParsedTheme};

pub const DEFAULT_THEME: &str = "builtin-synthwave";

pub struct Builtin {
    pub id: &'static str,
    pub theme: ParsedTheme,
}

const SOURCES: [(&str, &str); 4] = [
    (
        "builtin-synthwave",
        include_str!("../../../../app/src/themes/builtin/builtin-synthwave.css"),
    ),
    (
        "builtin-midnight",
        include_str!("../../../../app/src/themes/builtin/builtin-midnight.css"),
    ),
    (
        "builtin-daylight",
        include_str!("../../../../app/src/themes/builtin/builtin-daylight.css"),
    ),
    (
        "builtin-contrast",
        include_str!("../../../../app/src/themes/builtin/builtin-contrast.css"),
    ),
];

pub fn builtins() -> &'static [Builtin] {
    static ALL: OnceLock<Vec<Builtin>> = OnceLock::new();
    ALL.get_or_init(|| {
        SOURCES
            .iter()
            .map(|(id, css)| Builtin {
                id,
                theme: parse_theme(css.as_bytes()).expect("built-in themes parse (tested)"),
            })
            .collect()
    })
}

pub fn builtin(id: &str) -> Option<&'static Builtin> {
    builtins().iter().find(|b| b.id == id)
}

/// Whether `id` names a built-in theme or is a lowercase uuid v4.
pub fn is_valid_theme_id(id: &str) -> bool {
    let b = id.as_bytes();
    builtin(id).is_some()
        || (b.len() == 36
            && b.iter().enumerate().all(|(i, &c)| match i {
                8 | 13 | 18 | 23 => c == b'-',
                14 => c == b'4',
                19 => matches!(c, b'8' | b'9' | b'a' | b'b'),
                _ => c.is_ascii_digit() || (b'a'..=b'f').contains(&c),
            }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn theme_ids() {
        use super::is_valid_theme_id as ok;
        assert!(ok("builtin-contrast") && ok("3f2a9c1e-5b7d-4e8f-a1b2-0c3d4e5f6a7b"));
        for bad in [
            "",
            "../x",
            "builtin-nope",
            "BUILTIN-SYNTHWAVE",
            "3f2a9c1e-5b7d-1e8f-a1b2-0c3d4e5f6a7b",
            "3f2a9c1e-5b7d-4e8f-c1b2-0c3d4e5f6a7b",
            "3F2A9C1E-5B7D-4E8F-A1B2-0C3D4E5F6A7B",
        ] {
            assert!(!ok(bad), "{bad}");
        }
    }

    use super::super::contrast::contrast_warnings;
    use super::super::resolve::resolve;
    use super::super::variables::variables;
    use super::*;

    #[test]
    fn builtins_parse_clean_and_complete() {
        assert_eq!(builtins().len(), 4);
        for b in builtins() {
            assert!(
                b.theme.warnings.is_empty(),
                "{}: {:?}",
                b.id,
                b.theme.warnings
            );
            assert_eq!(b.theme.vars.len(), variables().len(), "{}", b.id);
            assert_eq!(b.theme.meta.name.as_deref(), Some(b.id));
        }
    }

    #[test]
    fn builtins_meet_contrast() {
        for b in builtins() {
            let w = contrast_warnings(&resolve(&b.theme));
            assert!(w.is_empty(), "{}: {w:?}", b.id);
        }
    }

    #[test]
    fn synthwave_matches_theme_css() {
        let css = std::fs::read_to_string(format!(
            "{}/../../app/src/styles/theme.css",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(css.contains("color-scheme: dark;"));
        for (name, want) in &builtin(DEFAULT_THEME).unwrap().theme.vars {
            if name == "--color-scheme" {
                continue;
            }
            let line = css
                .lines()
                .find_map(|l| l.trim().strip_prefix(&format!("{name}:")))
                .unwrap_or_else(|| panic!("{name} missing from theme.css"));
            let raw = line.split(';').next().unwrap();
            let got = parse_theme(format!(":root {{ {name}: {raw}; }}").as_bytes()).unwrap();
            assert_eq!(&got.vars[name], want, "{name}");
        }
    }
}
