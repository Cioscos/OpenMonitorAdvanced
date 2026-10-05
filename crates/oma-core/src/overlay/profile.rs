//! Overlay profile: serde model, strict validation and limits (spec §6).

use std::collections::HashSet;
use std::fmt;

use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::frames::metrics::LowDefinition;

pub const PROFILE_FORMAT: u32 = 1;
pub const MAX_BLOCKS: usize = 256;
pub const MAX_TEXT_BYTES: usize = 65_536;
pub const MAX_PROFILE_BYTES: usize = 1_048_576;
/// Edge of one layout cell at scale 1.0 and 96 dpi, in logical pixels.
pub const CELL_PX: f64 = 8.0;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("profile is larger than {MAX_PROFILE_BYTES} bytes")]
    TooLarge,
    #[error("invalid profile JSON: {0}")]
    Json(String),
    #[error("unsupported profile format {0}")]
    Format(u32),
    #[error("too many blocks ({0})")]
    TooManyBlocks(usize),
    #[error("duplicate block id {0:?}")]
    DuplicateBlock(String),
    #[error("value out of range at {path}")]
    OutOfRange { path: String },
    #[error("text too long in block {block:?}")]
    TextTooLong { block: String },
    #[error("value not finite at {path}")]
    NotFinite { path: String },
}

/// Reads and validates a profile from JSON text.
pub fn parse_profile(json: &str) -> Result<Profile, ProfileError> {
    if json.len() > MAX_PROFILE_BYTES {
        return Err(ProfileError::TooLarge);
    }
    let profile: Profile =
        serde_json::from_str(json).map_err(|e| ProfileError::Json(e.to_string()))?;
    profile.validate()?;
    Ok(profile)
}

// ---------------------------------------------------------------- colours

/// 8-bit RGBA colour, written as `#RRGGBB` (alpha 255) or `#RRGGBBAA`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const WHITE: Rgba = Rgba::rgb(255, 255, 255);
    pub const BLACK: Rgba = Rgba::rgb(0, 0, 0);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.strip_prefix('#')?;
        if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Self {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: if hex.len() == 8 { byte(6)? } else { 255 },
        })
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.a == 255 {
            write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            write!(
                f,
                "#{:02X}{:02X}{:02X}{:02X}",
                self.r, self.g, self.b, self.a
            )
        }
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgba::parse(&s).ok_or_else(|| {
            de::Error::custom(format!(
                "invalid colour {s:?}, expected #RRGGBB or #RRGGBBAA"
            ))
        })
    }
}

// ------------------------------------------------------------------ model

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellPoint {
    pub x: i32,
    pub y: i32,
}

