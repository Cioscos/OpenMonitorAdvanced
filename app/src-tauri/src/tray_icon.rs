//! Dynamic tray icon (a number on a rounded square in the color of the health
//! level), tooltip and alert texts, as pure functions.

use oma_core::format::FormatOptions;
use oma_core::model::{DeviceKind, Schema, Unit};
use oma_core::rules::{Alert, HealthReport, OverallLevel};
use oma_core::settings::{TemperatureUnit, ThroughputUnit};

use crate::i18n::{sensor_label, t, Lang};

pub const ICON_SIZE: u32 = 32;

/// The tray tooltip before any reading has a value (and at start-up).
pub const PRODUCT_NAME: &str = "OpenMonitor Advanced";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconStyle {
    pub background: [u8; 4],
    pub foreground: [u8; 4],
}

/// `--surface-2` with light (`--text`) digits: no alert and no valid data yet.
pub const NEUTRAL: IconStyle = IconStyle {
    background: [0x21, 0x17, 0x33, 0xff],
    foreground: [0xf5, 0xee, 0xfe, 0xff],
};

/// Dark digits on the status colors, for contrast.
const ON_STATUS: [u8; 4] = [0x0f, 0x0a, 0x1a, 0xff];

/// `--ok`.
pub const OK: IconStyle = IconStyle {
    background: [0x3e, 0xe8, 0xb5, 0xff],
    foreground: ON_STATUS,
};

/// `--warn`.
pub const WARN: IconStyle = IconStyle {
    background: [0xff, 0xc5, 0x3d, 0xff],
    foreground: ON_STATUS,
};

/// `--crit`.
pub const CRIT: IconStyle = IconStyle {
    background: [0xff, 0x4d, 0x4d, 0xff],
    foreground: ON_STATUS,
};

/// The icon's colors for the overall health level (spec §3.5).
pub fn style_for(level: OverallLevel) -> IconStyle {
    match level {
        OverallLevel::Neutral => NEUTRAL,
        OverallLevel::Ok => OK,
        OverallLevel::Warn => WARN,
        OverallLevel::Crit => CRIT,
    }
}

/// What the icon draws: a whole number, or a bar filled to a percentage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconContent {
    Text(String),
    /// A vertical bar filled to this percentage (0..=100).
    Bar(u8),
}

/// A finite percentage is a bar filled to the same whole number the tooltip
/// shows (0..=100); everything else is [`icon_text`].
pub fn icon_content(value: Option<f64>, unit: Unit, temperature: TemperatureUnit) -> IconContent {
    match value.filter(|v| v.is_finite()) {
        Some(v) if unit == Unit::Percent => IconContent::Bar(whole(v).clamp(0, 100) as u8),
        _ => IconContent::Text(icon_text(value, unit, temperature)),
    }
}

// The bar: a 16x24 outlined track, centered, with a fill area inset by the
// outline and a one pixel gap.
const BAR_W: usize = 16;
const BAR_H: usize = 24;
const BAR_OUTLINE: usize = 2;
const BAR_GAP: usize = 1;
const BAR_LEFT: usize = (ICON_SIZE as usize - BAR_W) / 2;
const BAR_TOP: usize = (ICON_SIZE as usize - BAR_H) / 2;
const BAR_FILL_TOP: usize = BAR_TOP + BAR_OUTLINE + BAR_GAP;
const BAR_FILL_BOTTOM: usize = BAR_TOP + BAR_H - 1 - BAR_OUTLINE - BAR_GAP;

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
/// edge) with the content drawn in `style.foreground`, as RGBA: a number
/// centered, or a bar filling up from the bottom. With `recording`, a red dot
/// with a dark ring is drawn last, in the top right corner (L9).
pub fn render(content: &IconContent, style: IconStyle, recording: bool) -> Vec<u8> {
    let mut rgba = match content {
        IconContent::Text(text) => render_text(text, style),
        IconContent::Bar(level) => render_bar(*level, style),
    };
    if recording {
        draw_recording_dot(&mut rgba);
    }
    rgba
}

/// `--crit`, the recording dot.
const DOT_COLOR: [u8; 4] = [0xff, 0x4d, 0x4d, 0xff];
/// `--bg`, the ring that keeps the dot apart from any background.
const DOT_RING: [u8; 4] = [0x0f, 0x0a, 0x1a, 0xff];
const DOT_CENTER: (f64, f64) = (26.5, 5.5);
const DOT_RADIUS: f64 = 4.5;
const DOT_RING_WIDTH: f64 = 1.0;

