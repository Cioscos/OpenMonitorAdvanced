//! Pure geometry and text of the blocks: where a block goes in pixels, what
//! its three text parts read, and how a chart maps its samples into its
//! rectangle. No Windows code, so all of it is tested without a GPU.

use oma_core::format::{format_frame_metric, format_value, FormatOptions, DASH};
use oma_core::model::Unit;
use oma_core::overlay::{
    cell_px, footprint, Align, AxisMode, Block, FrameMetric, GraphMode, Kind, LowDefinitionKey,
    Orientation, Profile, RangeBound, Source, Stat, StatOp, YAxis, CELL_PX,
};
use oma_core::settings::{TemperatureUnit, ThroughputUnit};

use oma_ipc::overlay::{WireBenchmarkSummary, WireFrameTime};

use crate::state::{metric_value, range_key, OverlayState, SourceKey, AUTO_RANGE_S};

/// A rectangle in physical pixels, as Direct2D takes it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// The three texts of a block, each drawn with its own style.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextParts {
    pub label: String,
    pub value: String,
    pub unit: String,
}

/// Where the profile is drawn inside its window: the cell size, the pixel
/// position of cell (0, 0) and the panel's size (as `place` computes it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub cell: f64,
    pub origin: (f64, f64),
    pub size: (f32, f32),
}

/// The frame of `profile` at `dpi`; `None` without blocks.
pub fn profile_frame(profile: &Profile, dpi: u32) -> Option<Frame> {
    let f = footprint(&profile.blocks)?;
    let cell = cell_px(profile.scale, dpi);
    let pad = f64::from(profile.panel.padding);
    let w = ((f64::from(f.w) + 2.0 * pad) * cell).round() as f32;
    let h = ((f64::from(f.h) + 2.0 * pad) * cell).round() as f32;
    Some(Frame {
        cell,
        origin: ((pad - f64::from(f.x)) * cell, (pad - f64::from(f.y)) * cell),
        size: (w, h),
    })
}

/// The block's rectangle in pixels; `origin` is where cell (0, 0) lies.
pub fn block_px(block: &Block, origin: (f64, f64), cell: f64) -> RectF {
    let r = block.rect;
    RectF {
        x: (origin.0 + f64::from(r.x) * cell) as f32,
        y: (origin.1 + f64::from(r.y) * cell) as f32,
        w: (f64::from(r.w) * cell) as f32,
        h: (f64::from(r.h) * cell) as f32,
    }
}

/// A length of the profile in 96-DPI pixels at scale 1 (font sizes, outline
/// widths, shadow offsets, radii) in physical pixels, for a cell of `cell`.
pub fn style_px(v: f64, cell: f64) -> f32 {
    (v * cell / CELL_PX) as f32
}

/// A font size in points (§6.3) in physical pixels, for a cell of `cell`:
/// `pt × 96/72 × scale × dpi/96`, the cell carrying scale and DPI.
pub fn font_px(size_pt: f32, cell: f64) -> f32 {
    (f64::from(size_pt) * 4.0 / 3.0 * cell / CELL_PX) as f32
}

/// A corner radius in pixels: `radius` is in 96-DPI pixels at scale 1, and
/// never more than half the shorter side of a `w × h` rectangle.
pub fn panel_radius_px(radius: f64, cell: f64, w: f32, h: f32) -> f32 {
    let r = style_px(radius.max(0.0), cell);
    r.min(w.min(h) / 2.0).max(0.0)
}

/// The profile's spelling of a frame metric, as in `metric.<name>`.
pub fn metric_name(m: FrameMetric) -> &'static str {
    match m {
        FrameMetric::FpsDisplayed => "fps-displayed",
        FrameMetric::FpsRendered => "fps-rendered",
        FrameMetric::FpsPresented => "fps-presented",
        FrameMetric::FrametimeDisplayed => "frametime-displayed",
        FrameMetric::FrametimeApp => "frametime-app",
        FrameMetric::Low1 => "low-1",
        FrameMetric::Low01 => "low-01",
        FrameMetric::FgMultiplier => "fg-multiplier",
        FrameMetric::Stutter => "stutter",
        FrameMetric::LatencyPc => "latency-pc",
        FrameMetric::LatencyDisplay => "latency-display",
        FrameMetric::Bound => "bound",
    }
}

/// The core `Unit` from its snake_case wire spelling (`SensorInfo.unit`).
pub fn parse_unit(s: &str) -> Option<Unit> {
    Some(match s {
        "celsius" => Unit::Celsius,
        "percent" => Unit::Percent,
        "megahertz" => Unit::Megahertz,
        "watt" => Unit::Watt,
        "volt" => Unit::Volt,
        "ampere" => Unit::Ampere,
        "rpm" => Unit::Rpm,
        "bytes" => Unit::Bytes,
        "bytes_per_second" => Unit::BytesPerSecond,
        "bits_per_second" => Unit::BitsPerSecond,
        "joule" => Unit::Joule,
        "boolean" => Unit::Boolean,
        "pcie_generation" => Unit::PcieGeneration,
        "lanes" => Unit::Lanes,
        "hours" => Unit::Hours,
        "count" => Unit::Count,
        _ => return None,
    })
}

fn definition_name(d: LowDefinitionKey) -> &'static str {
    match d {
        LowDefinitionKey::Integral => "integral",
        LowDefinitionKey::Percentile => "percentile",
    }
}

/// Whether the frame engine runs; otherwise every frame value is absent.
fn frames_running(state: &OverlayState) -> bool {
    state.metrics.as_ref().is_some_and(|m| m.state == "running")
}

/// The value of `source` after `stat`: a ring for `min`, `avg` and `max`,
/// the last datum for `current`, the app's lows for `low-*`. `None` when
/// absent, for frames while the engine is not running, for texts and `bound`.
pub fn source_value(source: &Source, stat: &Stat, state: &OverlayState) -> Option<f64> {
    let from_ring = || {
        let key = SourceKey::of(source)?;
        state.rings.get(&key)?.value(stat)
    };
    let value = match source {
        Source::Text(_) => None,
        Source::Sensor(id) if stat.op == StatOp::Current => {
            state.values.get(id).and_then(|(v, _)| *v)
        }
        Source::Sensor(_) => from_ring(),
        Source::Frames(m) => {
            let metrics = state.metrics.as_ref().filter(|_| frames_running(state))?;
            match m {
                FrameMetric::Bound => None,
                FrameMetric::Low1 | FrameMetric::Low01 => {
                    let def = definition_name(stat.definition);
                    let low = metrics
                        .lows
                        .iter()
                        .find(|l| l.window_s == stat.window && l.definition == def)?;
                    if *m == FrameMetric::Low1 {
                        low.one_percent
                    } else {
                        low.point_one_percent
                    }
                }
                _ if stat.op == StatOp::Current => metric_value(metrics, *m),
                _ => from_ring(),
            }
        }
    };
    value.filter(|v| v.is_finite())
}

