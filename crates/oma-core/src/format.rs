//! Value formatting shared by the tray and the overlay: the rules of
//! `formatValue` in `app/src/lib/format.ts`, with the number and the unit
//! returned apart so the overlay can draw them with different styles.

use crate::model::Unit;
use crate::overlay::{FrameMetric, UnitChoice};
use crate::settings::{TemperatureUnit, ThroughputUnit};

pub const DASH: &str = "\u{2014}";

#[derive(Debug, Clone)]
pub struct FormatOptions {
    /// Italian-style numbers: `,` as decimal separator, `.` for thousands.
    pub decimal_comma: bool,
    pub temperature: TemperatureUnit,
    pub rate: ThroughputUnit,
    pub flag_on: String,
    pub flag_off: String,
    /// Fixed number of decimals, replacing the unit's default.
    pub decimals: Option<u8>,
    /// Fixed display unit; one that does not fit the sensor's unit is `auto`.
    pub unit: UnitChoice,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            decimal_comma: false,
            temperature: TemperatureUnit::default(),
            rate: ThroughputUnit::default(),
            flag_on: "On".to_owned(),
            flag_off: "Off".to_owned(),
            decimals: None,
            unit: UnitChoice::Auto,
        }
    }
}

const BYTE_UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
const BIT_UNITS: [&str; 5] = ["bit/s", "kbit/s", "Mbit/s", "Gbit/s", "Tbit/s"];
const JOULE_UNITS: [&str; 4] = ["J", "kJ", "MJ", "GJ"];

/// A sensor value as `formatValue` in `app/src/lib/format.ts` shows it, as
/// number and unit; the dash when absent or not finite. Texts without a
/// separate unit (flags, `Gen 4`, `x16`) come back whole in the number.
pub fn format_value(value: Option<f64>, unit: Unit, opts: &FormatOptions) -> (String, String) {
    let Some(v) = value.filter(|v| v.is_finite()) else {
        return (DASH.to_owned(), String::new());
    };
    // The fixed decimals, or the unit's own.
    let n = |value: f64, default: usize| {
        number(
            value,
            opts.decimals.map_or(default, usize::from),
            opts.decimal_comma,
        )
    };
    let plain = |text: String| (text, String::new());
    let with = |text: String, unit: &str| (text, unit.to_owned());
    match unit {
        Unit::Celsius => {
            let (shown, symbol) = match opts.temperature {
                TemperatureUnit::C => (v, "\u{b0}C"),
                TemperatureUnit::F => (v * 9.0 / 5.0 + 32.0, "\u{b0}F"),
            };
            with(n(shown, 0), symbol)
        }
        Unit::Percent => with(n(v, 0), "%"),
        Unit::Megahertz => match opts.unit {
            UnitChoice::GHz => with(n(v / 1000.0, 2), "GHz"),
            UnitChoice::MHz => with(n(v, 0), "MHz"),
            _ if v >= 1000.0 => with(n(v / 1000.0, 2), "GHz"),
            _ => with(n(v, 0), "MHz"),
        },
        Unit::Watt => with(n(v, 0), "W"),
        Unit::Volt => with(n(v, 3), "V"),
        Unit::Ampere => with(n(v, 1), "A"),
        Unit::Rpm => with(n(v, 0), "RPM"),
        Unit::Bytes => match fixed_bytes(opts.unit) {
            Some(index) => bytes_fixed(v, index, "", opts),
            None => stepped(v, 1024.0, &BYTE_UNITS, "", opts),
        },
        Unit::BytesPerSecond => throughput(v, opts.rate, opts),
        Unit::BitsPerSecond => throughput(v / 8.0, ThroughputUnit::Bits, opts),
        Unit::Joule => stepped(v, 1000.0, &JOULE_UNITS, "", opts),
        Unit::Boolean => plain(if v >= 0.5 {
            opts.flag_on.clone()
        } else {
            opts.flag_off.clone()
        }),
        // `Math.round`: half up.
        Unit::PcieGeneration => plain(format!("Gen {}", (v + 0.5).floor())),
        Unit::Lanes => plain(format!("x{}", (v + 0.5).floor())),
        Unit::Hours => with(n(v, 0), "h"),
        Unit::Count => plain(n(v, 0)),
    }
}

/// A frame metric with its unit: FPS with no decimals, times in milliseconds
/// with one, the generation multiplier as `×2.0`, the stutter as a count.
/// `Bound` is text translated by the app and never comes through here.
pub fn format_frame_metric(
    metric: FrameMetric,
    value: Option<f64>,
    opts: &FormatOptions,
) -> (String, String) {
    let Some(v) = value.filter(|v| v.is_finite()) else {
        return (DASH.to_owned(), String::new());
    };
    let comma = opts.decimal_comma;
    match metric {
        FrameMetric::FpsDisplayed
        | FrameMetric::FpsRendered
        | FrameMetric::FpsPresented
        | FrameMetric::Low1
        | FrameMetric::Low01 => (number(v, 0, comma), "FPS".to_owned()),
        FrameMetric::FrametimeDisplayed
        | FrameMetric::FrametimeApp
        | FrameMetric::LatencyPc
        | FrameMetric::LatencyDisplay => (number(v, 1, comma), "ms".to_owned()),
        FrameMetric::FgMultiplier => (format!("\u{d7}{}", number(v, 1, comma)), String::new()),
        FrameMetric::Stutter => (number(v, 0, comma), String::new()),
        FrameMetric::Bound => (DASH.to_owned(), String::new()),
    }
}

