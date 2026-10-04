//! The display key of a sensor value (R7): values with the same key format to
//! the same text in the UI, so the health report changes only when a
//! formatted value would.

use crate::model::Unit;

/// Rounds half away from zero like the UI's `Intl.NumberFormat`; the cast
/// saturates.
fn round(value: f64) -> i64 {
    value.round() as i64
}

/// Step and rounded digits of `value` shown in steps of `base` up to
/// `last_step`, with `digits(step, scaled)` decimals, as
/// `[step * 4 + digits, rounded]`: the text depends on all three.
fn stepped(mut value: f64, base: f64, last_step: usize, digits: fn(usize, f64) -> i32) -> [i64; 2] {
    let mut step = 0;
    while value.abs() >= base && step < last_step {
        value /= base;
        step += 1;
    }
    let digits = digits(step, value);
    [
        step as i64 * 4 + i64::from(digits),
        round(value * 10f64.powi(digits)),
    ]
}

/// `formatBytes`: binary steps, B to TB, one decimal below 100 past bytes.
fn bytes_key(value: f64) -> [i64; 2] {
    stepped(value, 1024.0, 4, |step, v| {
        i32::from(step != 0 && v < 100.0)
    })
}

/// `formatRate` in bits: decimal steps, bit/s to Tbit/s, one decimal below 10.
fn bits_key(bytes_per_second: f64) -> [i64; 2] {
    stepped(bytes_per_second * 8.0, 1000.0, 4, |_, v| {
        i32::from(v < 10.0)
    })
}

/// Display key of `value` in `unit` (R7): values with the same key format to
/// the same text in every display unit (°C and °F, bytes and bits), so the
/// report changes only when a formatted value would. Mirrors `formatValue`
/// in `app/src/lib/format.ts`, the most precise formatter; the tray
/// (`app/src-tauri/src/tray_icon.rs`) shows whole numbers only. Floating
/// point ties may round differently from `Intl.NumberFormat`.
pub(crate) fn display_key(value: f64, unit: Unit) -> [i64; 4] {
    let pair = |[a, b]: [i64; 2], [c, d]: [i64; 2]| [a, b, c, d];
    match unit {
        Unit::Celsius => [round(value), round(value * 9.0 / 5.0 + 32.0), 0, 0],
        Unit::Percent | Unit::Watt | Unit::Rpm | Unit::Hours | Unit::Count => {
            [round(value), 0, 0, 0]
        }
        Unit::Megahertz if value >= 1000.0 => [1, round(value / 1000.0 * 100.0), 0, 0],
        Unit::Megahertz => [0, round(value), 0, 0],
        Unit::Volt => [round(value * 1000.0), 0, 0, 0],
        Unit::Ampere => [round(value * 10.0), 0, 0, 0],
        Unit::Bytes => pair(bytes_key(value), [0, 0]),
        // Throughput is shown in bytes or in bits, as the settings say.
        Unit::BytesPerSecond => pair(bytes_key(value), bits_key(value)),
        Unit::BitsPerSecond => pair(bytes_key(value / 8.0), bits_key(value / 8.0)),
        // `formatEnergy`: decimal steps, J to GJ, decimals like bytes.
        Unit::Joule => pair(
            stepped(value, 1000.0, 3, |step, v| {
                i32::from(step != 0 && v < 100.0)
            }),
            [0, 0],
        ),
        Unit::Boolean => [i64::from(value >= 0.5), 0, 0, 0],
        // `Math.round`: half up.
        Unit::PcieGeneration | Unit::Lanes => [(value + 0.5).floor() as i64, 0, 0, 0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_key_matches_formatter_precision() {
        let same = |a: f64, b: f64, unit: Unit| display_key(a, unit) == display_key(b, unit);
        // Whole percent, watt, rpm.
        assert!(same(50.4, 49.6, Unit::Percent));
        assert!(!same(50.4, 50.6, Unit::Percent));
        assert!(same(120.2, 119.8, Unit::Watt));
        // Temperatures in both °C and °F.
        assert!(same(91.6, 91.8, Unit::Celsius));
        assert!(!same(91.9, 91.95, Unit::Celsius));
        // Volts with three decimals, amperes with one.
        assert!(!same(1.2004, 1.2006, Unit::Volt));
        assert!(same(1.2004, 1.2001, Unit::Volt));
        assert!(!same(1.04, 1.06, Unit::Ampere));
        // Throughput in bytes (binary steps) and bits (decimal steps):
        // 1234 B/s is 1.2 KB/s and 9.9 kbit/s; 1240 B/s is 1.2 KB/s and
        // 9.9 kbit/s; 1300 B/s is 1.3 KB/s and 10 kbit/s.
        assert!(same(1234.0, 1240.0, Unit::BytesPerSecond));
        assert!(!same(1240.0, 1300.0, Unit::BytesPerSecond));
        // Same text in bits, not in bytes: 96 kbit/s, 11.7 vs 11.8 KB/s.
        assert!(!same(12_000.0, 12_050.0, Unit::BytesPerSecond));
        // Same text in bytes, not in bits: 11.7 KB/s, 95 vs 96 kbit/s.
        assert!(!same(11_930.0, 11_950.0, Unit::BytesPerSecond));
        // 15.0 KB and 150 KB share digits but not text.
        assert!(!same(15.0 * 1024.0, 150.0 * 1024.0, Unit::Bytes));
        assert!(!same(15.0 * 1024.0, 150.0 * 1024.0, Unit::BytesPerSecond));
        assert!(same(12_000.0, 12_400.0, Unit::BitsPerSecond));
        // Clocks: MHz below 1000, GHz with two decimals above.
        assert!(same(3601.0, 3604.0, Unit::Megahertz));
        assert!(!same(3601.0, 3606.0, Unit::Megahertz));
        // Flags: on or off.
        assert!(same(1.0, 0.7, Unit::Boolean));
        assert!(!same(1.0, 0.0, Unit::Boolean));
    }
}
