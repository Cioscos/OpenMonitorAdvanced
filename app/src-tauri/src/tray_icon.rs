//! Dynamic tray icon (a number on a rounded square) and tooltip, as pure functions.

use oma_core::model::Unit;
use oma_core::settings::TemperatureUnit;

use crate::i18n::{t, Lang};

pub const ICON_SIZE: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconStyle {
    pub background: [u8; 4],
    pub foreground: [u8; 4],
}

pub const NEUTRAL: IconStyle = IconStyle {
    background: [0x21, 0x17, 0x33, 0xff],
    foreground: [0xf5, 0xee, 0xfe, 0xff],
};

const DASH: &str = "\u{2014}";
const CORNER_RADIUS: f64 = 6.0;
const MAX_TOOLTIP_UNITS: usize = 127;
const SEPARATOR: &str = " \u{b7} ";

/// Whole-number text for the icon: no unit symbol, Fahrenheit for temperatures
/// when asked, limited to -99..=999 so it fits in three characters.
pub fn icon_text(value: Option<f64>, unit: Unit, temperature: TemperatureUnit) -> String {
    match value.filter(|v| v.is_finite()) {
        Some(v) => whole(display_value(v, unit, temperature))
            .clamp(-99, 999)
            .to_string(),
        None => DASH.to_owned(),
    }
}

fn display_value(value: f64, unit: Unit, temperature: TemperatureUnit) -> f64 {
    if unit == Unit::Celsius && temperature == TemperatureUnit::F {
        value * 9.0 / 5.0 + 32.0
    } else {
        value
    }
}

/// Rounds half away from zero like the UI's `Intl.NumberFormat`; the cast saturates.
fn whole(value: f64) -> i32 {
    value.round() as i32
}

// Hand-written 4x8 bitmap font: `#` is a lit pixel.
const GLYPH_W: usize = 4;
const GLYPH_H: usize = 8;

fn glyph(c: char) -> Option<[&'static str; GLYPH_H]> {
    Some(match c {
        '0' => [
            ".##.", "#..#", "#..#", "#..#", "#..#", "#..#", "#..#", ".##.",
        ],
        '1' => [
            "..#.", ".##.", "..#.", "..#.", "..#.", "..#.", "..#.", ".###",
        ],
        '2' => [
            ".##.", "#..#", "...#", "...#", "..#.", ".#..", "#...", "####",
        ],
        '3' => [
            ".##.", "#..#", "...#", ".##.", "...#", "...#", "#..#", ".##.",
        ],
        '4' => [
            "...#", "..##", ".#.#", "#..#", "####", "...#", "...#", "...#",
        ],
        '5' => [
            "####", "#...", "#...", "###.", "...#", "...#", "#..#", ".##.",
        ],
        '6' => [
            ".##.", "#...", "#...", "###.", "#..#", "#..#", "#..#", ".##.",
        ],
        '7' => [
            "####", "...#", "...#", "..#.", "..#.", ".#..", ".#..", ".#..",
        ],
        '8' => [
            ".##.", "#..#", "#..#", ".##.", "#..#", "#..#", "#..#", ".##.",
        ],
        '9' => [
            ".##.", "#..#", "#..#", ".###", "...#", "...#", "..#.", ".##.",
        ],
        '-' => [
            "....", "....", "....", ".##.", "....", "....", "....", "....",
        ],
        '\u{2014}' => [
            "....", "....", "....", "####", "....", "....", "....", "....",
        ],
        _ => return None,
    })
}

/// A rounded square in `style.background` (transparent corners, anti-aliased
/// edge) with `text` centered in `style.foreground`, as RGBA.
pub fn render(text: &str, style: IconStyle) -> Vec<u8> {
    let size = ICON_SIZE as usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let mut pixel = style.background;
            pixel[3] = (f64::from(pixel[3]) * corner_coverage(x, y)).round() as u8;
            rgba[(y * size + x) * 4..][..4].copy_from_slice(&pixel);
        }
    }

    let glyphs: Vec<_> = text.chars().filter_map(glyph).collect();
    if glyphs.is_empty() {
        return rgba;
    }
    // The largest scale (3, 2 or 1) whose text, with one scaled pixel between
    // characters, stays within the 30 central columns.
    let count = glyphs.len();
    let width_at = |scale: usize| (count * GLYPH_W + (count - 1)) * scale;
    let scale = (1..=3)
        .rev()
        .find(|&s| width_at(s) <= size - 2)
        .unwrap_or(1);
    let left = (size - width_at(scale).min(size)) / 2;
    let top = (size - GLYPH_H * scale) / 2;
    for (index, rows) in glyphs.iter().enumerate() {
        let origin = left + index * (GLYPH_W + 1) * scale;
        for (row, line) in rows.iter().enumerate() {
            for (col, _) in line.bytes().enumerate().filter(|&(_, b)| b == b'#') {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let (px, py) = (origin + col * scale + dx, top + row * scale + dy);
                        if px < size {
                            rgba[(py * size + px) * 4..][..4].copy_from_slice(&style.foreground);
                        }
                    }
                }
            }
        }
    }
    rgba
}

