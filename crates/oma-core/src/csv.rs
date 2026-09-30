//! Pure CSV formatter for the sensor log: column layout, header, rows,
//! timestamps with a UTC offset, numbers, file names and the new-part
//! decision. No I/O and no Windows code.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::mem::size_of;

use crate::model::{DeviceKind, Schema, Sensor, Unit};
use crate::settings::{TemperatureUnit, ThroughputUnit};

/// UTF-8 byte order mark written at the start of every file part.
pub const BOM: &[u8] = b"\xEF\xBB\xBF";
/// Title of the first column.
pub const TIMESTAMP_TITLE: &str = "Timestamp";

/// Display units the log follows (same settings as the UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayUnits {
    pub temperature: TemperatureUnit,
    pub throughput: ThroughputUnit,
}

/// How a raw sensor value becomes the written number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conversion {
    None,
    CelsiusToFahrenheit,
    BytesToBits,
    /// Any non-zero value is written as `1`.
    Flag,
}

/// One value column.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub sensor_id: String,
    /// Position of the sensor in the schema (and in snapshot values).
    pub index: usize,
    /// Display name of the sensor's device, as the schema carries it.
    pub device: String,
    /// Translated sensor label.
    pub label: String,
    /// Unit symbol without brackets; empty when the sensor has none.
    pub unit: &'static str,
    pub conversion: Conversion,
}

/// The columns of a log part.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub columns: Vec<Column>,
}

impl Layout {
    /// Columns in schema order; `selection` `None` = all sensors; ids not in
    /// the schema have no column.
    ///
    /// The returned `bool` is `true` on overflow: more selected columns than
    /// `max_columns`. The layout then holds only the first `max_columns`
    /// columns and the caller must NOT write it; it has to refuse the session
    /// with an error instead of silently truncating the requested sensors.
    pub fn build(
        schema: &Schema,
        selection: Option<&[String]>,
        units: DisplayUnits,
        label: &dyn Fn(&Sensor) -> String,
        max_columns: usize,
    ) -> (Layout, bool) {
        let selected: Option<HashSet<&str>> =
            selection.map(|ids| ids.iter().map(String::as_str).collect());
        let devices: HashMap<&str, _> = schema
            .devices
            .iter()
            .map(|device| (device.id.as_str(), device))
            .collect();
        let mut columns = Vec::new();
        let mut overflow = false;
        for (index, sensor) in schema.sensors.iter().enumerate() {
            if let Some(selected) = &selected {
                if !selected.contains(sensor.id.as_str()) {
                    continue;
                }
            }
            if columns.len() >= max_columns {
                overflow = true;
                break;
            }
            let device = devices.get(sensor.device_id.as_str());
            let network = device.is_some_and(|d| d.kind == DeviceKind::Network);
            let (unit, conversion) = unit_symbol(sensor.unit, units, network);
            columns.push(Column {
                sensor_id: sensor.id.clone(),
                index,
                device: device.map_or_else(|| sensor.device_id.clone(), |d| d.name.clone()),
                label: label(sensor),
                unit,
                conversion,
            });
        }
        (Layout { columns }, overflow)
    }

    /// Two layouts produce the same output when ids, devices, labels and unit
    /// symbols match column by column; the schema index is ignored (L4).
    pub fn same_output(&self, other: &Layout) -> bool {
        self.columns.len() == other.columns.len()
            && self.columns.iter().zip(&other.columns).all(|(a, b)| {
                a.sensor_id == b.sensor_id
                    && a.device == b.device
                    && a.label == b.label
                    && a.unit == b.unit
            })
    }

    /// Raw values in column order; a value missing from `values` is `None`.
    pub fn extract(&self, values: &[Option<f64>]) -> Box<[Option<f64>]> {
        self.columns
            .iter()
            .map(|column| values.get(column.index).copied().flatten())
            .collect()
    }