/// Number and unit as one text: no space before `%`, none when there is no unit.
pub fn join(number: &str, unit: &str) -> String {
    match unit {
        "" => number.to_owned(),
        "%" => format!("{number}%"),
        _ => format!("{number} {unit}"),
    }
}

/// `value` divided by `step` while it reaches it, with the matching unit.
fn scaled(mut value: f64, step: f64, units: &[&str]) -> (f64, usize) {
    let mut unit = 0;
    while value.abs() >= step && unit < units.len() - 1 {
        value /= step;
        unit += 1;
    }
    (value, unit)
}

/// Index in [`BYTE_UNITS`] of a fixed byte unit.
fn fixed_bytes(choice: UnitChoice) -> Option<usize> {
    match choice {
        UnitChoice::B => Some(0),
        UnitChoice::KB => Some(1),
        UnitChoice::MB => Some(2),
        UnitChoice::GB => Some(3),
        UnitChoice::TB => Some(4),
        _ => None,
    }
}

/// Index in [`BIT_UNITS`] of a fixed bit-rate unit.
fn fixed_bits(choice: UnitChoice) -> Option<usize> {
    match choice {
        UnitChoice::BitS => Some(0),
        UnitChoice::KbitS => Some(1),
        UnitChoice::MbitS => Some(2),
        UnitChoice::GbitS => Some(3),
        _ => None,
    }
}

/// `formatBytes` and `formatEnergy`: a decimal only between the first step
/// and 100. `suffix` is appended to the unit (`/s`).
fn stepped(
    value: f64,
    step: f64,
    units: &[&str],
    suffix: &str,
    opts: &FormatOptions,
) -> (String, String) {
    let (value, unit) = scaled(value, step, units);
    let default = usize::from(unit != 0 && value < 100.0);
    let digits = opts.decimals.map_or(default, usize::from);
    (
        number(value, digits, opts.decimal_comma),
        format!("{}{suffix}", units[unit]),
    )
}

/// A byte count in a fixed binary unit (`index` in [`BYTE_UNITS`]).
fn bytes_fixed(bytes: f64, index: usize, suffix: &str, opts: &FormatOptions) -> (String, String) {
    let value = bytes / 1024f64.powi(index as i32);
    let default = usize::from(index != 0 && value.abs() < 100.0);
    let digits = opts.decimals.map_or(default, usize::from);
    (
        number(value, digits, opts.decimal_comma),
        format!("{}{suffix}", BYTE_UNITS[index]),
    )
}

/// `formatRate`: bits with decimal steps, or bytes with binary steps; a fixed
/// unit of either kind replaces the `rate` setting.
fn throughput(
    bytes_per_second: f64,
    rate: ThroughputUnit,
    opts: &FormatOptions,
) -> (String, String) {
    if let Some(index) = fixed_bytes(opts.unit) {
        return bytes_fixed(bytes_per_second, index, "/s", opts);
    }
    let bits = |fixed: Option<usize>| {
        let bits = bytes_per_second * 8.0;
        let (value, unit) = match fixed {
            Some(i) => (bits / 1000f64.powi(i as i32), i),
            None => scaled(bits, 1000.0, &BIT_UNITS),
        };
        let default = usize::from(value.abs() < 10.0);
        let digits = opts.decimals.map_or(default, usize::from);
        (
            number(value, digits, opts.decimal_comma),
            BIT_UNITS[unit].to_owned(),
        )
    };
    if let Some(index) = fixed_bits(opts.unit) {
        return bits(Some(index));
    }
    match rate {
        ThroughputUnit::Bytes => stepped(bytes_per_second, 1024.0, &BYTE_UNITS, "/s", opts),
        ThroughputUnit::Bits => bits(None),
    }
}