/// 0..=1 coverage of pixel (x, y) by the square with rounded corners.
fn corner_coverage(x: usize, y: usize) -> f64 {
    let edge = f64::from(ICON_SIZE) - CORNER_RADIUS;
    let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
    let dx = (CORNER_RADIUS - cx).max(cx - edge).max(0.0);
    let dy = (CORNER_RADIUS - cy).max(cy - edge).max(0.0);
    (CORNER_RADIUS + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0)
}

pub struct TooltipItem {
    pub label_key: &'static str,
    pub value: Option<f64>,
    pub unit: Unit,
}

/// `"CPU 45 °C · GPU 62 °C · RAM 48 %"`: items without a value are left out; a
/// result over 127 UTF-16 units (the tray tooltip limit) is cut after the last
/// whole item that fits, plus `…`.
pub fn tooltip(lang: Lang, items: &[TooltipItem], temperature: TemperatureUnit) -> String {
    let parts: Vec<String> = items
        .iter()
        .filter_map(|item| {
            let value = item.value.filter(|v| v.is_finite())?;
            let label = t(lang, item.label_key, &[]);
            Some(format!(
                "{label} {}",
                tooltip_value(value, item.unit, temperature)
            ))
        })
        .collect();
    let full = parts.join(SEPARATOR);
    if utf16_len(&full) <= MAX_TOOLTIP_UNITS {
        return full;
    }
    let mut text = String::new();
    for part in &parts {
        let candidate = if text.is_empty() {
            part.clone()
        } else {
            format!("{text}{SEPARATOR}{part}")
        };
        if utf16_len(&candidate) + 1 > MAX_TOOLTIP_UNITS {
            break;
        }
        text = candidate;
    }
    if text.is_empty() {
        // Even the first item is too long: cut it by characters.
        for c in parts[0].chars() {
            if utf16_len(&text) + c.len_utf16() + 1 > MAX_TOOLTIP_UNITS {
                break;
            }
            text.push(c);
        }
    }
    text.push('\u{2026}');
    text
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn tooltip_value(value: f64, unit: Unit, temperature: TemperatureUnit) -> String {
    let number = whole(display_value(value, unit, temperature));
    match (unit, temperature) {
        (Unit::Celsius, TemperatureUnit::C) => format!("{number} \u{b0}C"),
        (Unit::Celsius, TemperatureUnit::F) => format!("{number} \u{b0}F"),
        (Unit::Percent, _) => format!("{number} %"),
        (Unit::Watt, _) => format!("{number} W"),
        _ => number.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: TemperatureUnit = TemperatureUnit::C;

    fn px(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * ICON_SIZE + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    fn fg_pixels(rgba: &[u8], style: IconStyle) -> Vec<(u32, u32)> {
        (0..ICON_SIZE)
            .flat_map(|y| (0..ICON_SIZE).map(move |x| (x, y)))
            .filter(|&(x, y)| px(rgba, x, y) == style.foreground)
            .collect()
    }

    /// One char per pixel: `#` foreground, `.` background, space transparent.
    fn ascii(rgba: &[u8], style: IconStyle) -> String {
        let mut out = String::new();
        for y in 0..ICON_SIZE {
            for x in 0..ICON_SIZE {
                let p = px(rgba, x, y);
                out.push(if p == style.foreground {
                    '#'
                } else if p == style.background {
                    '.'
                } else {
                    ' '
                });
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn icon_text_handles_extremes() {
        assert_eq!(icon_text(None, Unit::Celsius, C), "—");
        assert_eq!(icon_text(Some(f64::NAN), Unit::Percent, C), "—");
        assert_eq!(icon_text(Some(f64::INFINITY), Unit::Percent, C), "—");
        assert_eq!(icon_text(Some(45.4), Unit::Celsius, C), "45");
        assert_eq!(icon_text(Some(100.0), Unit::Percent, C), "100");
        assert_eq!(
            icon_text(Some(100.0), Unit::Celsius, TemperatureUnit::F),
            "212"
        );
        // Fahrenheit applies to temperatures only.
        assert_eq!(
            icon_text(Some(50.0), Unit::Percent, TemperatureUnit::F),
            "50"
        );
        assert_eq!(icon_text(Some(-5.6), Unit::Celsius, C), "-6");
        assert_eq!(icon_text(Some(-0.2), Unit::Celsius, C), "0");
        assert_eq!(icon_text(Some(1500.0), Unit::Watt, C), "999");
        assert_eq!(icon_text(Some(-150.0), Unit::Celsius, C), "-99");
    }

    #[test]
    fn render_is_rgba_32x32() {
        let img = render("45", NEUTRAL);
        assert_eq!(img.len(), 4096);
        assert_eq!(px(&img, 0, 0)[3], 0, "corner is transparent");
        assert_eq!(px(&img, 16, 2), NEUTRAL.background);
        assert!(!fg_pixels(&img, NEUTRAL).is_empty());
    }

    #[test]
    fn render_fits_three_digits_and_minus() {
        for text in ["212", "-99", "999", "—"] {
            let img = render(text, NEUTRAL);
            let pixels = fg_pixels(&img, NEUTRAL);
            assert!(!pixels.is_empty(), "{text} draws something");
            assert!(
                pixels.iter().all(|&(x, _)| x != 0 && x != ICON_SIZE - 1),
                "{text} leaves columns 0 and 31 free"
            );
        }
    }

    #[test]
    fn render_draws_two_digits_large() {
        let img = render("88", NEUTRAL);
        let pixels = fg_pixels(&img, NEUTRAL);
        let top = pixels.iter().map(|&(_, y)| y).min().unwrap();
        let bottom = pixels.iter().map(|&(_, y)| y).max().unwrap();
        assert!(bottom - top + 1 >= 16, "rows {top}..={bottom}");
    }

    #[test]
    fn render_distinguishes_glyphs() {
        assert_ne!(render("45", NEUTRAL), render("46", NEUTRAL));
        assert_ne!(render("-", NEUTRAL), render("—", NEUTRAL));
    }

    fn item(label_key: &'static str, value: Option<f64>, unit: Unit) -> TooltipItem {
        TooltipItem {
            label_key,
            value,
            unit,
        }
    }

    #[test]
    fn tooltip_formats_and_truncates() {
        let items = [
            item("tray.tooltip.cpu", Some(45.2), Unit::Celsius),
            item("tray.tooltip.gpu", Some(61.6), Unit::Celsius),
            item("tray.tooltip.ram", Some(48.0), Unit::Percent),
        ];
        assert_eq!(
            tooltip(Lang::En, &items, C),
            "CPU 45 °C · GPU 62 °C · RAM 48 %"
        );
        assert_eq!(
            tooltip(Lang::En, &items, TemperatureUnit::F),
            "CPU 113 °F · GPU 143 °F · RAM 48 %"
        );

        let missing = [
            item("tray.tooltip.cpu", Some(45.0), Unit::Celsius),
            item("tray.tooltip.gpu", None, Unit::Celsius),
            item("tray.tooltip.ram", Some(48.0), Unit::Percent),
        ];
        assert_eq!(tooltip(Lang::En, &missing, C), "CPU 45 °C · RAM 48 %");
        assert_eq!(tooltip(Lang::En, &[], C), "");

        // Unknown keys are shown as they are, which makes long labels easy to test.
        const LONG: &str = "a-very-long-label-that-eats-the-tooltip-budget-of-the-tray";
        let long = [
            item(LONG, Some(1.0), Unit::Percent),
            item(LONG, Some(2.0), Unit::Percent),
            item(LONG, Some(3.0), Unit::Percent),
        ];
        let text = tooltip(Lang::En, &long, C);
        assert!(text.encode_utf16().count() <= 127, "{text}");
        assert!(text.ends_with('…'), "{text}");
        assert!(text.starts_with(&format!("{LONG} 1 %")), "{text}");
        assert!(!text.contains("3 %"), "{text}");
    }

    #[test]
    fn tooltip_truncates_a_single_oversized_item() {
        let huge: &'static str = Box::leak("x".repeat(200).into_boxed_str());
        let text = tooltip(Lang::En, &[item(huge, Some(1.0), Unit::Percent)], C);
        assert!(text.encode_utf16().count() <= 127);
        assert!(text.ends_with('…'));
    }

    #[test]
    #[ignore = "prints the icons for a visual check"]
    fn print_icons() {
        for text in ["45", "212", "-99", "—"] {
            println!("== {text}\n{}", ascii(&render(text, NEUTRAL), NEUTRAL));
        }
    }
}