/// A filled circle with a ring, hard-edged on pixel centers, over whatever is
/// drawn already (also over the transparent corner).
fn draw_recording_dot(rgba: &mut [u8]) {
    let size = ICON_SIZE as usize;
    for y in 0..size {
        for x in 0..size {
            let distance = (x as f64 + 0.5 - DOT_CENTER.0).hypot(y as f64 + 0.5 - DOT_CENTER.1);
            let color = if distance <= DOT_RADIUS {
                DOT_COLOR
            } else if distance <= DOT_RADIUS + DOT_RING_WIDTH {
                DOT_RING
            } else {
                continue;
            };
            rgba[(y * size + x) * 4..][..4].copy_from_slice(&color);
        }
    }
}

/// Rows of the fill area lit for `level` percent: none at 0, at least one above.
fn filled_rows(level: u8) -> usize {
    let area = BAR_FILL_BOTTOM - BAR_FILL_TOP + 1;
    match level.min(100) {
        0 => 0,
        100.. => area,
        // Rounded, but never empty above 0 and never full below 100.
        level => ((usize::from(level) * area + 50) / 100).clamp(1, area - 1),
    }
}

fn render_bar(level: u8, style: IconStyle) -> Vec<u8> {
    let mut rgba = background(style);
    let size = ICON_SIZE as usize;
    let mut set = |x: usize, y: usize| {
        rgba[(y * size + x) * 4..][..4].copy_from_slice(&style.foreground);
    };
    // The outline of the track.
    for y in BAR_TOP..BAR_TOP + BAR_H {
        for x in BAR_LEFT..BAR_LEFT + BAR_W {
            let inner = (BAR_LEFT + BAR_OUTLINE..BAR_LEFT + BAR_W - BAR_OUTLINE).contains(&x)
                && (BAR_TOP + BAR_OUTLINE..BAR_TOP + BAR_H - BAR_OUTLINE).contains(&y);
            if !inner {
                set(x, y);
            }
        }
    }
    // The fill, from the bottom up.
    let inset = BAR_OUTLINE + BAR_GAP;
    for y in BAR_FILL_BOTTOM + 1 - filled_rows(level)..=BAR_FILL_BOTTOM {
        for x in BAR_LEFT + inset..BAR_LEFT + BAR_W - inset {
            set(x, y);
        }
    }
    rgba
}

/// The rounded square in `style.background`, transparent corners, anti-aliased edge.
fn background(style: IconStyle) -> Vec<u8> {
    let size = ICON_SIZE as usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let mut pixel = style.background;
            pixel[3] = (f64::from(pixel[3]) * corner_coverage(x, y)).round() as u8;
            rgba[(y * size + x) * 4..][..4].copy_from_slice(&pixel);
        }
    }
    rgba
}