/// The value a block shows (and its thresholds compare), before rounding.
pub fn block_value(block: &Block, state: &OverlayState) -> Option<f64> {
    source_value(&block.source, &block.stat, state)
}

fn string(state: &OverlayState, key: &str) -> Option<String> {
    state.strings.get(key).cloned()
}

/// The formatting options of `block` under the drawing settings of `state`.
pub fn format_options(block: &Block, state: &OverlayState) -> FormatOptions {
    let d = &state.draw;
    // The settings spelling (`c`, `f`) and the long one are both accepted.
    let temperature = match d.temperature_unit.as_str() {
        "f" | "fahrenheit" => TemperatureUnit::F,
        _ => TemperatureUnit::C,
    };
    let rate = ThroughputUnit::parse(&d.throughput_unit).unwrap_or_default();
    let defaults = FormatOptions::default();
    FormatOptions {
        decimal_comma: d.decimal_comma,
        temperature,
        rate,
        flag_on: string(state, "flag.on").unwrap_or(defaults.flag_on),
        flag_off: string(state, "flag.off").unwrap_or(defaults.flag_off),
        decimals: block.style.decimals,
        unit: block.style.unit,
    }
}

/// Number and unit of `value` as `block` shows it: a sensor missing from
/// the profile's sensors reads `sensorAbsent`, frames read a dash while the
/// engine is not running, `fps-rendered` reads «FG?» on the heuristic, and
/// `bound` its translated text.
pub fn format_block_value(
    block: &Block,
    state: &OverlayState,
    value: Option<f64>,
) -> (String, String) {
    let dash = || (DASH.to_owned(), String::new());
    match &block.source {
        Source::Text(t) => (t.clone(), String::new()),
        Source::Sensor(id) => match state.sensors.get(id) {
            None => (
                string(state, "sensorAbsent").unwrap_or_else(|| DASH.to_owned()),
                String::new(),
            ),
            // An unknown unit is shown as a plain number.
            Some(info) => {
                let unit = parse_unit(&info.unit).unwrap_or(Unit::Count);
                format_value(value, unit, &format_options(block, state))
            }
        },
        Source::Frames(m) => {
            let Some(metrics) = state.metrics.as_ref().filter(|_| frames_running(state)) else {
                return dash();
            };
            match m {
                FrameMetric::Bound => match metrics.bound.as_deref() {
                    Some(b @ ("gpu" | "cpu")) => string(state, &format!("bound.{b}"))
                        .map_or_else(dash, |t| (t, String::new())),
                    _ => dash(),
                },
                FrameMetric::FpsRendered if metrics.fg_suspected => (
                    string(state, "fgSuspected").unwrap_or_else(|| "FG?".to_owned()),
                    String::new(),
                ),
                _ => format_frame_metric(*m, value, &format_options(block, state)),
            }
        }
    }
}

/// Whether `block` reads a percentage sensor (its automatic range is 0–100).
pub fn is_percent(block: &Block, state: &OverlayState) -> bool {
    match &block.source {
        Source::Sensor(id) => state
            .sensors
            .get(id)
            .is_some_and(|s| parse_unit(&s.unit) == Some(Unit::Percent)),
        _ => false,
    }
}

/// A `frametime` graph of a `frametime-*` source: one bar per frame.
pub fn is_frametime_chart(block: &Block) -> bool {
    block.kind == Kind::Graph
        && block.style.graph.mode == GraphMode::Frametime
        && matches!(
            block.source,
            Source::Frames(FrameMetric::FrametimeDisplayed | FrameMetric::FrametimeApp)
        )
}

/// The frametime of a frame for `block`'s source.
pub fn frame_ms(block: &Block, f: &WireFrameTime) -> Option<f64> {
    match block.source {
        Source::Frames(FrameMetric::FrametimeApp) => f.app_ms,
        _ => f.displayed_ms,
    }
}

/// The data generation a chart of `block` is drawn from: the frames for a
/// frametime chart, otherwise its source's ring.
pub fn chart_gen(block: &Block, state: &OverlayState) -> u64 {
    if is_frametime_chart(block) {
        return state.frames_gen();
    }
    SourceKey::of(&block.source).map_or(0, |k| state.data_gen(&k))
}

/// A graph's minimum, average and maximum over its range, as
/// `min / avg / max unit`; empty without data.
pub fn stats_text(block: &Block, state: &OverlayState) -> String {
    let range = block.style.graph.range_s;
    let (min, avg, max) = if is_frametime_chart(block) {
        let newest = state.frame_times.back().map_or(0.0, |f| f.t_s);
        let from = newest - f64::from(range);
        let (mut lo, mut hi, mut sum, mut n) = (f64::INFINITY, f64::NEG_INFINITY, 0.0, 0u32);
        for ms in state
            .frame_times
            .iter()
            .filter(|f| f.t_s >= from)
            .filter_map(|f| frame_ms(block, f))
        {
            lo = lo.min(ms);
            hi = hi.max(ms);
            sum += ms;
            n += 1;
        }
        if n == 0 {
            return String::new();
        }
        (Some(lo), Some(sum / f64::from(n)), Some(hi))
    } else {
        let stat = |op| {
            let s = Stat {
                op,
                window: range,
                ..block.stat
            };
            source_value(&block.source, &s, state)
        };
        (stat(StatOp::Min), stat(StatOp::Avg), stat(StatOp::Max))
    };
    if avg.is_none() {
        return String::new();
    }
    // Frames read through the frame formatter even with the engine stopped:
    // these are the kept frames, not the live metrics.
    let fmt = |v| match &block.source {
        Source::Frames(m) => format_frame_metric(*m, v, &format_options(block, state)),
        _ => format_block_value(block, state, v),
    };
    let (lo, _) = fmt(min);
    let (mid, _) = fmt(avg);
    let (hi, unit) = fmt(max);
    oma_core::format::join(&format!("{lo} / {mid} / {hi}"), &unit)
}

/// The label of `block`: its own, the sensor's, or the metric's name; a low
/// without its own label also names its definition (§4.4).
pub fn block_label(block: &Block, state: &OverlayState) -> String {
    if let Some(label) = &block.style.label {
        return label.clone();
    }
    match &block.source {
        Source::Text(_) => String::new(),
        Source::Sensor(id) => state
            .sensors
            .get(id)
            .map(|s| s.label.clone())
            .unwrap_or_default(),
        Source::Frames(m) => {
            let name = string(state, &format!("metric.{}", metric_name(*m))).unwrap_or_default();
            if !matches!(m, FrameMetric::Low1 | FrameMetric::Low01) {
                return name;
            }
            let key = format!("low.{}", definition_name(block.stat.definition));
            match string(state, &key) {
                Some(def) if !name.is_empty() => format!("{name} {def}"),
                Some(def) => def,
                None => name,
            }
        }
    }
}