impl Default for CellPoint {
    fn default() -> Self {
        Self { x: 1, y: 1 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Panel {
    #[serde(default = "Panel::default_color")]
    pub color: Rgba,
    #[serde(default = "Panel::default_opacity")]
    pub opacity: f64,
    #[serde(default = "Panel::default_radius")]
    pub radius: f64,
    #[serde(default = "Panel::default_padding")]
    pub padding: u32,
}

impl Panel {
    fn default_color() -> Rgba {
        Rgba::BLACK
    }
    fn default_opacity() -> f64 {
        0.35
    }
    fn default_radius() -> f64 {
        4.0
    }
    fn default_padding() -> u32 {
        1
    }
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            color: Self::default_color(),
            opacity: Self::default_opacity(),
            radius: Self::default_radius(),
            padding: Self::default_padding(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub format: u32,
    pub name: String,
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default)]
    pub offset: CellPoint,
    #[serde(default = "Profile::default_scale")]
    pub scale: f64,
    #[serde(default)]
    pub panel: Panel,
    #[serde(default)]
    pub blocks: Vec<Block>,
}

impl Profile {
    fn default_scale() -> f64 {
        1.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Block {
    pub id: String,
    pub rect: CellRect,
    #[serde(default)]
    pub z: i32,
    pub source: Source,
    #[serde(default)]
    pub stat: Stat,
    pub kind: Kind,
    #[serde(default)]
    pub style: Style,
    #[serde(default)]
    pub thresholds: Vec<Threshold>,
    #[serde(default)]
    pub visible_if: Option<VisibleIf>,
    #[serde(default)]
    pub panel: Option<Panel>,
}

/// What a block shows; a JSON object with exactly one of these keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// Sensor id in the form `<device_id>/<kind>/<name>`.
    Sensor(String),
    Frames(FrameMetric),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrameMetric {
    FpsDisplayed,
    FpsRendered,
    FpsPresented,
    FrametimeDisplayed,
    FrametimeApp,
    #[serde(rename = "low-1")]
    Low1,
    #[serde(rename = "low-01")]
    Low01,
    FgMultiplier,
    Stutter,
    LatencyPc,
    LatencyDisplay,
    Bound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum StatOp {
    #[default]
    Current,
    Min,
    Avg,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LowDefinitionKey {
    #[default]
    Integral,
    Percentile,
}

impl From<LowDefinitionKey> for LowDefinition {
    fn from(k: LowDefinitionKey) -> Self {
        match k {
            LowDefinitionKey::Integral => LowDefinition::Integral,
            LowDefinitionKey::Percentile => LowDefinition::Percentile,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Stat {
    #[serde(default)]
    pub op: StatOp,
    #[serde(default = "Stat::default_window")]
    pub window: u32,
    #[serde(default)]
    pub definition: LowDefinitionKey,
}

impl Stat {
    fn default_window() -> u32 {
        1
    }
}

impl Default for Stat {
    fn default() -> Self {
        Self {
            op: StatOp::Current,
            window: 1,
            definition: LowDefinitionKey::Integral,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Text,
    Graph,
    Meter,
    Sparkline,
    Gauge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// Display unit: automatic scaling or a fixed unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UnitChoice {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    B,
    KB,
    MB,
    GB,
    TB,
    MHz,
    GHz,
    #[serde(rename = "bit/s")]
    BitS,
    #[serde(rename = "kbit/s")]
    KbitS,
    #[serde(rename = "Mbit/s")]
    MbitS,
    #[serde(rename = "Gbit/s")]
    GbitS,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Style {
    #[serde(default)]
    pub label_style: TextStyle,
    #[serde(default)]
    pub value_style: TextStyle,
    #[serde(default)]
    pub unit_style: TextStyle,
    #[serde(default)]
    pub align: Align,
    #[serde(default)]
    pub decimals: Option<u8>,
    #[serde(default)]
    pub unit: UnitChoice,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub graph: GraphStyle,
    #[serde(default)]
    pub meter: RangeStyle,
    #[serde(default)]
    pub gauge: RangeStyle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextStyle {
    #[serde(default = "TextStyle::default_font")]
    pub font: String,
    #[serde(default = "TextStyle::default_size")]
    pub size: f32,
    #[serde(default = "TextStyle::default_weight")]
    pub weight: u16,
    #[serde(default)]
    pub italic: bool,
    #[serde(default = "TextStyle::default_color")]
    pub color: Rgba,
    #[serde(default)]
    pub outline: Option<Outline>,
    #[serde(default)]
    pub shadow: Option<Shadow>,
}

impl TextStyle {
    fn default_font() -> String {
        "Segoe UI".to_owned()
    }
    fn default_size() -> f32 {
        12.0
    }
    fn default_weight() -> u16 {
        600
    }
    fn default_color() -> Rgba {
        Rgba::WHITE
    }
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: Self::default_font(),
            size: Self::default_size(),
            weight: Self::default_weight(),
            italic: false,
            color: Self::default_color(),
            outline: None,
            shadow: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outline {
    pub width: f32,
    pub color: Rgba,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shadow {
    pub dx: f32,
    pub dy: f32,
    pub color: Rgba,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum GraphMode {
    #[default]
    Line,
    Area,
    Bars,
    Frametime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AxisMode {
    #[default]
    Auto,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YAxis {
    #[serde(default)]
    pub mode: AxisMode,
    #[serde(default)]
    pub min: f64,
    #[serde(default = "YAxis::default_max")]
    pub max: f64,
}

impl YAxis {
    fn default_max() -> f64 {
        100.0
    }
}

impl Default for YAxis {
    fn default() -> Self {
        Self {
            mode: AxisMode::Auto,
            min: 0.0,
            max: Self::default_max(),
        }
    }
}

const ACCENT: Rgba = Rgba::rgb(0x00, 0xE5, 0xFF);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    #[serde(default = "Stroke::default_color")]
    pub color: Rgba,
    #[serde(default = "Stroke::default_width")]
    pub width: f32,
}

impl Stroke {
    fn default_color() -> Rgba {
        ACCENT
    }
    fn default_width() -> f32 {
        1.5
    }
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            color: Self::default_color(),
            width: Self::default_width(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fill {
    #[serde(default = "Fill::default_color")]
    pub color: Rgba,
    #[serde(default = "Fill::default_alpha")]
    pub alpha: f64,
}

impl Fill {
    fn default_color() -> Rgba {
        ACCENT
    }
    fn default_alpha() -> f64 {
        0.25
    }
}

impl Default for Fill {
    fn default() -> Self {
        Self {
            color: Self::default_color(),
            alpha: Self::default_alpha(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphStyle {
    #[serde(default)]
    pub mode: GraphMode,
    #[serde(default = "GraphStyle::default_range_s")]
    pub range_s: u32,
    #[serde(default)]
    pub y: YAxis,
    #[serde(default)]
    pub line: Stroke,
    #[serde(default)]
    pub fill: Fill,
    #[serde(default = "GraphStyle::default_grid_lines")]
    pub grid_lines: u8,
    #[serde(default)]
    pub show_min_avg_max: bool,
    #[serde(default = "GraphStyle::default_show_value")]
    pub show_value: bool,
}

impl GraphStyle {
    fn default_range_s() -> u32 {
        60
    }
    fn default_grid_lines() -> u8 {
        2
    }
    fn default_show_value() -> bool {
        true
    }
}

impl Default for GraphStyle {
    fn default() -> Self {
        Self {
            mode: GraphMode::Line,
            range_s: Self::default_range_s(),
            y: YAxis::default(),
            line: Stroke::default(),
            fill: Fill::default(),
            grid_lines: Self::default_grid_lines(),
            show_min_avg_max: false,
            show_value: Self::default_show_value(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

/// Bound of a meter or gauge: derived from the data or a fixed number.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RangeBound {
    #[default]
    Auto,
    Fixed(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RangeStyle {
    #[serde(default)]
    pub orientation: Orientation,
    #[serde(default)]
    pub min: RangeBound,
    #[serde(default)]
    pub max: RangeBound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareOp {
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = "<=")]
    Le,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ThresholdTarget {
    #[default]
    Value,
    Graph,
    Panel,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Threshold {
    pub op: CompareOp,
    pub value: f64,
    pub color: Rgba,
    #[serde(default)]
    pub target: ThresholdTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FgState {
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FgCondition {
    pub fg: FgState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Comparison {
    pub source: Source,
    #[serde(default)]
    pub stat: Stat,
    pub op: CompareOp,
    pub value: f64,
}

/// Show a block only when the foreground game is active or a value compares true.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VisibleIf {
    Fg(FgCondition),
    Compare(Comparison),
}

// ------------------------------------------------------------- validation

fn oor(path: &str) -> ProfileError {
    ProfileError::OutOfRange {
        path: path.to_owned(),
    }
}

fn finite(path: &str, v: f64) -> Result<(), ProfileError> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(ProfileError::NotFinite {
            path: path.to_owned(),
        })
    }
}

fn range_f(path: &str, v: f64, lo: f64, hi: f64) -> Result<(), ProfileError> {
    finite(path, v)?;
    if (lo..=hi).contains(&v) {
        Ok(())
    } else {
        Err(oor(path))
    }
}

fn range_i(path: &str, v: i64, lo: i64, hi: i64) -> Result<(), ProfileError> {
    if (lo..=hi).contains(&v) {
        Ok(())
    } else {
        Err(oor(path))
    }
}

fn text_len(block: &str, s: &str) -> Result<(), ProfileError> {
    if s.len() > MAX_TEXT_BYTES {
        Err(ProfileError::TextTooLong {
            block: block.to_owned(),
        })
    } else {
        Ok(())
    }
}

fn check_panel(path: &str, p: &Panel) -> Result<(), ProfileError> {
    range_f(&format!("{path}.opacity"), p.opacity, 0.0, 1.0)?;
    range_f(&format!("{path}.radius"), p.radius, 0.0, 32.0)?;
    range_i(&format!("{path}.padding"), i64::from(p.padding), 0, 4)
}

fn check_text_style(path: &str, s: &TextStyle) -> Result<(), ProfileError> {
    if s.font.len() > 64 {
        return Err(oor(&format!("{path}.font")));
    }
    range_f(&format!("{path}.size"), f64::from(s.size), 6.0, 72.0)?;
    range_i(&format!("{path}.weight"), i64::from(s.weight), 100, 900)?;
    if let Some(o) = &s.outline {
        range_f(
            &format!("{path}.outline.width"),
            f64::from(o.width),
            0.5,
            4.0,
        )?;
    }
    if let Some(sh) = &s.shadow {
        range_f(&format!("{path}.shadow.dx"), f64::from(sh.dx), -8.0, 8.0)?;
        range_f(&format!("{path}.shadow.dy"), f64::from(sh.dy), -8.0, 8.0)?;
    }
    Ok(())
}

fn check_range_style(path: &str, r: &RangeStyle) -> Result<(), ProfileError> {
    for (name, b) in [("min", r.min), ("max", r.max)] {
        if let RangeBound::Fixed(v) = b {
            finite(&format!("{path}.{name}"), v)?;
        }
    }
    Ok(())
}

fn check_stat(path: &str, s: &Stat) -> Result<(), ProfileError> {
    range_i(&format!("{path}.window"), i64::from(s.window), 1, 300)
}

fn check_source_text(block: &str, s: &Source) -> Result<(), ProfileError> {
    match s {
        Source::Sensor(t) | Source::Text(t) => text_len(block, t),
        Source::Frames(_) => Ok(()),
    }
}

impl Profile {
    /// Checks every limit of the format; `parse_profile` calls this.
    pub fn validate(&self) -> Result<(), ProfileError> {
        if self.format != PROFILE_FORMAT {
            return Err(ProfileError::Format(self.format));
        }
        if self.blocks.len() > MAX_BLOCKS {
            return Err(ProfileError::TooManyBlocks(self.blocks.len()));
        }
        text_len("", &self.name)?;
        range_f("scale", self.scale, 0.5, 3.0)?;
        check_panel("panel", &self.panel)?;
        let mut seen = HashSet::new();
        for b in &self.blocks {
            if !seen.insert(b.id.as_str()) {
                return Err(ProfileError::DuplicateBlock(b.id.clone()));
            }
            check_block(b)?;
        }
        Ok(())
    }
}

fn check_block(b: &Block) -> Result<(), ProfileError> {
    let p = |s: &str| format!("blocks[{}].{s}", b.id);
    if b.id.is_empty() || b.id.len() > 64 {
        return Err(oor(&p("id")));
    }
    range_i(&p("rect.x"), i64::from(b.rect.x), 0, 400)?;
    range_i(&p("rect.y"), i64::from(b.rect.y), 0, 400)?;
    range_i(&p("rect.w"), i64::from(b.rect.w), 1, 200)?;
    range_i(&p("rect.h"), i64::from(b.rect.h), 1, 200)?;
    check_source_text(&b.id, &b.source)?;
    check_stat(&p("stat"), &b.stat)?;
    let s = &b.style;
    if let Some(l) = &s.label {
        text_len(&b.id, l)?;
    }
    check_text_style(&p("style.labelStyle"), &s.label_style)?;
    check_text_style(&p("style.valueStyle"), &s.value_style)?;
    check_text_style(&p("style.unitStyle"), &s.unit_style)?;
    if let Some(d) = s.decimals {
        range_i(&p("style.decimals"), i64::from(d), 0, 3)?;
    }
    let g = &s.graph;
    range_i(&p("style.graph.rangeS"), i64::from(g.range_s), 5, 300)?;
    finite(&p("style.graph.y.min"), g.y.min)?;
    finite(&p("style.graph.y.max"), g.y.max)?;
    range_f(
        &p("style.graph.line.width"),
        f64::from(g.line.width),
        0.5,
        4.0,
    )?;
    range_f(&p("style.graph.fill.alpha"), g.fill.alpha, 0.0, 1.0)?;
    range_i(&p("style.graph.gridLines"), i64::from(g.grid_lines), 0, 8)?;
    check_range_style(&p("style.meter"), &s.meter)?;
    check_range_style(&p("style.gauge"), &s.gauge)?;
    if b.thresholds.len() > 8 {
        return Err(oor(&p("thresholds")));
    }
    for (i, t) in b.thresholds.iter().enumerate() {
        finite(&p(&format!("thresholds[{i}].value")), t.value)?;
    }
    if let Some(VisibleIf::Compare(c)) = &b.visible_if {
        check_source_text(&b.id, &c.source)?;
        check_stat(&p("visibleIf.stat"), &c.stat)?;
        finite(&p("visibleIf.value"), c.value)?;
    }
    if let Some(panel) = &b.panel {
        check_panel(&p("panel"), panel)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn minimal() -> Value {
        json!({
            "format": 1,
            "name": "t",
            "blocks": [{
                "id": "a",
                "rect": {"x": 0, "y": 0, "w": 4, "h": 1},
                "source": {"frames": "fps-displayed"},
                "kind": "text"
            }]
        })
    }

    fn parse(v: &Value) -> Result<Profile, ProfileError> {
        parse_profile(&v.to_string())
    }

    /// Returns `minimal()` with the value at `path` replaced (or inserted).
    fn with(path: &[&str], value: Value) -> Value {
        let mut v = minimal();
        let mut cur = &mut v;
        for k in path {
            cur = match k.parse::<usize>() {
                Ok(i) => &mut cur[i],
                Err(_) => &mut cur[*k],
            };
        }
        *cur = value;
        v
    }

    #[test]
    fn profile_round_trip() {
        let mut v = minimal();
        v["anchor"] = json!("bottom-right");
        v["scale"] = json!(1.25);
        v["blocks"][0]["source"] = json!({"sensor": "cpu/load/total"});
        v["blocks"][0]["stat"] = json!({"op": "avg", "window": 10, "definition": "percentile"});
        v["blocks"][0]["style"] = json!({
            "unit": "Mbit/s", "decimals": 2, "label": "L",
            "valueStyle": {"color": "#11223344", "outline": {"width": 1.0, "color": "#000000"},
                           "shadow": {"dx": 1.0, "dy": -1.0, "color": "#00000080"}},
            "meter": {"orientation": "vertical", "min": {"fixed": 0.0}, "max": "auto"}
        });
        v["blocks"][0]["thresholds"] =
            json!([{"op": ">=", "value": 90.0, "color": "#FF0000", "target": "panel"}]);
        v["blocks"][0]["visibleIf"] = json!({"fg": "active"});
        v["blocks"][0]["panel"] = json!({"opacity": 0.5});
        let p = parse(&v).unwrap();
        let again = parse_profile(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, again);
        assert_eq!(p.anchor, Anchor::BottomRight);
        assert_eq!(p.blocks[0].style.unit, UnitChoice::MbitS);
        assert_eq!(
            p.blocks[0].visible_if,
            Some(VisibleIf::Fg(FgCondition {
                fg: FgState::Active
            }))
        );
        let cmp = json!({"source": {"sensor": "x/y/z"}, "op": "<", "value": 3.0});
        let p = parse(&with(&["blocks", "0", "visibleIf"], cmp)).unwrap();
        assert!(matches!(
            p.blocks[0].visible_if,
            Some(VisibleIf::Compare(_))
        ));
        assert_eq!(
            p,
            parse_profile(&serde_json::to_string(&p).unwrap()).unwrap()
        );
    }

    #[test]
    fn rgba_round_trip_is_stable() {
        for s in ["#112233", "#11223344", "#FFFFFF"] {
            assert_eq!(Rgba::parse(s).unwrap().to_string(), s);
        }
        assert_eq!(Rgba::parse("#112233FF").unwrap().to_string(), "#112233");
        assert!(Rgba::parse("#12345").is_none());
        assert!(Rgba::parse("112233").is_none());
    }

    #[test]
    fn defaults_fill_missing_fields() {
        let p = parse(&minimal()).unwrap();
        assert_eq!(p.anchor, Anchor::TopLeft);
        assert_eq!(p.offset, CellPoint { x: 1, y: 1 });
        assert_eq!(p.scale, 1.0);
        assert_eq!(p.panel.opacity, 0.35);
        assert_eq!(p.panel.padding, 1);
        let b = &p.blocks[0];
        assert_eq!(
            b.stat,
            Stat {
                op: StatOp::Current,
                window: 1,
                definition: LowDefinitionKey::Integral
            }
        );
        assert_eq!(b.style.value_style.font, "Segoe UI");
        assert_eq!(b.style.value_style.size, 12.0);
        assert_eq!(b.style.value_style.weight, 600);
        assert_eq!(b.style.graph.range_s, 60);
        assert_eq!(b.style.unit, UnitChoice::Auto);
        assert!(b.thresholds.is_empty() && b.visible_if.is_none() && b.panel.is_none());
        assert_eq!(
            LowDefinition::from(LowDefinitionKey::Percentile),
            LowDefinition::Percentile
        );
    }

    #[test]
    fn unknown_keys_rejected() {
        for v in [
            with(&["extra"], json!(1)),
            with(&["blocks", "0", "extra"], json!(1)),
            with(
                &["blocks", "0", "style"],
                json!({"valueStyle": {"bogus": 1}}),
            ),
        ] {
            assert!(matches!(parse(&v), Err(ProfileError::Json(_))), "{v}");
        }
    }

    #[test]
    fn limits_rejected() {
        let bad = |v: Value| parse(&v).unwrap_err();

        let mut v = minimal();
        let b = v["blocks"][0].clone();
        v["blocks"] = Value::Array(
            (0..257)
                .map(|i| {
                    let mut b = b.clone();
                    b["id"] = json!(format!("b{i}"));
                    b
                })
                .collect(),
        );
        assert_eq!(bad(v), ProfileError::TooManyBlocks(257));

        let v = with(
            &["blocks", "0", "source"],
            json!({"text": "x".repeat(65_537)}),
        );
        assert!(matches!(bad(v), ProfileError::TextTooLong { .. }));

        for s in [0.49, 3.01] {
            let v = with(&["scale"], json!(s));
            assert!(
                matches!(bad(v), ProfileError::OutOfRange { .. }),
                "scale {s}"
            );
        }
        for w in [0, 301] {
            let v = with(&["blocks", "0", "stat"], json!({"window": w}));
            assert!(
                matches!(bad(v), ProfileError::OutOfRange { .. }),
                "window {w}"
            );
        }
        for r in [4, 301] {
            let v = with(&["blocks", "0", "style"], json!({"graph": {"rangeS": r}}));
            assert!(
                matches!(bad(v), ProfileError::OutOfRange { .. }),
                "rangeS {r}"
            );
        }

        // 1e999 is not representable: serde_json rejects it while parsing.
        let json = minimal().to_string().replace(
            r#""kind":"text""#,
            r##""kind":"text","thresholds":[{"op":">","value":1e999,"color":"#FF0000"}]"##,
        );
        assert!(parse_profile(&json).is_err());
        // A non-finite value built in memory is caught by validate().
        let mut p = parse(&minimal()).unwrap();
        p.blocks[0].thresholds.push(Threshold {
            op: CompareOp::Gt,
            value: f64::NAN,
            color: Rgba::WHITE,
            target: ThresholdTarget::Value,
        });
        assert!(matches!(p.validate(), Err(ProfileError::NotFinite { .. })));
        p.blocks[0].thresholds.clear();
        p.scale = f64::INFINITY;
        assert!(matches!(p.validate(), Err(ProfileError::NotFinite { .. })));

        assert_eq!(bad(with(&["format"], json!(2))), ProfileError::Format(2));

        let mut v = minimal();
        let b = v["blocks"][0].clone();
        v["blocks"].as_array_mut().unwrap().push(b);
        assert_eq!(bad(v), ProfileError::DuplicateBlock("a".into()));

        let v = with(
            &["blocks", "0", "style"],
            json!({"valueStyle": {"color": "#12345"}}),
        );
        assert!(matches!(bad(v), ProfileError::Json(_)));

        let v = with(
            &["blocks", "0", "rect"],
            json!({"x": 0, "y": 0, "w": 0, "h": 1}),
        );
        assert!(matches!(bad(v), ProfileError::OutOfRange { .. }));
        let v = with(&["blocks", "0", "id"], json!(""));
        assert!(matches!(bad(v), ProfileError::OutOfRange { .. }));
    }

    #[test]
    fn oversized_input_rejected_before_parsing() {
        // Not valid JSON: if it were parsed the error would be Json.
        let s = "x".repeat(MAX_PROFILE_BYTES + 1);
        assert_eq!(parse_profile(&s), Err(ProfileError::TooLarge));
        assert!(matches!(
            parse_profile(&"x".repeat(MAX_PROFILE_BYTES)),
            Err(ProfileError::Json(_))
        ));
    }
}