fn render_text(text: &str, style: IconStyle) -> Vec<u8> {
    let size = ICON_SIZE as usize;
    let mut rgba = background(style);

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

/// `"CPU 45 °C · GPU 62 °C · RAM 48 %"`, after the verdict when there is one
/// (`"RTX 4080 overheating (92 °C) · CPU 45 °C · …"`): items without a value
/// are left out, and with nothing left the tooltip is [`PRODUCT_NAME`]; a
/// result over 127 UTF-16 units (the tray tooltip limit) is cut after the last
/// whole part that fits, plus `…`, so the verdict is the last to go.
pub fn tooltip(
    lang: Lang,
    verdict: Option<&str>,
    items: &[TooltipItem],
    temperature: TemperatureUnit,
) -> String {
    let values = items.iter().filter_map(|item| {
        let value = item.value.filter(|v| v.is_finite())?;
        let label = t(lang, item.label_key, &[]);
        Some(format!(
            "{label} {}",
            tooltip_value(value, item.unit, temperature)
        ))
    });
    let parts: Vec<String> = verdict
        .map(str::to_owned)
        .into_iter()
        .chain(values)
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

/// The text of one alert, shared by the tooltip verdict and the toast: its
/// `messageKey` with `{device}` (and the other report params), `{sensor}`
/// (the translated label), `{volume}` (the label's own name, such as "C:",
/// else the translated label), `{threshold}` and `{value}` in the chosen units;
/// `{value}` is "—" while the sensor has no value. Values read like the
/// tooltip's (`92 %`), so the verdict matches the values after it.
pub fn alert_text(
    lang: Lang,
    alert: &Alert,
    schema: &Schema,
    temperature: TemperatureUnit,
    throughput: ThroughputUnit,
) -> String {
    let rate = rate_for(schema, &alert.device_id, throughput);
    let value = if alert.valid {
        alert_value(lang, alert.value, alert.unit, temperature, rate)
    } else {
        DASH.to_owned()
    };
    let threshold = alert_value(lang, alert.threshold, alert.unit, temperature, rate);
    let sensor = sensor_label(lang, &alert.sensor_label);
    let volume = alert.sensor_label.arg.as_deref().unwrap_or(&sensor);
    // The computed params come first: `t` uses the first one with a name.
    let params: Vec<(&str, &str)> = [
        ("sensor", sensor.as_str()),
        ("volume", volume),
        ("threshold", threshold.as_str()),
        ("value", value.as_str()),
    ]
    .into_iter()
    .chain(alert.params.iter().map(|(k, v)| (k.as_str(), v.as_str())))
    .collect();
    t(lang, &alert.message_key, &params)
}

/// What the tray says before its values: nothing at `neutral` or `ok`, the
/// alert's text with one alert, "N problems" with more.
pub fn verdict(
    lang: Lang,
    report: &HealthReport,
    schema: &Schema,
    temperature: TemperatureUnit,
    throughput: ThroughputUnit,
) -> Option<String> {
    match report.level {
        OverallLevel::Neutral | OverallLevel::Ok => None,
        OverallLevel::Warn | OverallLevel::Crit => match report.alerts.as_slice() {
            [] => None,
            [alert] => Some(alert_text(lang, alert, schema, temperature, throughput)),
            alerts => {
                let count = alerts.len().to_string();
                Some(t(lang, "health.problems", &[("count", &count)]))
            }
        },
    }
}

/// Throughput follows the setting on network devices and is shown in bytes
/// elsewhere, like the device pages of the Advanced view. A device gone
/// from the schema (a retained alert) is recognized by its id.
fn rate_for(schema: &Schema, device_id: &str, throughput: ThroughputUnit) -> ThroughputUnit {
    let network = schema
        .devices
        .iter()
        .find(|device| device.id == device_id)
        .map_or_else(
            || device_id.starts_with("network/"),
            |device| device.kind == DeviceKind::Network,
        );
    if network {
        throughput
    } else {
        ThroughputUnit::Bytes
    }
}

/// A value in an alert's text: [`format_value`], with percentages in the
/// tray tooltip's style (`92 %`, ruling R-E).
fn alert_value(
    lang: Lang,
    value: Option<f64>,
    unit: Unit,
    temperature: TemperatureUnit,
    rate: ThroughputUnit,
) -> String {
    match value.filter(|v| v.is_finite()) {
        Some(v) if unit == Unit::Percent => {
            format!("{} %", oma_core::format::number(v, 0, lang == Lang::It))
        }
        _ => format_value(lang, value, unit, temperature, rate),
    }
}

/// A sensor value as `formatValue` in `app/src/lib/format.ts` shows it, with
/// `rate` for bytes per second; "—" when absent or not finite.
fn format_value(
    lang: Lang,
    value: Option<f64>,
    unit: Unit,
    temperature: TemperatureUnit,
    rate: ThroughputUnit,
) -> String {
    let opts = FormatOptions {
        decimal_comma: lang == Lang::It,
        temperature,
        rate,
        flag_on: t(lang, "flag.on", &[]),
        flag_off: t(lang, "flag.off", &[]),
        ..FormatOptions::default()
    };
    let (number, unit) = oma_core::format::format_value(value, unit, &opts);
    oma_core::format::join(&number, &unit)
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

    fn num(value: &str) -> IconContent {
        IconContent::Text(value.to_owned())
    }

    /// Lit pixels of the centre column strictly inside the bar's fill area.
    fn fill_probe(rgba: &[u8]) -> Vec<u32> {
        let x = ICON_SIZE / 2;
        (BAR_FILL_TOP as u32..=BAR_FILL_BOTTOM as u32)
            .filter(|&y| px(rgba, x, y) == NEUTRAL.foreground)
            .collect()
    }

    #[test]
    fn icon_content_is_a_bar_for_finite_percentages_only() {
        let bar = |v: f64| icon_content(Some(v), Unit::Percent, C);
        assert_eq!(bar(0.0), IconContent::Bar(0));
        assert_eq!(bar(45.4), IconContent::Bar(45));
        assert_eq!(bar(99.6), IconContent::Bar(100));
        assert_eq!(bar(-3.0), IconContent::Bar(0));
        assert_eq!(bar(250.0), IconContent::Bar(100));
        // Fahrenheit is for temperatures only.
        assert_eq!(
            icon_content(Some(50.0), Unit::Percent, TemperatureUnit::F),
            IconContent::Bar(50)
        );
        // Everything else stays a number.
        assert_eq!(icon_content(Some(45.4), Unit::Celsius, C), num("45"));
        assert_eq!(
            icon_content(Some(100.0), Unit::Celsius, TemperatureUnit::F),
            num("212")
        );
        assert_eq!(icon_content(Some(1500.0), Unit::Watt, C), num("999"));
        assert_eq!(icon_content(Some(3200.0), Unit::Megahertz, C), num("999"));
    }

    #[test]
    fn non_finite_percent_is_a_dash() {
        for value in [None, Some(f64::NAN), Some(f64::INFINITY)] {
            for unit in [Unit::Percent, Unit::Celsius, Unit::Watt] {
                assert_eq!(icon_content(value, unit, C), num("—"), "{value:?} {unit:?}");
            }
        }
    }

    #[test]
    fn percent_is_drawn_as_a_bar() {
        let full = BAR_FILL_BOTTOM - BAR_FILL_TOP + 1;
        let rows = |level: u8| fill_probe(&render(&IconContent::Bar(level), NEUTRAL, false));

        assert!(rows(0).is_empty(), "0 leaves the track empty");
        let one = rows(1);
        assert_eq!(one.len(), 1, "any value above 0 fills at least one row");
        assert_eq!(
            one,
            [BAR_FILL_BOTTOM as u32],
            "the fill grows from the bottom"
        );
        let half = rows(50);
        assert_eq!(half.len(), full / 2);
        assert_eq!(half.last(), Some(&(BAR_FILL_BOTTOM as u32)));
        assert_eq!(
            half.len() as u32,
            half.last().unwrap() - half[0] + 1,
            "contiguous"
        );
        assert_eq!(rows(100).len(), full, "100 fills the track");
        assert!(rows(99).len() < full, "only 100 fills the track");
        // Monotonic with the level.
        let heights: Vec<usize> = (0..=100).map(|l| rows(l).len()).collect();
        assert!(heights.windows(2).all(|w| w[0] <= w[1]));

        // A track, not a fill only: an outline of 14..=18 px by 22..=24 px, centred.
        let img = render(&IconContent::Bar(0), NEUTRAL, false);
        let pixels = fg_pixels(&img, NEUTRAL);
        let (min_x, max_x) = (
            pixels.iter().map(|p| p.0).min().unwrap(),
            pixels.iter().map(|p| p.0).max().unwrap(),
        );
        let (min_y, max_y) = (
            pixels.iter().map(|p| p.1).min().unwrap(),
            pixels.iter().map(|p| p.1).max().unwrap(),
        );
        assert!((14..=18).contains(&(max_x - min_x + 1)), "width");
        assert!((22..=24).contains(&(max_y - min_y + 1)), "height");
        assert_eq!(min_x, ICON_SIZE - 1 - max_x, "centred horizontally");
        assert_eq!(min_y, ICON_SIZE - 1 - max_y, "centred vertically");
        // No digits on the bar: the level is not written anywhere.
        assert_ne!(
            render(&IconContent::Bar(48), NEUTRAL, false),
            render(&num("48"), NEUTRAL, false)
        );
        assert!(pixels.len() < 200, "an empty track is only an outline");
    }

    #[test]
    fn bar_fits_the_icon() {
        for level in [0, 1, 50, 99, 100] {
            let img = render(&IconContent::Bar(level), NEUTRAL, false);
            let pixels = fg_pixels(&img, NEUTRAL);
            assert!(!pixels.is_empty());
            assert!(
                pixels.iter().all(|&(x, _)| x != 0 && x != ICON_SIZE - 1),
                "bar {level} leaves columns 0 and 31 free"
            );
            assert_eq!(px(&img, 0, 0)[3], 0, "corner is transparent");
            // Only the two style colours (plus the anti-aliased corners).
            for y in 3..ICON_SIZE - 3 {
                for x in 3..ICON_SIZE - 3 {
                    let p = px(&img, x, y);
                    assert!(p == NEUTRAL.foreground || p == NEUTRAL.background);
                }
            }
        }
    }

    #[test]
    fn temperatures_are_numbers_without_a_mark() {
        // A temperature is the number alone, 24 rows tall and centred as before.
        let c = icon_content(Some(45.0), Unit::Celsius, C);
        assert_eq!(c, num("45"));
        let pixels = fg_pixels(&render(&c, NEUTRAL, false), NEUTRAL);
        let (top, bottom) = (
            pixels.iter().map(|p| p.1).min().unwrap(),
            pixels.iter().map(|p| p.1).max().unwrap(),
        );
        assert_eq!((top, bottom), (4, 27));
        // Nothing in the top-right corner where a unit mark would sit.
        assert!(pixels.iter().all(|&(x, y)| !(x >= 24 && y < 4)));
        // Three characters stay at the smaller scale.
        let pixels = fg_pixels(&render(&num("212"), NEUTRAL, false), NEUTRAL);
        assert_eq!(pixels.iter().map(|p| p.1).min(), Some(8));
        assert_eq!(pixels.iter().map(|p| p.1).max(), Some(23));
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
        let img = render(&num("45"), NEUTRAL, false);
        assert_eq!(img.len(), 4096);
        assert_eq!(px(&img, 0, 0)[3], 0, "corner is transparent");
        assert_eq!(px(&img, 16, 2), NEUTRAL.background);
        assert!(!fg_pixels(&img, NEUTRAL).is_empty());
    }

    #[test]
    fn render_fits_three_digits_and_minus() {
        for text in ["212", "-99", "999", "—"] {
            let img = render(&num(text), NEUTRAL, false);
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
        let img = render(&num("88"), NEUTRAL, false);
        let pixels = fg_pixels(&img, NEUTRAL);
        let top = pixels.iter().map(|&(_, y)| y).min().unwrap();
        let bottom = pixels.iter().map(|&(_, y)| y).max().unwrap();
        assert!(bottom - top + 1 >= 16, "rows {top}..={bottom}");
    }

    #[test]
    fn render_distinguishes_glyphs() {
        assert_ne!(
            render(&num("45"), NEUTRAL, false),
            render(&num("46"), NEUTRAL, false)
        );
        assert_ne!(
            render(&num("-"), NEUTRAL, false),
            render(&num("—"), NEUTRAL, false)
        );
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
            tooltip(Lang::En, None, &items, C),
            "CPU 45 °C · GPU 62 °C · RAM 48 %"
        );
        assert_eq!(
            tooltip(Lang::En, None, &items, TemperatureUnit::F),
            "CPU 113 °F · GPU 143 °F · RAM 48 %"
        );

        let missing = [
            item("tray.tooltip.cpu", Some(45.0), Unit::Celsius),
            item("tray.tooltip.gpu", None, Unit::Celsius),
            item("tray.tooltip.ram", Some(48.0), Unit::Percent),
        ];
        assert_eq!(tooltip(Lang::En, None, &missing, C), "CPU 45 °C · RAM 48 %");

        // Nothing to show yet (the first tick): the product name, never an empty tooltip.
        assert_eq!(tooltip(Lang::En, None, &[], C), "OpenMonitor Advanced");
        let none = [
            item("tray.tooltip.cpu", None, Unit::Celsius),
            item("tray.tooltip.gpu", Some(f64::NAN), Unit::Celsius),
        ];
        assert_eq!(tooltip(Lang::It, None, &none, C), "OpenMonitor Advanced");

        // Unknown keys are shown as they are, which makes long labels easy to test.
        const LONG: &str = "a-very-long-label-that-eats-the-tooltip-budget-of-the-tray";
        let long = [
            item(LONG, Some(1.0), Unit::Percent),
            item(LONG, Some(2.0), Unit::Percent),
            item(LONG, Some(3.0), Unit::Percent),
        ];
        let text = tooltip(Lang::En, None, &long, C);
        assert!(text.encode_utf16().count() <= 127, "{text}");
        assert!(text.ends_with('…'), "{text}");
        assert!(text.starts_with(&format!("{LONG} 1 %")), "{text}");
        assert!(!text.contains("3 %"), "{text}");
    }

    #[test]
    fn tooltip_truncates_a_single_oversized_item() {
        let huge: &'static str = Box::leak("x".repeat(200).into_boxed_str());
        let text = tooltip(Lang::En, None, &[item(huge, Some(1.0), Unit::Percent)], C);
        assert!(text.encode_utf16().count() <= 127);
        assert!(text.ends_with('…'));
    }

    // --- Health level colors and the verdict ---

    use std::collections::BTreeMap;

    use oma_core::model::{Device, DeviceKind, Label};
    use oma_core::rules::{Coverage, Level};

    const B: ThroughputUnit = ThroughputUnit::Bits;
    const GPU: &str = "gpu/pci-0000:01:00.0";
    const NIC: &str = "network/{1234}";

    fn schema() -> Schema {
        let device = |id: &str, kind, name: &str| Device {
            id: id.to_owned(),
            kind,
            name: name.to_owned(),
            vendor: None,
            properties: BTreeMap::new(),
        };
        Schema {
            revision: 3,
            devices: vec![
                device(GPU, DeviceKind::Gpu, "RTX 4080"),
                device(NIC, DeviceKind::Network, "Ethernet"),
                device("storage/0", DeviceKind::Storage, "Samsung 990"),
            ],
            sensors: Vec::new(),
        }
    }

    fn alert(rule_id: &str, device_id: &str, unit: Unit, level: Level, value: f64) -> Alert {
        let device = schema()
            .devices
            .iter()
            .find(|d| d.id == device_id)
            .map_or_else(|| device_id.to_owned(), |d| d.name.clone());
        Alert {
            rule_id: rule_id.to_owned(),
            sensor_id: format!("{device_id}/temperature/core"),
            device_id: device_id.to_owned(),
            unit,
            sensor_label: Label::new("gpu.temperature.core"),
            level,
            value: Some(value),
            threshold: Some(90.0),
            since_ms: 1_000,
            valid: true,
            last_valid_ms: Some(1_000),
            message_key: format!("rule.{rule_id}.message"),
            params: BTreeMap::from([("device".to_owned(), device)]),
        }
    }

    fn gpu_hot() -> Alert {
        alert("gpu-temp", GPU, Unit::Celsius, Level::Crit, 92.4)
    }

    fn report(level: OverallLevel, alerts: Vec<Alert>) -> HealthReport {
        HealthReport {
            level,
            since_ms: 1_000,
            revision: 4,
            coverage: Coverage::Complete,
            unavailable_targets: Vec::new(),
            alerts,
        }
    }

    fn opaque(hex: u32) -> [u8; 4] {
        let [_, r, g, b] = hex.to_be_bytes();
        [r, g, b, 0xff]
    }

    #[test]
    fn level_styles_use_the_palette() {
        const DARK: u32 = 0x0f0a1a;
        const LIGHT: u32 = 0xf5eefe;
        let cases = [
            (OverallLevel::Neutral, 0x211733, LIGHT),
            (OverallLevel::Ok, 0x3ee8b5, DARK),
            (OverallLevel::Warn, 0xffc53d, DARK),
            (OverallLevel::Crit, 0xff4d4d, DARK),
        ];
        for (level, background, foreground) in cases {
            let style = style_for(level);
            assert_eq!(style.background, opaque(background), "{level:?}");
            assert_eq!(style.foreground, opaque(foreground), "{level:?}");
            // The drawn pixels: the square in the background, digits and bar
            // in the foreground.
            let digits = render(&num("88"), style, false);
            assert_eq!(px(&digits, 16, 2), opaque(background), "{level:?}");
            assert!(!fg_pixels(&digits, style).is_empty(), "{level:?}");
            let bar = render(&IconContent::Bar(100), style, false);
            assert_eq!(px(&bar, 16, 20), opaque(foreground), "{level:?}");
        }
        assert_eq!(style_for(OverallLevel::Neutral), NEUTRAL);
        assert_eq!(
            [
                style_for(OverallLevel::Ok),
                style_for(OverallLevel::Warn),
                style_for(OverallLevel::Crit)
            ],
            [OK, WARN, CRIT]
        );
    }

    #[test]
    fn verdict_with_one_alert() {
        let one = report(OverallLevel::Crit, vec![gpu_hot()]);
        assert_eq!(
            verdict(Lang::En, &one, &schema(), C, B).as_deref(),
            Some("RTX 4080 overheating (92 °C)")
        );
        assert_eq!(
            verdict(Lang::It, &one, &schema(), C, B).as_deref(),
            Some("RTX 4080 surriscaldata (92 °C)")
        );
        // A value no longer measured is a dash, never the old number.
        let mut lost = gpu_hot();
        lost.valid = false;
        assert_eq!(
            verdict(
                Lang::En,
                &report(OverallLevel::Crit, vec![lost]),
                &schema(),
                C,
                B
            )
            .as_deref(),
            Some("RTX 4080 overheating (—)")
        );
    }

    #[test]
    fn verdict_with_many_alerts() {
        let many = report(
            OverallLevel::Crit,
            vec![
                gpu_hot(),
                alert("disk-temp", "storage/0", Unit::Celsius, Level::Warn, 71.0),
                alert("volume-used", "storage/0", Unit::Percent, Level::Warn, 95.0),
            ],
        );
        assert_eq!(
            verdict(Lang::En, &many, &schema(), C, B).as_deref(),
            Some("3 problems")
        );
        assert_eq!(
            verdict(Lang::It, &many, &schema(), C, B).as_deref(),
            Some("3 problemi")
        );
    }

    #[test]
    fn verdict_absent_when_ok() {
        for level in [OverallLevel::Neutral, OverallLevel::Ok] {
            assert_eq!(
                verdict(Lang::En, &report(level, Vec::new()), &schema(), C, B),
                None
            );
        }
    }

    fn ram_full() -> Alert {
        let mut ram = alert("ram-used", "memory/0", Unit::Percent, Level::Warn, 92.4);
        ram.sensor_label = Label::new("memory.load");
        ram.params = BTreeMap::new();
        ram
    }

    #[test]
    fn verdict_uses_the_tray_percent_style() {
        let one = report(OverallLevel::Warn, vec![ram_full()]);
        let verdict_en = verdict(Lang::En, &one, &schema(), C, B);
        assert_eq!(verdict_en.as_deref(), Some("Memory almost full (92 %)"));
        assert_eq!(
            verdict(Lang::It, &one, &schema(), C, B).as_deref(),
            Some("Memoria quasi piena (92 %)")
        );
        // The verdict and the values after it read the same way.
        let items = [item("tray.tooltip.ram", Some(92.4), Unit::Percent)];
        assert_eq!(
            tooltip(Lang::En, verdict_en.as_deref(), &items, C),
            "Memory almost full (92 %) · RAM 92 %"
        );
        // The toast body shares the text, threshold included.
        let mut custom = ram_full();
        custom.message_key = "rule.custom.above".to_owned();
        assert_eq!(
            alert_text(Lang::En, &custom, &schema(), C, B),
            format!(
                "{} above 90 % (92 %)",
                sensor_label(Lang::En, &custom.sensor_label)
            )
        );
    }

    #[test]
    fn volume_messages_name_the_volume() {
        let mut volume = alert("volume-used", "storage/0", Unit::Percent, Level::Warn, 95.0);
        volume.sensor_label = Label::with_arg("storage.volumeUsed", "C:");
        let text = |lang, volume: &Alert| alert_text(lang, volume, &schema(), C, B);
        assert_eq!(text(Lang::En, &volume), "Volume C: almost full (95 %)");
        assert_eq!(text(Lang::It, &volume), "Volume C: quasi pieno (95 %)");
        // Without the volume's own name, the whole label stands in for it.
        volume.sensor_label = Label::new("memory.load");
        let label = sensor_label(Lang::En, &volume.sensor_label);
        assert_eq!(
            text(Lang::En, &volume),
            format!("Volume {label} almost full (95 %)")
        );
    }

    #[test]
    fn verdict_formats_fahrenheit() {
        let one = report(OverallLevel::Crit, vec![gpu_hot()]);
        assert_eq!(
            verdict(Lang::En, &one, &schema(), TemperatureUnit::F, B).as_deref(),
            Some("RTX 4080 overheating (198 °F)")
        );
    }

    #[test]
    fn custom_rules_name_the_sensor_and_the_threshold() {
        let mut above = gpu_hot();
        above.rule_id = "custom-00000000-0000-4000-8000-000000000001".to_owned();
        above.message_key = "rule.custom.above".to_owned();
        let text =
            |alert: &Alert, lang, temperature| alert_text(lang, alert, &schema(), temperature, B);
        assert_eq!(
            text(&above, Lang::En, C),
            "Core temperature above 90 °C (92 °C)"
        );
        assert_eq!(
            text(&above, Lang::It, TemperatureUnit::F),
            "Temperatura core sopra 194 °F (198 °F)"
        );
        let mut below = above.clone();
        below.message_key = "rule.custom.below".to_owned();
        assert_eq!(
            text(&below, Lang::En, C),
            "Core temperature below 90 °C (92 °C)"
        );
        let mut flag = above.clone();
        flag.message_key = "rule.custom.flag".to_owned();
        flag.unit = Unit::Boolean;
        flag.sensor_label = Label::new("gpu.throttle.thermal");
        flag.threshold = None;
        flag.value = Some(1.0);
        assert_eq!(
            text(&flag, Lang::En, C),
            "Thermal throttling on RTX 4080: active"
        );
        assert_eq!(
            text(&flag, Lang::It, C),
            "Limitazione termica su RTX 4080: attivo"
        );
    }

    #[test]
    fn throughput_follows_the_setting_on_network_devices_only() {
        let mut net = alert(
            "custom-x",
            NIC,
            Unit::BytesPerSecond,
            Level::Warn,
            12_500_000.0,
        );
        net.message_key = "rule.custom.above".to_owned();
        net.sensor_label = Label::new("network.down");
        net.threshold = Some(10_000_000.0);
        assert_eq!(
            alert_text(Lang::En, &net, &schema(), C, ThroughputUnit::Bits),
            "Download above 80 Mbit/s (100 Mbit/s)"
        );
        assert_eq!(
            alert_text(Lang::It, &net, &schema(), C, ThroughputUnit::Bytes),
            "Download sopra 9,5 MB/s (11,9 MB/s)"
        );
        // Disks show bytes, like their page in the Advanced view.
        let mut disk = net.clone();
        disk.device_id = "storage/0".to_owned();
        assert_eq!(
            alert_text(Lang::En, &disk, &schema(), C, ThroughputUnit::Bits),
            "Download above 9.5 MB/s (11.9 MB/s)"
        );
        // A network device gone from the schema is still recognized by its id.
        let mut gone = net.clone();
        gone.device_id = "network/{gone}".to_owned();
        assert_eq!(
            alert_text(Lang::En, &gone, &schema(), C, ThroughputUnit::Bits),
            "Download above 80 Mbit/s (100 Mbit/s)"
        );
    }

    #[test]
    fn format_value_mirrors_the_ui_formatter() {
        let f = |value: f64, unit, lang| format_value(lang, Some(value), unit, C, B);
        assert_eq!(f(48.0, Unit::Percent, Lang::En), "48%");
        assert_eq!(f(3_200.0, Unit::Megahertz, Lang::En), "3.20 GHz");
        assert_eq!(f(3_200.0, Unit::Megahertz, Lang::It), "3,20 GHz");
        assert_eq!(f(800.0, Unit::Megahertz, Lang::En), "800 MHz");
        assert_eq!(f(1.2346, Unit::Volt, Lang::En), "1.235 V");
        assert_eq!(f(12.34, Unit::Ampere, Lang::It), "12,3 A");
        assert_eq!(f(1_500.0, Unit::Rpm, Lang::En), "1,500 RPM");
        assert_eq!(f(1_500.0, Unit::Rpm, Lang::It), "1500 RPM");
        assert_eq!(f(15_000.0, Unit::Count, Lang::It), "15.000");
        assert_eq!(f(1_234_567.0, Unit::Hours, Lang::En), "1,234,567 h");
        assert_eq!(f(512.0, Unit::Bytes, Lang::En), "512 B");
        assert_eq!(f(1_536.0, Unit::Bytes, Lang::En), "1.5 KB");
        assert_eq!(f(4.0, Unit::PcieGeneration, Lang::En), "Gen 4");
        assert_eq!(f(16.0, Unit::Lanes, Lang::En), "x16");
        assert_eq!(f(1.0, Unit::Boolean, Lang::En), "Active");
        assert_eq!(f(0.0, Unit::Boolean, Lang::It), "No");
        assert_eq!(f(2_500.0, Unit::Joule, Lang::En), "2.5 kJ");
        assert_eq!(f(1_000.0, Unit::BitsPerSecond, Lang::En), "1.0 kbit/s");
        assert_eq!(f(-0.2, Unit::Celsius, Lang::En), "0 °C");
        assert_eq!(f(-5.6, Unit::Celsius, Lang::En), "-6 °C");
        assert_eq!(format_value(Lang::En, None, Unit::Celsius, C, B), "—");
        assert_eq!(
            format_value(Lang::En, Some(f64::NAN), Unit::Percent, C, B),
            "—"
        );
    }

    #[test]
    fn tooltip_starts_with_the_verdict_and_keeps_the_limit() {
        let items = [
            item("tray.tooltip.cpu", Some(45.0), Unit::Celsius),
            item("tray.tooltip.gpu", Some(92.0), Unit::Celsius),
        ];
        assert_eq!(
            tooltip(Lang::En, Some("RTX 4080 overheating (92 °C)"), &items, C),
            "RTX 4080 overheating (92 °C) · CPU 45 °C · GPU 92 °C"
        );
        // A verdict without any value to show stands alone.
        assert_eq!(tooltip(Lang::En, Some("2 problems"), &[], C), "2 problems");
        // A long verdict keeps its start; the values are dropped first.
        let long = "x".repeat(120);
        let text = tooltip(Lang::En, Some(&long), &items, C);
        assert!(text.encode_utf16().count() <= 127, "{text}");
        assert!(text.starts_with(&long), "{text}");
        let huge = "y".repeat(300);
        let text = tooltip(Lang::En, Some(&huge), &items, C);
        assert!(text.encode_utf16().count() <= 127, "{text}");
        assert!(text.starts_with("yyy") && text.ends_with('…'), "{text}");
    }

    #[test]
    #[ignore = "prints the icons for a visual check"]
    fn print_icons() {
        let contents = [
            ("45 C", num("45")),
            ("212", num("212")),
            ("bar 0", IconContent::Bar(0)),
            ("bar 7", IconContent::Bar(7)),
            ("bar 48", IconContent::Bar(48)),
            ("bar 100", IconContent::Bar(100)),
            ("dash", num("—")),
        ];
        for (name, content) in contents {
            println!(
                "== {name}
{}",
                ascii(&render(&content, NEUTRAL, false), NEUTRAL)
            );
        }
    }

    const DOT: [u8; 4] = [0xff, 0x4d, 0x4d, 0xff];
    const RING: [u8; 4] = [0x0f, 0x0a, 0x1a, 0xff];

    #[test]
    fn recording_dot_is_drawn_in_the_corner() {
        let plain = render(&num("48"), NEUTRAL, false);
        let dotted = render(&num("48"), NEUTRAL, true);
        assert_eq!(px(&dotted, 26, 5), DOT);
        assert_eq!(px(&dotted, 26, 0), RING);
        assert_eq!(px(&dotted, 21, 5), RING);
        // Only the corner changes; without the dot the icon is the M5b one.
        assert_ne!(plain, dotted);
        assert_eq!(px(&plain, 26, 5), NEUTRAL.background);
        for y in 12..32 {
            for x in 0..32 {
                assert_eq!(px(&plain, x, y), px(&dotted, x, y), "({x}, {y})");
            }
        }
    }

    #[test]
    fn dot_stays_visible_on_the_critical_background() {
        for content in [num("48"), IconContent::Bar(100)] {
            let img = render(&content, CRIT, true);
            assert_eq!(px(&img, 26, 5), DOT);
            assert_eq!(
                px(&img, 21, 5),
                RING,
                "the ring separates it from the square"
            );
            assert_eq!(px(&img, 26, 0), RING);
        }
    }
}