/// The label, value and unit of `block` (spec §6.2).
pub fn text_parts(block: &Block, state: &OverlayState) -> TextParts {
    let (value, unit) = format_block_value(block, state, block_value(block, state));
    TextParts {
        label: block_label(block, state),
        value,
        unit,
    }
}

/// The y range of a chart: fixed, or the visible minimum to maximum with a
/// 10% margin; all-equal values are centred, `v ± max(10% of |v|, 0.5)`.
fn y_range(values: impl Iterator<Item = f64>, y: &YAxis) -> Option<(f64, f64)> {
    if y.mode == AxisMode::Fixed {
        return (y.max > y.min).then_some((y.min, y.max));
    }
    let (lo, hi) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
        (lo.min(v), hi.max(v))
    });
    if !(lo.is_finite() && hi.is_finite()) {
        return None;
    }
    if hi == lo {
        let half = (lo.abs() * 0.1).max(0.5);
        return Some((lo - half, hi + half));
    }
    let margin = (hi - lo) * 0.1;
    Some((lo - margin, hi + margin))
}

/// The chart points of `samples` (`(t_s, value)`, oldest first) in `rect`:
/// the last `range_s` seconds up to `now_s`, which is the right edge; the y
/// range fixed (values clamped) or automatic.
pub fn graph_points(
    samples: &[(f64, f64)],
    rect: RectF,
    range_s: f64,
    y: &YAxis,
    now_s: f64,
) -> Vec<(f32, f32)> {
    if range_s <= 0.0 {
        return Vec::new();
    }
    let from = now_s - range_s;
    let visible = || {
        samples
            .iter()
            .copied()
            .filter(move |&(t, v)| t >= from && t <= now_s && v.is_finite())
    };
    let Some((lo, hi)) = y_range(visible().map(|(_, v)| v), y) else {
        return Vec::new();
    };
    let (w, h) = (f64::from(rect.w), f64::from(rect.h));
    visible()
        .map(|(t, v)| {
            let x = f64::from(rect.x) + w * (1.0 - (now_s - t) / range_s);
            let f = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
            let y = f64::from(rect.y) + h * (1.0 - f);
            (x as f32, y as f32)
        })
        .collect()
}

/// One bar per frame (`(t_s, frametime_ms)`, `t_s` when it ended, oldest
/// first), as wide as its frametime, the newest ending on the right edge
/// (anchored to the last frame, DP12); heights from 0 to the longest
/// visible frame plus 10%.
pub fn frametime_bars(samples: &[(f64, f64)], rect: RectF, range_s: f64) -> Vec<RectF> {
    let Some(&(newest, _)) = samples.last() else {
        return Vec::new();
    };
    if range_s <= 0.0 || !newest.is_finite() {
        return Vec::new();
    }
    let from = newest - range_s;
    let visible = || {
        samples
            .iter()
            .copied()
            .filter(move |&(t, ft)| t > from && t <= newest && ft.is_finite() && ft > 0.0)
    };
    let top = visible().map(|(_, ft)| ft).fold(0.0, f64::max) * 1.1;
    if top <= 0.0 {
        return Vec::new();
    }
    let (w, h) = (f64::from(rect.w), f64::from(rect.h));
    let x_of = |t: f64| f64::from(rect.x) + w * (1.0 - (newest - t) / range_s);
    visible()
        .map(|(t, ft)| {
            let x1 = x_of(t);
            let x0 = x_of(t - ft / 1000.0).max(f64::from(rect.x));
            let bh = h * ft / top;
            RectF {
                x: x0 as f32,
                y: (f64::from(rect.y) + h - bh) as f32,
                w: (x1 - x0) as f32,
                h: bh as f32,
            }
        })
        .collect()
}

/// How full a meter is: `value` in `min..max`, clamped to 0–1; 0 for an
/// empty range or no value.
pub fn meter_fraction(value: f64, min: f64, max: f64) -> f32 {
    if !(value.is_finite() && min.is_finite() && max.is_finite()) || max <= min {
        return 0.0;
    }
    ((value - min) / (max - min)).clamp(0.0, 1.0) as f32
}

/// The sweep of a gauge's value arc, in degrees of its 270° arc.
pub fn gauge_sweep(value: f64, min: f64, max: f64) -> f32 {
    meter_fraction(value, min, max) * 270.0
}

/// The highest value of the block's source in the last [`AUTO_RANGE_S`]
/// seconds (DD16), from its ring; `None` without samples.
pub fn auto_max(block: &Block, state: &OverlayState) -> Option<f64> {
    let ring = state.rings.get(&range_key(&block.source)?)?;
    ring.value(&Stat {
        op: StatOp::Max,
        window: AUTO_RANGE_S,
        ..Stat::default()
    })
}

/// The range of a meter or gauge: each bound fixed, or automatic. The
/// automatic range of a percentage is 0 to 100; otherwise it runs from 0
/// (or the value, when negative) up to the highest recent value,
/// `recent_max` (see [`auto_max`]), so a reading is never shown as always
/// full and an old spike does not keep the scale wide.
pub fn value_range(
    min: RangeBound,
    max: RangeBound,
    percent: bool,
    value: Option<f64>,
    recent_max: Option<f64>,
) -> (f64, f64) {
    let v = value.unwrap_or(0.0);
    let lo = match min {
        RangeBound::Fixed(m) => m,
        RangeBound::Auto if percent => 0.0,
        RangeBound::Auto => v.min(0.0),
    };
    let hi = match max {
        RangeBound::Fixed(m) => m,
        RangeBound::Auto if percent => 100.0,
        RangeBound::Auto => v.max(recent_max.unwrap_or(v)).max(0.0).max(lo),
    };
    (lo, hi)
}

/// The benchmark badge: `● REC mm:ss`, stopping at `60:00` (the capture's
/// limit).
pub fn rec_text(seconds: u32) -> String {
    let s = seconds.min(3600);
    format!("\u{25cf} REC {:02}:{:02}", s / 60, s % 60)
}

/// The four rows of the benchmark summary: average FPS, 1% and 0.1% lows
/// and stutters, with the labels from `strings` (English without them).
pub fn bench_rows(sum: &WireBenchmarkSummary, state: &OverlayState) -> [TextParts; 4] {
    let opts = FormatOptions {
        decimal_comma: state.draw.decimal_comma,
        ..FormatOptions::default()
    };
    let row = |key: &str, default: &str, metric: FrameMetric, v: f64| {
        let (value, unit) = format_frame_metric(metric, Some(v), &opts);
        TextParts {
            label: string(state, key).unwrap_or_else(|| default.to_owned()),
            value,
            unit,
        }
    };
    [
        row(
            "bench.avg",
            "Avg FPS",
            FrameMetric::FpsDisplayed,
            sum.fps_displayed,
        ),
        row(
            "bench.low1",
            "1% low",
            FrameMetric::Low1,
            sum.low_one_percent,
        ),
        row(
            "bench.low01",
            "0.1% low",
            FrameMetric::Low01,
            sum.low_point_one_percent,
        ),
        row(
            "bench.stutter",
            "Stutter",
            FrameMetric::Stutter,
            f64::from(sum.stutter_count),
        ),
    ]
}