    /// Conservative estimate of the memory this layout retains when held in
    /// an `Arc`: the `Arc` counters, the struct, the column vector capacity
    /// and the string capacities (L2).
    pub fn retained_bytes(&self) -> usize {
        let strings: usize = self
            .columns
            .iter()
            .map(|c| c.sensor_id.capacity() + c.device.capacity() + c.label.capacity())
            .sum();
        2 * size_of::<usize>()
            + size_of::<Layout>()
            + self.columns.capacity() * size_of::<Column>()
            + strings
    }
}

/// Unit symbol (empty for none) and value conversion of a sensor unit in the
/// log (L5). No scaling; `network` is whether the sensor's device is a
/// network adapter, the only place throughput becomes `bit/s`.
pub fn unit_symbol(unit: Unit, units: DisplayUnits, network: bool) -> (&'static str, Conversion) {
    match unit {
        Unit::Celsius => match units.temperature {
            TemperatureUnit::C => ("°C", Conversion::None),
            TemperatureUnit::F => ("°F", Conversion::CelsiusToFahrenheit),
        },
        Unit::Percent => ("%", Conversion::None),
        Unit::Megahertz => ("MHz", Conversion::None),
        Unit::Watt => ("W", Conversion::None),
        Unit::Volt => ("V", Conversion::None),
        Unit::Ampere => ("A", Conversion::None),
        Unit::Rpm => ("RPM", Conversion::None),
        Unit::Bytes => ("B", Conversion::None),
        Unit::BytesPerSecond => {
            if network && units.throughput == ThroughputUnit::Bits {
                ("bit/s", Conversion::BytesToBits)
            } else {
                ("B/s", Conversion::None)
            }
        }
        // Already in bits per second: nothing to convert.
        Unit::BitsPerSecond => ("bit/s", Conversion::None),
        Unit::Joule => ("J", Conversion::None),
        Unit::Hours => ("h", Conversion::None),
        Unit::Boolean => ("", Conversion::Flag),
        Unit::PcieGeneration | Unit::Lanes | Unit::Count => ("", Conversion::None),
    }
}

/// Writes the CRLF-terminated header line (no BOM).
pub fn header_line(layout: &Layout, out: &mut String) {
    escape_field(TIMESTAMP_TITLE, out);
    let mut title = String::new();
    for column in &layout.columns {
        title.clear();
        let _ = write!(title, "{} / {}", column.device, column.label);
        if !column.unit.is_empty() {
            let _ = write!(title, " [{}]", column.unit);
        }
        let _ = write!(title, " {{{}}}", column.sensor_id);
        out.push(',');
        escape_field(&title, out);
    }
    out.push_str("\r\n");
}

/// Writes one CRLF-terminated row. `values` are raw values in column order
/// (see [`Layout::extract`]); conversions are applied here. Missing and
/// non-finite values are empty cells.
pub fn row_line(
    layout: &Layout,
    timestamp_ms: u64,
    offset_minutes: i32,
    values: &[Option<f64>],
    out: &mut String,
) {
    format_timestamp(timestamp_ms, offset_minutes, out);
    for (i, column) in layout.columns.iter().enumerate() {
        out.push(',');
        let Some(value) = values.get(i).copied().flatten().filter(|v| v.is_finite()) else {
            continue;
        };
        let converted = match column.conversion {
            Conversion::None => value,
            Conversion::CelsiusToFahrenheit => value * 9.0 / 5.0 + 32.0,
            Conversion::BytesToBits => value * 8.0,
            Conversion::Flag => f64::from(u8::from(value != 0.0)),
        };
        format_number(converted, out);
    }
    out.push_str("\r\n");
}

/// `2026-09-29T14:03:12.000+02:00`.
pub fn format_timestamp(unix_ms: u64, offset_minutes: i32, out: &mut String) {
    let t = local_time(unix_ms, offset_minutes);
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let abs = offset_minutes.unsigned_abs();
    let _ = write!(
        out,
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}{sign}{:02}:{:02}",
        t.year,
        t.month,
        t.day,
        t.hour,
        t.minute,
        t.second,
        t.millis,
        abs / 60,
        abs % 60
    );
}

