//! Dynamic tray icon (a number on a rounded square) and tooltip, as pure functions.

use oma_core::model::Unit;
use oma_core::settings::TemperatureUnit;

use crate::i18n::{t, Lang};

pub const ICON_SIZE: u32 = 32;

/// The tray tooltip before any reading has a value (and at start-up).
pub const PRODUCT_NAME: &str = "OpenMonitor Advanced";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconStyle {
    pub background: [u8; 4],
    pub foreground: [u8; 4],
}

pub const NEUTRAL: IconStyle = IconStyle {
    background: [0x21, 0x17, 0x33, 0xff],
    foreground: [0xf5, 0xee, 0xfe, 0xff],
};

/// The small mark in the icon's top-right corner that says what the number is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitMark {
    None,
    Degree,
    Percent,
}

/// The mark for a reading: a degree sign for temperatures (Celsius or
/// Fahrenheit, the tooltip says which), a percent sign for loads, nothing for
/// other units or a missing value.
pub fn unit_mark(value: Option<f64>, unit: Unit) -> UnitMark {
    if !value.is_some_and(f64::is_finite) {
        return UnitMark::None;
    }
    match unit {
        Unit::Celsius => UnitMark::Degree,
        Unit::Percent => UnitMark::Percent,
        _ => UnitMark::None,
    }
}

const DASH: &str = "\u{2014}";
const CORNER_RADIUS: f64 = 6.0;
const MAX_TOOLTIP_UNITS: usize = 127;
const SEPARATOR: &str = " \u{b7} ";

/// Whole-number text for the icon: no unit symbol (the unit is the separate
/// [`UnitMark`]), Fahrenheit for temperatures when asked, limited to -99..=999
/// so it fits in three characters.
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

// Hand-written 5x5 unit marks, drawn 1:1 in the top-right corner.
const MARK_W: usize = 5;
const MARK_X: usize = 24;
const MARK_Y: usize = 2;

fn mark_glyph(mark: UnitMark) -> &'static [&'static str] {
    match mark {
        UnitMark::None => &[],
        UnitMark::Degree => &[".###.", "#...#", "#...#", "#...#", ".###."],
        UnitMark::Percent => &["##..#", "##.#.", "..#..", ".#.##", "#..##"],
    }
}

/// The lit pixels of the unit mark, as (x, y).
fn mark_pixels(mark: UnitMark) -> Vec<(usize, usize)> {
    let mut pixels = Vec::new();
    for (row, line) in mark_glyph(mark).iter().enumerate() {
        for (col, _) in line.bytes().enumerate().filter(|&(_, b)| b == b'#') {
            pixels.push((MARK_X + col, MARK_Y + row));
        }
    }
    debug_assert!(MARK_X + MARK_W < ICON_SIZE as usize);
    pixels
}

/// How the digits are scaled and placed: `across` icon pixels per glyph pixel
/// horizontally, `heights[row]` vertically, the first row at `top`.
struct Layout {
    across: usize,
    heights: [usize; GLYPH_H],
    top: usize,
}

/// With a mark and a scale of 3 the digits give up two rows (rows 1 and 6 of
/// the glyph are drawn 2 px tall) so they start below the mark: 22 rows from
/// row 8, instead of 24 rows centered.
const TALL_HEIGHTS_WITH_MARK: [usize; GLYPH_H] = [3, 2, 3, 3, 3, 3, 2, 3];
/// First row of the digits when a mark is drawn: the mark ends on row 6 and one
/// row stays free between them.
const TOP_WITH_MARK: usize = 8;

fn layout(count: usize, mark: UnitMark) -> Layout {
    let size = ICON_SIZE as usize;
    // The largest scale (3, 2 or 1) whose text, with one scaled pixel between
    // characters, stays within the 30 central columns.
    let scale = (1..=3)
        .rev()
        .find(|&s| text_width(count, s) <= size - 2)
        .unwrap_or(1);
    if mark != UnitMark::None && scale == 3 {
        Layout {
            across: scale,
            heights: TALL_HEIGHTS_WITH_MARK,
            top: TOP_WITH_MARK,
        }
    } else {
        // Scale 2 (three characters) is 16 rows from row 8: below the mark as it is.
        Layout {
            across: scale,
            heights: [scale; GLYPH_H],
            top: (size - GLYPH_H * scale) / 2,
        }
    }
}

fn text_width(count: usize, scale: usize) -> usize {
    (count * GLYPH_W + count.saturating_sub(1)) * scale
}