/// The size of one laid-out text: width, ascent (top to baseline) and
/// descent (baseline to bottom); all zero for an empty part.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextBox {
    pub w: f32,
    pub ascent: f32,
    pub descent: f32,
}

/// The top-left corners of label, value and unit in one row of `rect`: the
/// label on the left; value and unit together, placed in the rest by
/// `align`; one shared baseline, centred vertically. `gap` separates label
/// and value, `unit_gap` value and unit.
pub fn row_positions(
    rect: RectF,
    parts: [TextBox; 3],
    align: Align,
    gap: f32,
    unit_gap: f32,
) -> [(f32, f32); 3] {
    let [label, value, unit] = parts;
    let ascent = parts.iter().map(|p| p.ascent).fold(0.0, f32::max);
    let descent = parts.iter().map(|p| p.descent).fold(0.0, f32::max);
    let baseline = rect.y + (rect.h - (ascent + descent)) / 2.0 + ascent;
    let start = if label.w > 0.0 {
        rect.x + label.w + gap
    } else {
        rect.x
    };
    let unit_gap = if unit.w > 0.0 && value.w > 0.0 {
        unit_gap
    } else {
        0.0
    };
    let group = value.w + unit_gap + unit.w;
    let free = (rect.right() - start - group).max(0.0);
    let value_x = match align {
        Align::Left => start,
        Align::Center => start + free / 2.0,
        Align::Right => start + free,
    };
    [
        (rect.x, baseline - label.ascent),
        (value_x, baseline - value.ascent),
        (value_x + value.w + unit_gap, baseline - unit.ascent),
    ]
}

/// Where a block draws its texts and its shape (chart, bar, arc).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Areas {
    pub text: RectF,
    pub shape: RectF,
}

/// The text and shape areas of a block of `kind` in `rect`:
/// - `text` and `graph`: the whole block (a graph's texts lie over it);
/// - `sparkline`: the text on the left 60%, the chart on the right 40%,
///   15% in from top and bottom;
/// - `meter`: the bar at the bottom (30% high, at least 2 px) under the
///   text, or under the whole block when it is less than two cells high; a
///   vertical bar on the left (25% wide), the text half a cell after it;
/// - `gauge`: the shape is the centred square, the text the whole block.
pub fn areas(kind: Kind, orientation: Orientation, rect: RectF, cell: f32) -> Areas {
    let whole = Areas {
        text: rect,
        shape: rect,
    };
    match kind {
        Kind::Text | Kind::Graph => whole,
        Kind::Sparkline => {
            let tw = rect.w * 3.0 / 5.0;
            let inset = rect.h * 3.0 / 20.0;
            Areas {
                text: RectF { w: tw, ..rect },
                shape: RectF {
                    x: rect.x + tw,
                    y: rect.y + inset,
                    w: rect.w - tw,
                    h: rect.h - 2.0 * inset,
                },
            }
        }
        Kind::Meter => match orientation {
            Orientation::Horizontal if rect.h < 2.0 * cell => whole,
            Orientation::Horizontal => {
                let bh = (rect.h * 3.0 / 10.0).max(2.0).min(rect.h);
                Areas {
                    text: RectF {
                        h: rect.h - bh,
                        ..rect
                    },
                    shape: RectF {
                        y: rect.bottom() - bh,
                        h: bh,
                        ..rect
                    },
                }
            }
            Orientation::Vertical => {
                let bw = (rect.w * 0.25).max(2.0).min(rect.w);
                let tx = (rect.x + bw + cell / 2.0).min(rect.right());
                Areas {
                    text: RectF {
                        x: tx,
                        w: rect.right() - tx,
                        ..rect
                    },
                    shape: RectF { w: bw, ..rect },
                }
            }
        },
        Kind::Gauge => {
            let side = rect.w.min(rect.h);
            Areas {
                text: rect,
                shape: RectF {
                    x: rect.x + (rect.w - side) / 2.0,
                    y: rect.y + (rect.h - side) / 2.0,
                    w: side,
                    h: side,
                },
            }
        }
    }
}

/// Frametime bars narrower than a pixel merged per pixel column, keeping
/// the tallest, so a long range draws at most one bar per pixel.
pub fn merge_bars(bars: &[RectF]) -> Vec<RectF> {
    let mut out: Vec<RectF> = Vec::new();
    for &bar in bars {
        if let Some(last) = out.last_mut() {
            if last.w < 1.0 && bar.x <= last.right() + 0.5 {
                let right = last.right().max(bar.right());
                let bottom = last.bottom().max(bar.bottom());
                last.x = last.x.min(bar.x);
                last.y = last.y.min(bar.y);
                last.w = right - last.x;
                last.h = bottom - last.y;
                continue;
            }
        }
        out.push(bar);
    }
    out
}