/// At most 3 decimals, no trailing zeros, no `-0`. Non-finite values write
/// nothing (callers write an empty cell instead).
pub fn format_number(value: f64, out: &mut String) {
    if !value.is_finite() {
        return;
    }
    // Written straight into the line and trimmed there: no allocation.
    let start = out.len();
    let _ = write!(out, "{value:.3}");
    // `{:.3}` always writes a `.` and three decimals for a finite value.
    let trimmed = out.trim_end_matches('0').trim_end_matches('.').len();
    out.truncate(trimmed);
    if &out[start..] == "-0" {
        out.truncate(start);
        out.push('0');
    }
}

/// Text field: a leading `=`, `+`, `-` or `@` gets an apostrophe in front
/// (formula guard), then the field is quoted if it holds `,`, `"`, CR or LF
/// (inner quotes doubled). Only for text, never for numbers.
pub fn escape_field(text: &str, out: &mut String) {
    let guarded = text.starts_with(['=', '+', '-', '@']);
    let quoted = text.contains([',', '"', '\r', '\n']);
    if quoted {
        out.push('"');
    }
    if guarded {
        out.push('\'');
    }
    for ch in text.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    if quoted {
        out.push('"');
    }
}

/// A civil date and time, already shifted by the UTC offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millis: u16,
}

/// Civil date and time of a Unix time shifted by `offset_minutes`; no
/// time-zone database.
pub fn local_time(unix_ms: u64, offset_minutes: i32) -> LocalTime {
    let shifted = i64::try_from(unix_ms)
        .unwrap_or(i64::MAX)
        .saturating_add(i64::from(offset_minutes) * 60_000);
    let days = shifted.div_euclid(86_400_000);
    let ms_of_day = shifted.rem_euclid(86_400_000);
    // Civil date from days since 1970-01-01 (proleptic Gregorian calendar).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    LocalTime {
        year: year as i32,
        month: month as u8,
        day: day as u8,
        hour: (ms_of_day / 3_600_000) as u8,
        minute: (ms_of_day % 3_600_000 / 60_000) as u8,
        second: (ms_of_day % 60_000 / 1_000) as u8,
        millis: (ms_of_day % 1_000) as u16,
    }
}

/// `oma-2026-09-29_14-03-12[-2][-partN].csv`; part 1 has no part suffix.
pub fn file_name(start: LocalTime, suffix: Option<u32>, part: u32) -> String {
    let mut name = format!(
        "oma-{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        start.year, start.month, start.day, start.hour, start.minute, start.second
    );
    if let Some(suffix) = suffix {
        let _ = write!(name, "-{suffix}");
    }
    if part > 1 {
        let _ = write!(name, "-part{part}");
    }
    name.push_str(".csv");
    name
}

/// What to do with the next row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartDecision {
    Append,
    NewPart,
    /// The row cannot fit even in an empty part.
    TooLarge,
}