/// `Intl.NumberFormat` for `en` and `it`: `digits` decimals rounded half away
/// from zero (`format!` would round ties to even), the language's decimal
/// separator, and thousands grouped from four digits in English, five in
/// Italian. A value rounded to zero has no sign.
pub fn number(value: f64, digits: usize, decimal_comma: bool) -> String {
    let scale = 10f64.powi(digits as i32);
    let rounded = (value * scale).round() / scale;
    let text = format!("{:.*}", digits, rounded.abs());
    let (int, frac) = text.split_once('.').unwrap_or((text.as_str(), ""));
    let (group, decimal, grouping_from) = if decimal_comma {
        ('.', ',', 5)
    } else {
        (',', '.', 4)
    };
    let mut out = String::with_capacity(text.len() + 4);
    if rounded < 0.0 {
        out.push('-');
    }
    for (i, c) in int.chars().enumerate() {
        if int.len() >= grouping_from && i > 0 && (int.len() - i) % 3 == 0 {
            out.push(group);
        }
        out.push(c);
    }
    if !frac.is_empty() {
        out.push(decimal);
        out.push_str(frac);
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> FormatOptions {
        FormatOptions::default()
    }

    fn fmt(v: f64, unit: Unit, o: &FormatOptions) -> String {
        let (n, u) = format_value(Some(v), unit, o);
        join(&n, &u)
    }

    #[test]
    fn fixed_unit_converts_mb_gb_mhz_ghz() {
        let o = |unit| FormatOptions { unit, ..opts() };
        assert_eq!(
            format_value(Some(3_145_728.0), Unit::Bytes, &o(UnitChoice::MB)),
            ("3.0".into(), "MB".into())
        );
        assert_eq!(
            format_value(Some(3_145_728.0), Unit::Bytes, &o(UnitChoice::GB)),
            ("0.0".into(), "GB".into())
        );
        assert_eq!(
            format_value(Some(2_147_483_648.0), Unit::Bytes, &o(UnitChoice::GB)),
            ("2.0".into(), "GB".into())
        );
        assert_eq!(
            format_value(Some(2_000_000.0), Unit::BytesPerSecond, &o(UnitChoice::MB)),
            ("1.9".into(), "MB/s".into())
        );
        assert_eq!(
            format_value(Some(3_200.0), Unit::Megahertz, &o(UnitChoice::GHz)),
            ("3.20".into(), "GHz".into())
        );
        assert_eq!(
            format_value(Some(3_200.0), Unit::Megahertz, &o(UnitChoice::MHz)),
            ("3,200".into(), "MHz".into())
        );
        assert_eq!(
            format_value(Some(1.5), Unit::Megahertz, &o(UnitChoice::GHz)),
            ("0.00".into(), "GHz".into())
        );
        // Incompatible choices fall back to auto.
        assert_eq!(fmt(48.0, Unit::Percent, &o(UnitChoice::GB)), "48%");
        assert_eq!(fmt(800.0, Unit::Megahertz, &o(UnitChoice::MB)), "800 MHz");
    }

    #[test]
    fn decimals_override() {
        let o = FormatOptions {
            decimals: Some(2),
            ..opts()
        };
        assert_eq!(fmt(48.0, Unit::Percent, &o), "48.00%");
        assert_eq!(fmt(1.2346, Unit::Volt, &o), "1.23 V");
        assert_eq!(fmt(1_536.0, Unit::Bytes, &o), "1.50 KB");
        let zero = FormatOptions {
            decimals: Some(0),
            ..opts()
        };
        assert_eq!(fmt(1.2346, Unit::Volt, &zero), "1 V");
    }

    #[test]
    fn decimal_comma_for_italian() {
        let o = FormatOptions {
            decimal_comma: true,
            ..opts()
        };
        assert_eq!(format_value(Some(12.5), Unit::Ampere, &o).0, "12,5");
        assert_eq!(fmt(15_000.0, Unit::Count, &o), "15.000");
    }

    #[test]
    fn absent_and_flags() {
        assert_eq!(
            format_value(None, Unit::Celsius, &opts()),
            (DASH.into(), String::new())
        );
        assert_eq!(format_value(Some(f64::NAN), Unit::Percent, &opts()).0, DASH);
        let o = FormatOptions {
            flag_on: "Sì".into(),
            flag_off: "No".into(),
            ..opts()
        };
        assert_eq!(fmt(1.0, Unit::Boolean, &o), "Sì");
        assert_eq!(fmt(0.0, Unit::Boolean, &o), "No");
    }

    #[test]
    fn frame_metric_formats() {
        let it = FormatOptions {
            decimal_comma: true,
            ..opts()
        };
        assert_eq!(
            format_frame_metric(FrameMetric::FpsDisplayed, Some(143.6), &it),
            ("144".into(), "FPS".into())
        );
        assert_eq!(
            format_frame_metric(FrameMetric::FrametimeDisplayed, Some(6.95), &it),
            ("7,0".into(), "ms".into())
        );
        assert_eq!(
            format_frame_metric(FrameMetric::FgMultiplier, Some(1.99), &it),
            ("×2,0".into(), String::new())
        );
        assert_eq!(
            format_frame_metric(FrameMetric::Stutter, Some(3.0), &it),
            ("3".into(), String::new())
        );
        assert_eq!(
            format_frame_metric(FrameMetric::LatencyPc, None, &it),
            (DASH.into(), String::new())
        );
    }
}