/// The point at `deg` degrees on a circle, 0° to the right and growing
/// clockwise on screen (y down).
pub fn arc_point(center: (f32, f32), radius: f32, deg: f32) -> (f32, f32) {
    let rad = deg.to_radians();
    (center.0 + radius * rad.cos(), center.1 + radius * rad.sin())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::overlay::AxisMode;
    use oma_ipc::overlay::{FrameMetrics, SensorInfo, WireLow};
    use serde_json::json;
    use std::collections::BTreeMap;

    const CPU: &str = "cpu/temperature/package";

    fn block(v: serde_json::Value) -> Block {
        let mut b =
            json!({ "id": "b", "rect": { "x": 2, "y": 1, "w": 10, "h": 2 }, "kind": "text" });
        for (k, val) in v.as_object().expect("object") {
            b[k] = val.clone();
        }
        serde_json::from_value(b).expect("block")
    }

    fn metrics(state: &str) -> FrameMetrics {
        FrameMetrics {
            state: state.into(),
            fps_displayed: Some(100.4),
            fps_rendered: None,
            fps_presented: None,
            rendered_source: None,
            fg_suspected: false,
            frametime_displayed_ms: Some(9.96),
            frametime_app_ms: None,
            fg_multiplier: None,
            stutter_count: None,
            stutter_percent: None,
            latency_pc_ms: None,
            latency_display_ms: None,
            bound: Some("gpu".into()),
            lows: vec![WireLow {
                window_s: 10,
                definition: "percentile".into(),
                one_percent: Some(61.2),
                point_one_percent: Some(40.0),
            }],
        }
    }

    fn state() -> OverlayState {
        let mut s = OverlayState::default();
        s.sensors.insert(
            CPU.into(),
            SensorInfo {
                id: CPU.into(),
                label: "CPU".into(),
                unit: "celsius".into(),
            },
        );
        s.values.insert(CPU.into(), (Some(45.4), "fresh".into()));
        s.strings = BTreeMap::from(
            [
                ("sensorAbsent", "sensor missing"),
                ("fgSuspected", "FG?"),
                ("bound.gpu", "GPU-bound"),
                ("bound.cpu", "CPU-bound"),
                ("metric.fps-displayed", "FPS"),
                ("metric.fps-rendered", "Rendered"),
                ("metric.low-1", "1% low"),
                ("metric.bound", "Bound"),
                ("low.integral", "(integral)"),
                ("low.percentile", "(percentile)"),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned())),
        );
        s.metrics = Some(metrics("running"));
        s
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> RectF {
        RectF { x, y, w, h }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn block_rect_scales_with_cell() {
        let b = block(json!({ "source": { "text": "x" } }));
        assert_eq!(block_px(&b, (8.0, 8.0), 8.0), rect(24.0, 16.0, 80.0, 16.0));
        assert_eq!(
            block_px(&b, (8.0, 8.0), 16.0),
            rect(40.0, 24.0, 160.0, 32.0)
        );
        // A footprint that starts left of zero moves the origin, not the block.
        assert_eq!(
            block_px(&b, (-8.0, 0.0), 12.0),
            rect(16.0, 12.0, 120.0, 24.0)
        );
    }

    #[test]
    fn graph_points_map_values_to_rect_with_auto_range() {
        let r = rect(10.0, 20.0, 100.0, 50.0);
        // The sample at -1 is outside the 2 s range: neither drawn nor scaled.
        let samples = [(-1.0, 1000.0), (0.0, 10.0), (1.0, 20.0), (2.0, 30.0)];
        let p = graph_points(&samples, r, 2.0, &YAxis::default(), 2.0);
        assert_eq!(p.len(), 3);
        // 10..30 with a 10% margin: 8..32.
        let want = [
            (10.0, 20.0 + 50.0 * (1.0 - 2.0 / 24.0)),
            (60.0, 45.0),
            (110.0, 20.0 + 50.0 * (2.0 / 24.0)),
        ];
        for ((x, y), (wx, wy)) in p.iter().zip(want) {
            assert!(close(*x, wx) && close(*y, wy), "{p:?}");
        }
        // All equal: centred, v ± max(10% of |v|, 0.5).
        let p = graph_points(&[(0.0, 0.0), (1.0, 0.0)], r, 2.0, &YAxis::default(), 1.0);
        assert!(p.iter().all(|&(_, y)| close(y, 45.0)), "{p:?}");
        assert!(close(p[1].0, 110.0) && close(p[0].0, 60.0), "{p:?}");
        let p = graph_points(&[(0.0, 50.0), (1.0, 50.0)], r, 2.0, &YAxis::default(), 1.0);
        assert!(p.iter().all(|&(_, y)| close(y, 45.0)), "{p:?}");
        assert!(graph_points(&[], r, 2.0, &YAxis::default(), 1.0).is_empty());
    }

    #[test]
    fn fixed_y_range_clamps() {
        let r = rect(0.0, 0.0, 100.0, 50.0);
        let y = YAxis {
            mode: AxisMode::Fixed,
            min: 0.0,
            max: 100.0,
        };
        let p = graph_points(&[(0.0, 150.0), (1.0, 50.0), (2.0, -10.0)], r, 2.0, &y, 2.0);
        let ys: Vec<f32> = p.iter().map(|&(_, y)| y).collect();
        assert_eq!(ys, vec![0.0, 25.0, 50.0]);
    }

    #[test]
    fn frametime_bars_one_per_frame_newest_right() {
        let r = rect(0.0, 0.0, 100.0, 40.0);
        // t is when the frame ended; its bar spans its frametime.
        let samples = [(0.5, 10.0), (1.0, 10.0), (1.01, 10.0), (1.03, 20.0)];
        let bars = frametime_bars(&samples, r, 0.1);
        // The frame at 0.5 ends before the 0.1 s window: no bar.
        assert_eq!(bars.len(), 3, "{bars:?}");
        let newest = bars[2];
        assert!(
            close(newest.x + newest.w, 100.0) && close(newest.x, 80.0),
            "{bars:?}"
        );
        assert!(close(bars[1].x, 70.0) && close(bars[1].w, 10.0), "{bars:?}");
        assert!(close(bars[0].x, 60.0), "{bars:?}");
        // Heights scale to the longest frame plus 10%, bottom-aligned.
        assert!(close(newest.h, 40.0 * 20.0 / 22.0), "{newest:?}");
        assert!(close(bars[0].h, 40.0 * 10.0 / 22.0), "{bars:?}");
        assert!(bars.iter().all(|b| close(b.y + b.h, 40.0)));
        assert!(frametime_bars(&[], r, 0.1).is_empty());
    }

    #[test]
    fn meter_fraction_clamped() {
        assert_eq!(meter_fraction(50.0, 0.0, 100.0), 0.5);
        assert_eq!(meter_fraction(150.0, 0.0, 100.0), 1.0);
        assert_eq!(meter_fraction(-5.0, 0.0, 100.0), 0.0);
        assert_eq!(meter_fraction(75.0, 50.0, 100.0), 0.5);
        // An empty or broken range, or no value, is empty.
        assert_eq!(meter_fraction(5.0, 10.0, 10.0), 0.0);
        assert_eq!(meter_fraction(f64::NAN, 0.0, 100.0), 0.0);
    }

    #[test]
    fn gauge_sweep_from_value() {
        assert_eq!(gauge_sweep(50.0, 0.0, 100.0), 135.0);
        assert_eq!(gauge_sweep(100.0, 0.0, 100.0), 270.0);
        assert_eq!(gauge_sweep(200.0, 0.0, 100.0), 270.0);
        assert_eq!(gauge_sweep(0.0, 0.0, 100.0), 0.0);
    }

    #[test]
    fn text_parts_use_label_override_or_sensor_label() {
        let s = state();
        let b = block(json!({ "source": { "sensor": CPU } }));
        let p = text_parts(&b, &s);
        assert_eq!(
            p,
            TextParts {
                label: "CPU".into(),
                value: "45".into(),
                unit: "\u{b0}C".into()
            }
        );
        let b = block(json!({ "source": { "sensor": CPU }, "style": { "label": "Proc" } }));
        assert_eq!(text_parts(&b, &s).label, "Proc");
        // A frame metric reads its translated name.
        let b = block(json!({ "source": { "frames": "fps-displayed" } }));
        let p = text_parts(&b, &s);
        assert_eq!(
            (p.label.as_str(), p.value.as_str(), p.unit.as_str()),
            ("FPS", "100", "FPS")
        );
        // A low names its definition (§4.4) and reads the matching window.
        let b = block(json!({
            "source": { "frames": "low-1" },
            "stat": { "window": 10, "definition": "percentile" }
        }));
        let p = text_parts(&b, &s);
        assert_eq!(
            (p.label.as_str(), p.value.as_str()),
            ("1% low (percentile)", "61")
        );
        // An explicit label is kept as it is, without the definition.
        let b = block(json!({
            "source": { "frames": "low-1" },
            "stat": { "window": 10, "definition": "percentile" },
            "style": { "label": "1%" }
        }));
        assert_eq!(text_parts(&b, &s).label, "1%");
        // A low over a window the app did not compute is absent.
        let b = block(json!({ "source": { "frames": "low-1" }, "stat": { "window": 30 } }));
        assert_eq!(text_parts(&b, &s).value, "\u{2014}");
        // A fixed text is the value.
        let b = block(json!({ "source": { "text": "Hello" } }));
        assert_eq!(text_parts(&b, &s).value, "Hello");
    }

    #[test]
    fn missing_sensor_shows_sensor_absent() {
        let s = state();
        let b = block(json!({ "source": { "sensor": "gpu9/load/core" } }));
        let p = text_parts(&b, &s);
        assert_eq!(p.value, "sensor missing");
        assert_eq!(p.unit, "");
    }

    #[test]
    fn frames_show_dash_unless_running() {
        let mut s = state();
        let fps = block(json!({ "source": { "frames": "fps-displayed" } }));
        let bound = block(json!({ "source": { "frames": "bound" } }));
        assert_eq!(text_parts(&bound, &s).value, "GPU-bound");
        for state in ["stopped", "unavailable", "denied"] {
            s.metrics = Some(metrics(state));
            let p = text_parts(&fps, &s);
            assert_eq!(
                (p.value.as_str(), p.unit.as_str()),
                ("\u{2014}", ""),
                "{state}"
            );
            assert_eq!(text_parts(&bound, &s).value, "\u{2014}", "{state}");
        }
        // No metrics yet: a dash too.
        s.metrics = None;
        assert_eq!(text_parts(&fps, &s).value, "\u{2014}");
        // A bound the app could not tell reads as a dash.
        let mut m = metrics("running");
        m.bound = Some("unknown".into());
        s.metrics = Some(m);
        assert_eq!(text_parts(&bound, &s).value, "\u{2014}");
    }

    #[test]
    fn rendered_shows_fg_suspected() {
        let mut s = state();
        let b = block(json!({ "source": { "frames": "fps-rendered" } }));
        assert_eq!(text_parts(&b, &s).value, "\u{2014}");
        let mut m = metrics("running");
        m.fg_suspected = true;
        s.metrics = Some(m);
        let p = text_parts(&b, &s);
        assert_eq!(
            (p.label.as_str(), p.value.as_str(), p.unit.as_str()),
            ("Rendered", "FG?", "")
        );
    }

    #[test]
    fn metric_names_and_units_match_their_serde_spelling() {
        use oma_core::model::Unit;
        for m in [
            FrameMetric::FpsDisplayed,
            FrameMetric::FpsRendered,
            FrameMetric::FpsPresented,
            FrameMetric::FrametimeDisplayed,
            FrameMetric::FrametimeApp,
            FrameMetric::Low1,
            FrameMetric::Low01,
            FrameMetric::FgMultiplier,
            FrameMetric::Stutter,
            FrameMetric::LatencyPc,
            FrameMetric::LatencyDisplay,
            FrameMetric::Bound,
        ] {
            assert_eq!(serde_json::to_value(m).unwrap(), json!(metric_name(m)));
        }
        for u in [
            Unit::Celsius,
            Unit::Percent,
            Unit::Megahertz,
            Unit::Watt,
            Unit::Volt,
            Unit::Ampere,
            Unit::Rpm,
            Unit::Bytes,
            Unit::BytesPerSecond,
            Unit::BitsPerSecond,
            Unit::Joule,
            Unit::Boolean,
            Unit::PcieGeneration,
            Unit::Lanes,
            Unit::Hours,
            Unit::Count,
        ] {
            let name = serde_json::to_value(u).unwrap();
            assert_eq!(parse_unit(name.as_str().unwrap()), Some(u));
        }
        assert_eq!(parse_unit("parsecs"), None);
    }

    #[test]
    fn profile_frame_matches_the_placed_panel() {
        // Blocks from cell (2, 1) to (12, 3), padding 1: 12 x 4 cells.
        let p = oma_core::overlay::parse_profile(
            r#"{ "format": 1, "name": "t", "scale": 1.5, "blocks": [
                { "id": "a", "rect": { "x": 2, "y": 1, "w": 10, "h": 2 },
                  "source": { "text": "x" }, "kind": "text" } ] }"#,
        )
        .unwrap();
        let f = profile_frame(&p, 144).unwrap();
        assert_eq!(f.cell, 18.0);
        assert_eq!(f.size, (216.0, 72.0));
        // The first block starts one padding cell in.
        let r = block_px(&p.blocks[0], f.origin, f.cell);
        assert_eq!((r.x, r.y), (18.0, 18.0));
        let placed = oma_core::overlay::place(
            &p,
            oma_core::overlay::PxRect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
            144,
        )
        .unwrap();
        assert_eq!((placed.w as f32, placed.h as f32), f.size);
    }

    #[test]
    fn panel_radius_scales_with_the_cell_and_fits_the_panel() {
        assert_eq!(panel_radius_px(4.0, 8.0, 100.0, 100.0), 4.0);
        assert_eq!(panel_radius_px(4.0, 24.0, 100.0, 100.0), 12.0);
        assert_eq!(panel_radius_px(40.0, 8.0, 100.0, 20.0), 10.0);
        assert_eq!(panel_radius_px(-1.0, 8.0, 100.0, 20.0), 0.0);
    }

    #[test]
    fn value_range_auto_and_fixed() {
        use oma_core::overlay::RangeBound::{Auto, Fixed};
        assert_eq!(
            value_range(Auto, Auto, true, Some(40.0), Some(90.0)),
            (0.0, 100.0)
        );
        // Not a percentage: up to the highest recent value, so it is not
        // always full.
        assert_eq!(
            value_range(Auto, Auto, false, Some(40.0), Some(90.0)),
            (0.0, 90.0)
        );
        assert_eq!(
            value_range(Auto, Auto, false, Some(40.0), None),
            (0.0, 40.0)
        );
        assert_eq!(
            value_range(Auto, Auto, false, Some(-5.0), Some(-5.0)),
            (-5.0, 0.0)
        );
        assert_eq!(
            value_range(Fixed(10.0), Fixed(90.0), true, None, None),
            (10.0, 90.0)
        );
    }

    #[test]
    fn auto_range_forgets_a_spike_after_sixty_seconds() {
        use oma_core::overlay::RangeBound::Auto;
        use oma_ipc::overlay::{OverlayMessage, SetProfile, Values, WireValue};
        let meter = json!({ "id": "m", "rect": { "x": 0, "y": 0, "w": 10, "h": 2 },
            "source": { "sensor": CPU }, "kind": "meter" });
        let mut s = OverlayState::default();
        s.apply(
            OverlayMessage::SetProfile(SetProfile {
                profile_id: "p".into(),
                profile_json: json!({ "format": 1, "name": "t", "blocks": [meter] }).to_string(),
                sensors: vec![SensorInfo {
                    id: CPU.into(),
                    label: "CPU".into(),
                    unit: "celsius".into(),
                }],
                strings: BTreeMap::new(),
                draw: crate::state::default_draw(),
            }),
            0.0,
        );
        let push = |s: &mut OverlayState, t: u32, v: f64| {
            s.apply(
                OverlayMessage::Values(Values {
                    at_ms: u64::from(t) * 1000,
                    values: vec![WireValue {
                        id: CPU.into(),
                        value: Some(v),
                        quality: "fresh".into(),
                    }],
                }),
                f64::from(t),
            );
        };
        push(&mut s, 0, 100.0);
        for t in 1..=30 {
            push(&mut s, t, 10.0);
        }
        let b = s.profile.as_ref().expect("profile").blocks[0].clone();
        let range =
            |s: &OverlayState| value_range(Auto, Auto, false, block_value(&b, s), auto_max(&b, s));
        assert_eq!(range(&s), (0.0, 100.0), "the spike is recent");
        for t in 31..=61 {
            push(&mut s, t, 10.0);
        }
        assert_eq!(range(&s), (0.0, 10.0), "the spike is forgotten");
    }

    #[test]
    fn auto_range_on_a_low_is_not_always_full() {
        use oma_core::overlay::RangeBound::Auto;
        use oma_ipc::overlay::{OverlayMessage, SetProfile};
        let meter = json!({ "id": "m", "rect": { "x": 0, "y": 0, "w": 10, "h": 2 },
            "source": { "frames": "low-1" }, "kind": "meter",
            "stat": { "window": 10, "definition": "percentile" } });
        let mut s = OverlayState::default();
        s.apply(
            OverlayMessage::SetProfile(SetProfile {
                profile_id: "p".into(),
                profile_json: json!({ "format": 1, "name": "t", "blocks": [meter] }).to_string(),
                sensors: vec![],
                strings: BTreeMap::new(),
                draw: crate::state::default_draw(),
            }),
            0.0,
        );
        // fps-displayed 100.4, 1% low 61.2 (see `metrics`).
        s.apply(OverlayMessage::FrameMetrics(metrics("running")), 1.0);
        let b = s.profile.as_ref().expect("profile").blocks[0].clone();
        let v = block_value(&b, &s);
        assert_eq!(v, Some(61.2));
        let (lo, hi) = value_range(Auto, Auto, false, v, auto_max(&b, &s));
        assert_eq!((lo, hi), (0.0, 100.4), "scaled to the recent FPS");
        assert!(meter_fraction(61.2, lo, hi) < 1.0);
    }

    #[test]
    fn bench_rows_use_strings_or_english() {
        let sum = WireBenchmarkSummary {
            fps_displayed: 119.6,
            low_one_percent: 80.0,
            low_point_one_percent: 60.0,
            stutter_count: 4,
            stutter_percent: 0.5,
        };
        let mut s = OverlayState::default();
        let rows = bench_rows(&sum, &s);
        assert_eq!(rows[0].label, "Avg FPS");
        assert_eq!(
            (rows[0].value.as_str(), rows[0].unit.as_str()),
            ("120", "FPS")
        );
        assert_eq!(rows[3].label, "Stutter");
        assert_eq!((rows[3].value.as_str(), rows[3].unit.as_str()), ("4", ""));
        s.strings.insert("bench.avg".into(), "FPS medi".into());
        assert_eq!(bench_rows(&sum, &s)[0].label, "FPS medi");
    }

    #[test]
    fn rec_text_formats_minutes_and_seconds() {
        assert_eq!(rec_text(0), "\u{25cf} REC 00:00");
        assert_eq!(rec_text(187), "\u{25cf} REC 03:07");
        assert_eq!(rec_text(3599), "\u{25cf} REC 59:59");
        assert_eq!(rec_text(3600), "\u{25cf} REC 60:00");
        assert_eq!(rec_text(5000), "\u{25cf} REC 60:00");
    }

    #[test]
    fn row_shares_a_baseline_and_aligns_value_and_unit() {
        use oma_core::overlay::Align;
        let r = rect(0.0, 0.0, 100.0, 20.0);
        let label = TextBox {
            w: 30.0,
            ascent: 10.0,
            descent: 2.0,
        };
        let value = TextBox {
            w: 20.0,
            ascent: 14.0,
            descent: 4.0,
        };
        let unit = TextBox {
            w: 10.0,
            ascent: 10.0,
            descent: 2.0,
        };
        let [l, v, u] = row_positions(r, [label, value, unit], Align::Left, 4.0, 2.0);
        // Ascent 14 + descent 4 centred in 20: baseline at 15.
        assert_eq!(l, (0.0, 5.0));
        assert_eq!(v, (34.0, 1.0));
        assert_eq!(u, (56.0, 5.0));
        let [_, v, u] = row_positions(r, [label, value, unit], Align::Right, 4.0, 2.0);
        assert_eq!((v.0, u.0 + unit.w), (68.0, 100.0));
        let [_, v, _] = row_positions(r, [label, value, unit], Align::Center, 4.0, 2.0);
        assert_eq!(v.0, 51.0);
        // Without a label the value starts at the left edge.
        let [_, v, _] = row_positions(r, [TextBox::default(), value, unit], Align::Left, 4.0, 2.0);
        assert_eq!(v.0, 0.0);
    }

    #[test]
    fn areas_split_text_and_shape_by_kind() {
        use oma_core::overlay::{Kind, Orientation};
        let r = rect(0.0, 0.0, 100.0, 40.0);
        let h = Orientation::Horizontal;
        // Text and graph: both the whole block.
        assert_eq!(areas(Kind::Text, h, r, 8.0), Areas { text: r, shape: r });
        assert_eq!(areas(Kind::Graph, h, r, 8.0).shape, r);
        // A sparkline: text on the left 60%, the chart on the right 40%.
        let a = areas(Kind::Sparkline, h, r, 8.0);
        assert_eq!(a.text, rect(0.0, 0.0, 60.0, 40.0));
        assert_eq!(a.shape, rect(60.0, 6.0, 40.0, 28.0));
        // A horizontal meter: the bar at the bottom, 30% high.
        let a = areas(Kind::Meter, h, r, 8.0);
        assert_eq!(a.text, rect(0.0, 0.0, 100.0, 28.0));
        assert_eq!(a.shape, rect(0.0, 28.0, 100.0, 12.0));
        // One cell high: the bar fills the block under the text.
        let low = rect(0.0, 0.0, 100.0, 8.0);
        assert_eq!(
            areas(Kind::Meter, h, low, 8.0),
            Areas {
                text: low,
                shape: low
            }
        );
        // A vertical meter: the bar on the left, a quarter wide.
        let a = areas(Kind::Meter, Orientation::Vertical, r, 8.0);
        assert_eq!(a.shape, rect(0.0, 0.0, 25.0, 40.0));
        assert_eq!(a.text, rect(29.0, 0.0, 71.0, 40.0));
        // A gauge: the centred square.
        let a = areas(Kind::Gauge, h, r, 8.0);
        assert_eq!(a.shape, rect(30.0, 0.0, 40.0, 40.0));
    }

    #[test]
    fn merge_bars_keeps_the_tallest_per_pixel() {
        let bars = [
            rect(0.0, 30.0, 0.3, 10.0),
            rect(0.3, 20.0, 0.3, 20.0),
            rect(0.6, 35.0, 0.3, 5.0),
            rect(5.0, 25.0, 3.0, 15.0),
        ];
        let m = merge_bars(&bars);
        assert_eq!(m.len(), 2, "{m:?}");
        assert!(
            close(m[0].x, 0.0) && close(m[0].w, 0.9) && close(m[0].y, 20.0) && close(m[0].h, 20.0),
            "{m:?}"
        );
        assert_eq!(m[1], bars[3]);
        assert!(merge_bars(&[]).is_empty());
    }

    #[test]
    fn arc_point_is_clockwise_from_the_right() {
        let p = arc_point((10.0, 10.0), 5.0, 0.0);
        assert!(close(p.0, 15.0) && close(p.1, 10.0));
        // 90° is down on screen.
        let p = arc_point((10.0, 10.0), 5.0, 90.0);
        assert!(close(p.0, 10.0) && close(p.1, 15.0));
    }

    #[test]
    fn font_size_is_in_points() {
        // 12 pt at 96 DPI and scale 1 (an 8 px cell) is 16 px.
        assert_eq!(font_px(12.0, 8.0), 16.0);
        // Scale 1.5 at 144 DPI: an 18 px cell.
        assert_eq!(font_px(9.0, 18.0), 27.0);
    }

    #[test]
    fn percent_is_read_from_the_unit() {
        let mut s2 = state();
        s2.sensors.insert(
            "gpu/load/core".into(),
            SensorInfo {
                id: "gpu/load/core".into(),
                label: "GPU".into(),
                unit: "percent".into(),
            },
        );
        let gpu = block(json!({ "source": { "sensor": "gpu/load/core" } }));
        let cpu = block(json!({ "source": { "sensor": CPU } }));
        let fps = block(json!({ "source": { "frames": "fps-displayed" } }));
        assert!(is_percent(&gpu, &s2));
        assert!(!is_percent(&cpu, &s2));
        assert!(!is_percent(&fps, &s2));
    }

    #[test]
    fn frametime_chart_needs_a_frametime_source() {
        let g = |source: serde_json::Value, mode: &str| {
            block(
                json!({ "source": source, "kind": "graph", "style": { "graph": { "mode": mode } } }),
            )
        };
        assert!(is_frametime_chart(&g(
            json!({ "frames": "frametime-displayed" }),
            "frametime"
        )));
        assert!(is_frametime_chart(&g(
            json!({ "frames": "frametime-app" }),
            "frametime"
        )));
        assert!(!is_frametime_chart(&g(
            json!({ "frames": "frametime-app" }),
            "line"
        )));
        assert!(!is_frametime_chart(&g(
            json!({ "frames": "fps-displayed" }),
            "frametime"
        )));
    }

    #[test]
    fn chart_gen_follows_the_chart_source() {
        let mut s = state();
        let json = r#"{ "format": 1, "name": "t", "blocks": [
            { "id": "g", "rect": { "x": 0, "y": 0, "w": 10, "h": 2 }, "kind": "sparkline",
              "source": { "sensor": "cpu/temperature/package" } },
            { "id": "t", "rect": { "x": 0, "y": 2, "w": 10, "h": 2 }, "kind": "graph",
              "source": { "frames": "frametime-displayed" },
              "style": { "graph": { "mode": "frametime" } } } ] }"#;
        s.apply(
            oma_ipc::overlay::OverlayMessage::SetProfile(oma_ipc::overlay::SetProfile {
                profile_id: "p".into(),
                profile_json: json.into(),
                sensors: s.sensors.values().cloned().collect(),
                strings: s.strings.clone(),
                draw: crate::state::default_draw(),
            }),
            0.0,
        );
        let p = s.profile.clone().unwrap();
        let (spark, ft) = (&p.blocks[0], &p.blocks[1]);
        let (a, b) = (chart_gen(spark, &s), chart_gen(ft, &s));
        s.apply(
            oma_ipc::overlay::OverlayMessage::FrameTimes(oma_ipc::overlay::FrameTimes {
                frames: vec![oma_ipc::overlay::WireFrameTime {
                    t_s: 1.0,
                    displayed_ms: Some(7.0),
                    app_ms: None,
                }],
            }),
            1.0,
        );
        assert_eq!(chart_gen(spark, &s), a);
        assert!(chart_gen(ft, &s) > b);
        s.apply(
            oma_ipc::overlay::OverlayMessage::Values(oma_ipc::overlay::Values {
                at_ms: 0,
                values: vec![oma_ipc::overlay::WireValue {
                    id: CPU.into(),
                    value: Some(50.0),
                    quality: "fresh".into(),
                }],
            }),
            2.0,
        );
        assert!(chart_gen(spark, &s) > a);
    }

    #[test]
    fn stats_text_over_the_graph_range() {
        use oma_ipc::overlay::WireFrameTime;
        let mut s = state();
        let ft = block(json!({
            "source": { "frames": "frametime-displayed" }, "kind": "graph",
            "style": { "graph": { "mode": "frametime", "rangeS": 5 } }
        }));
        assert_eq!(stats_text(&ft, &s), "");
        // The frame at t = 0 is older than the 5 s range.
        for (t, ms) in [(0.0, 100.0), (6.0, 10.0), (7.0, 20.0), (8.0, 30.0)] {
            s.frame_times.push_back(WireFrameTime {
                t_s: t,
                displayed_ms: Some(ms),
                app_ms: None,
            });
        }
        assert_eq!(stats_text(&ft, &s), "10.0 / 20.0 / 30.0 ms");
        // The app frametime reads its own column, absent here.
        let app = block(json!({
            "source": { "frames": "frametime-app" }, "kind": "graph",
            "style": { "graph": { "mode": "frametime", "rangeS": 5 } }
        }));
        assert_eq!(stats_text(&app, &s), "");
    }
}