/// `Append` if the row fits; otherwise `NewPart` if `BOM + header + row`
/// fits in a fresh part, else `TooLarge`. A part that holds only `BOM +
/// header` never rotates, so a row too large for it gives `TooLarge`.
pub fn part_decision(
    part_bytes: u64,
    row_bytes: u64,
    header_bytes: u64,
    limit_bytes: u64,
) -> PartDecision {
    if part_bytes.saturating_add(row_bytes) <= limit_bytes {
        return PartDecision::Append;
    }
    let empty = (BOM.len() as u64).saturating_add(header_bytes);
    if part_bytes > empty && empty.saturating_add(row_bytes) <= limit_bytes {
        PartDecision::NewPart
    } else {
        PartDecision::TooLarge
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Device, Label, SensorKind, Source};

    const T0: u64 = 1_790_683_392_000; // 2026-09-29T12:03:12Z

    fn ts(ms: u64, off: i32) -> String {
        let mut s = String::new();
        format_timestamp(ms, off, &mut s);
        s
    }

    fn num(v: f64) -> String {
        let mut s = String::new();
        format_number(v, &mut s);
        s
    }

    fn esc(t: &str) -> String {
        let mut s = String::new();
        escape_field(t, &mut s);
        s
    }

    fn units(t: TemperatureUnit, th: ThroughputUnit) -> DisplayUnits {
        DisplayUnits {
            temperature: t,
            throughput: th,
        }
    }

    fn cb() -> DisplayUnits {
        units(TemperatureUnit::C, ThroughputUnit::Bytes)
    }

    fn device(id: &str, kind: DeviceKind, name: &str) -> Device {
        Device {
            id: id.into(),
            kind,
            name: name.into(),
            vendor: None,
            properties: Default::default(),
        }
    }

    fn sensor(dev: &str, name: &str, unit: Unit) -> Sensor {
        Sensor {
            id: format!("{dev}/x/{name}"),
            device_id: dev.into(),
            kind: SensorKind::Load,
            unit,
            label: Label::new(name),
            source: Source::Mock,
            category: String::new(),
            experimental: false,
        }
    }

    fn schema(devices: Vec<Device>, sensors: Vec<Sensor>) -> Schema {
        Schema {
            revision: 1,
            devices,
            sensors,
        }
    }

    fn lab(s: &Sensor) -> String {
        s.id.rsplit('/').next().unwrap().to_string()
    }

    fn single(dev: Device, unit: Unit, u: DisplayUnits) -> Layout {
        let s = sensor(&dev.id, "v", unit);
        Layout::build(&schema(vec![dev], vec![s]), None, u, &lab, 10).0
    }

    #[test]
    fn timestamp_with_positive_and_negative_offsets() {
        assert_eq!(ts(T0, 120), "2026-09-29T14:03:12.000+02:00");
        assert_eq!(ts(T0 + 45, -330), "2026-09-29T06:33:12.045-05:30");
        assert_eq!(ts(T0, 0), "2026-09-29T12:03:12.000+00:00");
    }

    #[test]
    fn timestamp_across_midnight_and_leap_day() {
        // 2024-02-28T23:30:00Z +60 min = 2024-02-29T00:30 (leap day).
        assert_eq!(ts(1_709_163_000_000, 60), "2024-02-29T00:30:00.000+01:00");
        // 2024-03-01T00:10:00Z -30 min = 2024-02-29T23:40.
        assert_eq!(ts(1_709_251_800_000, -30), "2024-02-29T23:40:00.000-00:30");
        // 2023-12-31T23:59:59.999Z +1 min rolls into the new year.
        assert_eq!(ts(1_704_067_199_999, 1), "2024-01-01T00:00:59.999+00:01");
        // Before the epoch with a negative offset.
        assert_eq!(ts(0, -60), "1969-12-31T23:00:00.000-01:00");
    }

    #[test]
    fn offset_change_between_rows_keeps_the_part() {
        let s = schema(
            vec![device("d", DeviceKind::Cpu, "CPU")],
            vec![sensor("d", "v", Unit::Percent)],
        );
        let (a, _) = Layout::build(&s, None, cb(), &lab, 10);
        let (b, _) = Layout::build(&s, None, cb(), &lab, 10);
        assert!(a.same_output(&b));
        let (mut r1, mut r2) = (String::new(), String::new());
        row_line(&a, T0, 120, &[Some(1.0)], &mut r1);
        row_line(&a, T0, 60, &[Some(1.0)], &mut r2);
        assert_eq!(r1, "2026-09-29T14:03:12.000+02:00,1\r\n");
        assert_eq!(r2, "2026-09-29T13:03:12.000+01:00,1\r\n");
    }

    #[test]
    fn numbers_have_at_most_three_decimals() {
        assert_eq!(num(1.23456), "1.235");
        assert_eq!(num(2.5), "2.5");
        assert_eq!(num(3.0), "3");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(1e-4), "0");
        assert_eq!(num(-1e-4), "0");
        assert_eq!(num(-5.0), "-5");
        assert_eq!(num(100.0), "100");
        assert_eq!(num(12_345_678_901.0), "12345678901");
        // Rounding boundaries follow `{:.3}` on the binary value: 0.0005 is
        // just above the half, 1.0005 just below it.
        assert_eq!(format!("{:.3}", 0.0005), "0.001");
        assert_eq!(num(0.0005), "0.001");
        assert_eq!(format!("{:.3}", 1.0005), "1.000");
        assert_eq!(num(1.0005), "1");
        assert_eq!(num(-0.0005), "-0.001");
        assert_eq!(num(0.9995), "1");
    }

    #[test]
    fn numbers_append_without_touching_the_line() {
        for (prefix, value, expected) in [
            ("0.0,", 100.0, "0.0,100"),
            ("1.,", 2.5, "1.,2.5"),
            ("x,", -0.0004, "x,0"),
            ("-", -0.0, "-0"),
            ("7.10", 3.0, "7.103"),
        ] {
            let mut s = String::from(prefix);
            format_number(value, &mut s);
            assert_eq!(s, expected, "{prefix} {value}");
        }
    }

    #[test]
    fn missing_and_non_finite_are_empty() {
        let l = single(device("d", DeviceKind::Cpu, "CPU"), Unit::Percent, cb());
        for v in [
            None,
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(f64::NEG_INFINITY),
        ] {
            let mut s = String::new();
            row_line(&l, T0, 0, &[v], &mut s);
            assert_eq!(s, "2026-09-29T12:03:12.000+00:00,\r\n");
        }
    }

    #[test]
    fn fahrenheit_and_bits_conversions() {
        let f = units(TemperatureUnit::F, ThroughputUnit::Bits);
        let l = single(device("d", DeviceKind::Cpu, "CPU"), Unit::Celsius, f);
        assert_eq!(l.columns[0].unit, "°F");
        let mut s = String::new();
        row_line(&l, T0, 0, &[Some(60.0)], &mut s);
        assert!(s.ends_with(",140\r\n"), "{s}");

        let net = single(
            device("n", DeviceKind::Network, "Wi-Fi"),
            Unit::BytesPerSecond,
            f,
        );
        assert_eq!(net.columns[0].unit, "bit/s");
        let mut s = String::new();
        row_line(&net, T0, 0, &[Some(1000.0)], &mut s);
        assert!(s.ends_with(",8000\r\n"), "{s}");

        let disk = single(
            device("s", DeviceKind::Storage, "SSD"),
            Unit::BytesPerSecond,
            f,
        );
        assert_eq!(disk.columns[0].unit, "B/s");
        let mut s = String::new();
        row_line(&disk, T0, 0, &[Some(1000.0)], &mut s);
        assert!(s.ends_with(",1000\r\n"), "{s}");

        // Network with bytes selected stays in B/s.
        let nb = single(
            device("n", DeviceKind::Network, "Wi-Fi"),
            Unit::BytesPerSecond,
            cb(),
        );
        assert_eq!(nb.columns[0].unit, "B/s");
    }

    #[test]
    fn unit_symbols_follow_the_spec() {
        let u = cb();
        for (unit, sym) in [
            (Unit::Percent, "%"),
            (Unit::Megahertz, "MHz"),
            (Unit::Watt, "W"),
            (Unit::Volt, "V"),
            (Unit::Ampere, "A"),
            (Unit::Rpm, "RPM"),
            (Unit::Bytes, "B"),
            (Unit::Joule, "J"),
            (Unit::Hours, "h"),
            (Unit::Boolean, ""),
            (Unit::PcieGeneration, ""),
            (Unit::Lanes, ""),
            (Unit::Count, ""),
            (Unit::Celsius, "°C"),
        ] {
            assert_eq!(unit_symbol(unit, u, false).0, sym, "{unit:?}");
        }
    }

    #[test]
    fn flags_are_zero_or_one() {
        let l = single(device("d", DeviceKind::Cpu, "CPU"), Unit::Boolean, cb());
        assert_eq!(l.columns[0].unit, "");
        for (v, want) in [(0.0, ",0\r\n"), (2.0, ",1\r\n"), (1.0, ",1\r\n")] {
            let mut s = String::new();
            row_line(&l, T0, 0, &[Some(v)], &mut s);
            assert!(s.ends_with(want), "{s}");
        }
    }

    #[test]
    fn header_escapes_and_guards_formulas() {
        let s = schema(
            vec![
                device("a", DeviceKind::Storage, "Disk \"A\", 1"),
                device("b", DeviceKind::Cpu, "=cmd"),
                device("c", DeviceKind::Cpu, "-x"),
                device("d", DeviceKind::Cpu, "Scheda è"),
            ],
            vec![
                sensor("a", "v", Unit::Percent),
                sensor("b", "v", Unit::Boolean),
                sensor("c", "v", Unit::Percent),
                sensor("d", "v", Unit::Percent),
            ],
        );
        let (l, _) = Layout::build(&s, None, cb(), &lab, 10);
        let mut h = String::new();
        header_line(&l, &mut h);
        assert_eq!(
            h,
            "Timestamp,\"Disk \"\"A\"\", 1 / v [%] {a/x/v}\",'=cmd / v {b/x/v},'-x / v [%] {c/x/v},Scheda è / v [%] {d/x/v}\r\n"
        );
        assert_eq!(esc("+1"), "'+1");
        assert_eq!(esc("@a"), "'@a");
        assert_eq!(esc("-a,b"), "\"'-a,b\"");
        assert_eq!(esc("a\nb"), "\"a\nb\"");
    }

    #[test]
    fn negative_numbers_are_not_guarded() {
        let l = single(device("d", DeviceKind::Cpu, "CPU"), Unit::Percent, cb());
        let mut s = String::new();
        row_line(&l, T0, 0, &[Some(-5.0)], &mut s);
        assert!(s.ends_with(",-5\r\n"), "{s}");
    }

    #[test]
    fn lines_end_with_crlf() {
        let l = single(device("d", DeviceKind::Cpu, "CPU"), Unit::Percent, cb());
        let (mut h, mut r) = (String::new(), String::new());
        header_line(&l, &mut h);
        row_line(&l, T0, 0, &[Some(1.0)], &mut r);
        assert!(h.ends_with("\r\n") && !h[..h.len() - 2].contains('\n'));
        assert!(r.ends_with("\r\n") && !r[..r.len() - 2].contains('\n'));
        assert!(!h.starts_with('\u{feff}'));
    }

    #[test]
    fn layout_follows_schema_order_and_selection() {
        let s = schema(
            vec![device("d", DeviceKind::Cpu, "CPU")],
            vec![
                sensor("d", "a", Unit::Percent),
                sensor("d", "b", Unit::Percent),
                sensor("d", "c", Unit::Percent),
            ],
        );
        let sel = vec!["d/x/c".to_string(), "missing".into(), "d/x/a".into()];
        let (l, over) = Layout::build(&s, Some(&sel), cb(), &lab, 10);
        assert!(!over);
        let ids: Vec<_> = l.columns.iter().map(|c| c.sensor_id.as_str()).collect();
        assert_eq!(ids, ["d/x/a", "d/x/c"]);
        assert_eq!(l.columns[1].index, 2);
        let vals = l.extract(&[Some(1.0), Some(2.0), Some(3.0)]);
        assert_eq!(&*vals, &[Some(1.0), Some(3.0)]);
        assert_eq!(&*l.extract(&[Some(1.0)]), &[Some(1.0), None]);
        let (all, _) = Layout::build(&s, None, cb(), &lab, 10);
        assert_eq!(all.columns.len(), 3);
    }

    #[test]
    fn layout_reports_overflow_at_the_limit() {
        let s = schema(
            vec![device("d", DeviceKind::Cpu, "CPU")],
            vec![
                sensor("d", "a", Unit::Percent),
                sensor("d", "b", Unit::Percent),
            ],
        );
        let (l, over) = Layout::build(&s, None, cb(), &lab, 2);
        assert!(!over && l.columns.len() == 2);
        let (l, over) = Layout::build(&s, None, cb(), &lab, 1);
        assert!(over && l.columns.len() == 1);
        // A selection that fits is not an overflow even if the schema is larger.
        let sel = vec!["d/x/b".to_string()];
        let (_, over) = Layout::build(&s, Some(&sel), cb(), &lab, 1);
        assert!(!over);
    }

    #[test]
    fn same_output_ignores_indices_but_not_labels_or_units() {
        let base = single(device("d", DeviceKind::Cpu, "CPU"), Unit::Percent, cb());
        let mut moved = base.clone();
        moved.columns[0].index = 7;
        assert!(base.same_output(&moved));
        let mut label = base.clone();
        label.columns[0].label = "other".into();
        assert!(!base.same_output(&label));
        let mut unit = base.clone();
        unit.columns[0].unit = "W";
        assert!(!base.same_output(&unit));
        let mut device = base.clone();
        device.columns[0].device = "GPU".into();
        assert!(!base.same_output(&device));
        let mut longer = base.clone();
        longer.columns.push(base.columns[0].clone());
        assert!(!base.same_output(&longer));
    }

    #[test]
    fn retained_bytes_grows_with_labels() {
        let mut l = single(device("d", DeviceKind::Cpu, "CPU"), Unit::Percent, cb());
        let small = l.retained_bytes();
        l.columns[0].label = "x".repeat(1000);
        assert!(l.retained_bytes() >= small + 900);
    }

    #[test]
    fn file_names() {
        let start = local_time(T0, 120);
        assert_eq!(file_name(start, None, 1), "oma-2026-09-29_14-03-12.csv");
        assert_eq!(
            file_name(start, Some(2), 1),
            "oma-2026-09-29_14-03-12-2.csv"
        );
        assert_eq!(
            file_name(start, None, 2),
            "oma-2026-09-29_14-03-12-part2.csv"
        );
        assert_eq!(
            file_name(start, Some(2), 3),
            "oma-2026-09-29_14-03-12-2-part3.csv"
        );
    }

    #[test]
    fn local_time_matches_the_epoch_and_leap_years() {
        let t = local_time(0, 0);
        assert_eq!((t.year, t.month, t.day, t.hour), (1970, 1, 1, 0));
        let t = local_time(951_782_400_000, 0);
        assert_eq!((t.year, t.month, t.day), (2000, 2, 29));
    }

    #[test]
    fn part_decision_cases() {
        // limit 1000, header 100: an empty part is 103 bytes.
        assert_eq!(part_decision(500, 100, 100, 1000), PartDecision::Append);
        assert_eq!(part_decision(900, 100, 100, 1000), PartDecision::Append);
        assert_eq!(part_decision(901, 100, 100, 1000), PartDecision::NewPart);
        // Too large even for an empty part (103 + 898 > 1000).
        assert_eq!(part_decision(500, 898, 100, 1000), PartDecision::TooLarge);
        // Fresh part: never rotates again.
        assert_eq!(part_decision(103, 898, 100, 1000), PartDecision::TooLarge);
        assert_eq!(part_decision(103, 897, 100, 1000), PartDecision::Append);
        assert_eq!(
            part_decision(u64::MAX, u64::MAX, 100, 1000),
            PartDecision::TooLarge
        );
    }
}
