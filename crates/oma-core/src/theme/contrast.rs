//! WCAG 2.x contrast checks on a resolved theme (spec M10 §2.5).

use std::collections::BTreeMap;

use super::parse::{ThemeWarning, WarningCode};

/// Pairs checked: foreground, backgrounds, minimum ratio.
const PAIRS: &[(&str, &[&str], f64)] = &[
    ("--text", &["--bg", "--surface", "--surface-2"], 4.5),
    ("--text-muted", &["--bg", "--surface"], 3.0),
    ("--on-accent", &["--accent"], 4.5),
    ("--ok", &["--bg"], 3.0),
    ("--warn", &["--bg"], 3.0),
    ("--crit", &["--bg"], 3.0),
    ("--accent", &["--bg"], 3.0),
];

/// Reads a normalized color (see the parser) as RGBA in 0..=1.
pub fn parse_color(v: &str) -> Option<[f64; 4]> {
    let v = v.trim();
    if v == "transparent" {
        return Some([0.0; 4]);
    }
    if let Some(hex) = v.strip_prefix('#') {
        if hex.len() != 6 && hex.len() != 8 {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        let a = if hex.len() == 8 { byte(6)? } else { 255 };
        let c = |b: u8| f64::from(b) / 255.0;
        return Some([c(byte(0)?), c(byte(2)?), c(byte(4)?), c(a)]);
    }
    let (name, rest) = v.split_once('(')?;
    let inner = rest.strip_suffix(')')?;
    let parts: Vec<&str> = inner
        .split([',', '/', ' '])
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    // A number, or a percentage scaled by `full`.
    let num = |s: &str, full: f64| match s.strip_suffix('%') {
        Some(p) => p.parse::<f64>().ok().map(|n| n / 100.0 * full),
        None => s.parse::<f64>().ok(),
    };
    let alpha = match parts.get(3) {
        Some(a) => num(a, 1.0)?.clamp(0.0, 1.0),
        None => 1.0,
    };
    let [r, g, b] = if name.starts_with("rgb") {
        let c = |i: usize| num(parts[i], 255.0).map(|n| n.clamp(0.0, 255.0) / 255.0);
        [c(0)?, c(1)?, c(2)?]
    } else if name.starts_with("hsl") {
        let h = parts[0].strip_suffix("deg").unwrap_or(parts[0]);
        let h = h.parse::<f64>().ok()?.rem_euclid(360.0);
        // Saturation and lightness are percentages; a bare number is read as one too.
        let pct = |s: &str| num(s, 1.0).map(|n| if s.ends_with('%') { n } else { n / 100.0 });
        let s = pct(parts[1])?.clamp(0.0, 1.0);
        let l = pct(parts[2])?.clamp(0.0, 1.0);
        let f = |n: f64| {
            let k = (n + h / 30.0) % 12.0;
            let a = s * l.min(1.0 - l);
            l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
        };
        [f(0.0), f(8.0), f(4.0)]
    } else {
        return None;
    };
    Some([r, g, b, alpha])
}

fn luminance(c: [f64; 4]) -> f64 {
    let lin = |v: f64| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG contrast ratio; `fg` is composited over `bg` when translucent
/// (`bg` is taken as opaque).
pub fn contrast_ratio(fg: [f64; 4], bg: [f64; 4]) -> f64 {
    let a = fg[3];
    let mixed = [
        fg[0] * a + bg[0] * (1.0 - a),
        fg[1] * a + bg[1] * (1.0 - a),
        fg[2] * a + bg[2] * (1.0 - a),
        1.0,
    ];
    let (l1, l2) = (luminance(mixed), luminance(bg));
    (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
}

/// One `LowContrast` warning (`line: 0`) per failing pair of the §2.5 table.
pub fn contrast_warnings(resolved: &BTreeMap<String, String>) -> Vec<ThemeWarning> {
    let color = |n: &str| resolved.get(n).and_then(|v| parse_color(v));
    let mut out = Vec::new();
    for (fg_name, bgs, min) in PAIRS {
        let Some(fg) = color(fg_name) else { continue };
        for bg_name in *bgs {
            let Some(bg) = color(bg_name) else { continue };
            let ratio = contrast_ratio(fg, bg);
            if ratio < *min {
                out.push(ThemeWarning {
                    line: 0,
                    code: WarningCode::LowContrast,
                    detail: format!("{fg_name}/{bg_name} {ratio:.1}"),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ratio(a: &str, b: &str) -> f64 {
        contrast_ratio(parse_color(a).unwrap(), parse_color(b).unwrap())
    }

    #[test]
    fn contrast_of_known_pairs() {
        assert!((ratio("#000000", "#ffffff") - 21.0).abs() < 0.01);
        assert!((ratio("#777777", "#ffffff") - 4.48).abs() < 0.01);
    }

    #[test]
    fn alpha_is_composited() {
        let (a, b) = (ratio("#00000080", "#ffffff"), ratio("#808080", "#ffffff"));
        assert!((a - b).abs() < 0.1, "{a} {b}");
    }

    #[test]
    fn colors_parse_in_every_normalized_form() {
        assert_eq!(parse_color("transparent"), Some([0.0; 4]));
        assert_eq!(
            parse_color("rgb(255 0 0 / 50%)"),
            Some([1.0, 0.0, 0.0, 0.5])
        );
        assert_eq!(
            parse_color("rgba(0, 0, 255, .5)"),
            Some([0.0, 0.0, 1.0, 0.5])
        );
        let g = parse_color("hsl(120deg 100% 50%)").unwrap();
        assert!(g[0].abs() < 1e-9 && (g[1] - 1.0).abs() < 1e-9 && g[2].abs() < 1e-9);
        assert_eq!(parse_color("#abc"), None);
        assert_eq!(parse_color("red"), None);
    }
}