/// The lit pixels of `text`, as (x, y), laid out for a picture with `mark`.
fn digit_pixels(text: &str, mark: UnitMark) -> Vec<(usize, usize)> {
    let size = ICON_SIZE as usize;
    let glyphs: Vec<_> = text.chars().filter_map(glyph).collect();
    if glyphs.is_empty() {
        return Vec::new();
    }
    let layout = layout(glyphs.len(), mark);
    let left = (size - text_width(glyphs.len(), layout.across).min(size)) / 2;
    let mut pixels = Vec::new();
    for (index, rows) in glyphs.iter().enumerate() {
        let origin = left + index * (GLYPH_W + 1) * layout.across;
        let mut y = layout.top;
        for (line, &height) in rows.iter().zip(&layout.heights) {
            for (col, _) in line.bytes().enumerate().filter(|&(_, b)| b == b'#') {
                for dy in 0..height {
                    for dx in 0..layout.across {
                        let x = origin + col * layout.across + dx;
                        if x < size {
                            pixels.push((x, y + dy));
                        }
                    }
                }
            }
            y += height;
        }
    }
    pixels
}

/// A rounded square in `style.background` (transparent corners, anti-aliased
/// edge) with `text` centered in `style.foreground`, and the unit `mark` small
/// in the top-right corner; as RGBA.
pub fn render(text: &str, mark: UnitMark, style: IconStyle) -> Vec<u8> {
    let size = ICON_SIZE as usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let mut pixel = style.background;
            pixel[3] = (f64::from(pixel[3]) * corner_coverage(x, y)).round() as u8;
            rgba[(y * size + x) * 4..][..4].copy_from_slice(&pixel);
        }
    }
    for (x, y) in digit_pixels(text, mark)
        .into_iter()
        .chain(mark_pixels(mark))
    {
        rgba[(y * size + x) * 4..][..4].copy_from_slice(&style.foreground);
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

/// `"CPU 45 °C · GPU 62 °C · RAM 48 %"`: items without a value are left out,
/// and with none left the tooltip is [`PRODUCT_NAME`]; a result over 127
/// UTF-16 units (the tray tooltip limit) is cut after the last whole item that
/// fits, plus `…`.
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
    if parts.is_empty() {
        return PRODUCT_NAME.to_owned();
    }
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

    const DEG: UnitMark = UnitMark::Degree;
    const PCT: UnitMark = UnitMark::Percent;
    const NONE: UnitMark = UnitMark::None;

    #[test]
    fn unit_mark_follows_the_unit() {
        // The mark is the degree sign whatever the temperature setting is.
        assert_eq!(unit_mark(Some(45.0), Unit::Celsius), DEG);
        assert_eq!(unit_mark(Some(45.0), Unit::Percent), PCT);
        assert_eq!(unit_mark(Some(45.0), Unit::Watt), NONE);
        assert_eq!(unit_mark(Some(45.0), Unit::Megahertz), NONE);
        assert_eq!(unit_mark(None, Unit::Celsius), NONE);
        assert_eq!(unit_mark(Some(f64::NAN), Unit::Percent), NONE);
        assert_eq!(unit_mark(Some(f64::INFINITY), Unit::Celsius), NONE);
    }

    #[test]
    fn render_is_rgba_32x32() {
        for mark in [NONE, DEG, PCT] {
            let img = render("45", mark, NEUTRAL);
            assert_eq!(img.len(), 4096);
            assert_eq!(px(&img, 0, 0)[3], 0, "corner is transparent");
            assert_eq!(px(&img, 16, 4), NEUTRAL.background);
            assert!(!fg_pixels(&img, NEUTRAL).is_empty());
        }
    }

    #[test]
    fn render_fits_three_digits_and_minus() {
        for mark in [NONE, DEG, PCT] {
            for text in ["212", "-99", "999", "88", "5", "—"] {
                let img = render(text, mark, NEUTRAL);
                let pixels = fg_pixels(&img, NEUTRAL);
                assert!(!pixels.is_empty(), "{text} draws something");
                assert!(
                    pixels.iter().all(|&(x, _)| x != 0 && x != ICON_SIZE - 1),
                    "{text} {mark:?} leaves columns 0 and 31 free"
                );
            }
        }
    }

    #[test]
    fn render_draws_two_digits_large() {
        for mark in [NONE, DEG, PCT] {
            let digits = digit_pixels("88", mark);
            let top = digits.iter().map(|&(_, y)| y).min().unwrap();
            let bottom = digits.iter().map(|&(_, y)| y).max().unwrap();
            assert!(bottom - top + 1 >= 16, "{mark:?}: rows {top}..={bottom}");
            // What the picture shows is what was computed.
            let img = render("88", mark, NEUTRAL);
            let rows = fg_pixels(&img, NEUTRAL);
            assert!(rows.iter().any(|&(_, y)| y as usize == bottom));
        }
    }

    #[test]
    fn render_distinguishes_glyphs() {
        assert_ne!(render("45", DEG, NEUTRAL), render("46", DEG, NEUTRAL));
        assert_ne!(render("-", NONE, NEUTRAL), render("—", NONE, NEUTRAL));
    }

    #[test]
    fn mark_is_drawn_top_right_without_touching_digits() {
        for mark in [DEG, PCT] {
            let marks = mark_pixels(mark);
            assert!(!marks.is_empty());
            assert!(
                marks.iter().all(|&(x, y)| (24..=29).contains(&x) && y <= 7),
                "{mark:?} sits in the top-right corner"
            );
            let left = marks.iter().map(|&(x, _)| x).min().unwrap();
            let right = marks.iter().map(|&(x, _)| x).max().unwrap();
            let width = right - left + 1;
            assert!((5..=7).contains(&width), "{mark:?} is {width} px wide");

            for text in ["88", "5", "45", "212", "-99", "999", "100", "-5", "0"] {
                let digits = digit_pixels(text, mark);
                assert!(!digits.is_empty(), "{text}");
                for &(dx, dy) in &digits {
                    for &(mx, my) in &marks {
                        assert!(
                            dx.abs_diff(mx) > 1 || dy.abs_diff(my) > 1,
                            "{text} {mark:?}: digit ({dx},{dy}) touches mark ({mx},{my})"
                        );
                    }
                }
                // The picture is exactly the digits plus the mark.
                let img = render(text, mark, NEUTRAL);
                let mut expected: Vec<(u32, u32)> = digits
                    .iter()
                    .chain(&marks)
                    .map(|&(x, y)| (x as u32, y as u32))
                    .collect();
                expected.sort_by_key(|&(x, y)| (y, x));
                expected.dedup();
                assert_eq!(fg_pixels(&img, NEUTRAL), expected, "{text} {mark:?}");
                assert!(
                    expected.iter().all(|&(x, _)| x != 0 && x != ICON_SIZE - 1),
                    "{text} {mark:?} leaves columns 0 and 31 free"
                );
            }
        }
    }

    #[test]
    fn degree_and_percent_marks_differ() {
        assert_ne!(mark_pixels(DEG), mark_pixels(PCT));
        assert_ne!(render("45", DEG, NEUTRAL), render("45", PCT, NEUTRAL));
        assert_ne!(render("45", DEG, NEUTRAL), render("45", NONE, NEUTRAL));
        assert_ne!(render("45", PCT, NEUTRAL), render("45", NONE, NEUTRAL));
    }

    #[test]
    fn no_mark_for_the_dash() {
        assert!(mark_pixels(NONE).is_empty());
        let mark = unit_mark(None, Unit::Celsius);
        assert_eq!(mark, NONE);
        let img = render("—", mark, NEUTRAL);
        // Only the dash bar is drawn: nothing near the corner.
        let pixels = fg_pixels(&img, NEUTRAL);
        assert!(!pixels.is_empty());
        assert!(pixels.iter().all(|&(_, y)| y > 8));
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

        // Nothing to show yet (the first tick): the product name, never an empty tooltip.
        assert_eq!(tooltip(Lang::En, &[], C), "OpenMonitor Advanced");
        let none = [
            item("tray.tooltip.cpu", None, Unit::Celsius),
            item("tray.tooltip.gpu", Some(f64::NAN), Unit::Celsius),
        ];
        assert_eq!(tooltip(Lang::It, &none, C), "OpenMonitor Advanced");

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
        for (text, mark) in [
            ("45", DEG),
            ("48", PCT),
            ("212", DEG),
            ("-99", DEG),
            ("7", PCT),
            ("120", NONE),
            ("—", NONE),
        ] {
            println!(
                "== {text} {mark:?}\n{}",
                ascii(&render(text, mark, NEUTRAL), NEUTRAL)
            );
        }
    }
}
