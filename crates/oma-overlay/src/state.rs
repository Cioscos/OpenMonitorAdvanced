//! What the overlay knows: the active profile, the latest data from the app
//! and the rings the statistics and the charts read. Pure: the window (C11)
//! and the renderer (C12) read it, the link feeds it through [`OverlayState::apply`].

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use oma_core::overlay::{
    parse_profile, FrameMetric, GraphMode, Kind, Profile, Source, StatOp, StatRing, VisibleIf,
};
use oma_ipc::overlay::{
    DrawSettings, FrameMetrics, OverlayMessage, PxArea, SensorInfo, WireFrameTime,
};

/// What an applied message changed, so the window knows what to redo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Changes {
    /// The profile changed: blocks, geometry and every cache.
    pub layout: bool,
    /// Some text may read differently.
    pub text: bool,
    /// Some chart has new data.
    pub charts: bool,
    /// The target area, its DPI or the visibility changed.
    pub placement: bool,
    /// The drawing options changed.
    pub settings: bool,
}

impl Changes {
    pub fn any(&self) -> bool {
        self.layout || self.text || self.charts || self.placement || self.settings
    }

    /// Adds the changes of a later message (a drained burst of messages).
    pub fn merge(&mut self, other: Changes) {
        self.layout |= other.layout;
        self.text |= other.text;
        self.charts |= other.charts;
        self.placement |= other.placement;
        self.settings |= other.settings;
    }
}

/// A source that can have a ring: a sensor or a numeric frame metric.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceKey {
    Sensor(String),
    Frames(FrameMetric),
}

impl SourceKey {
    /// The key of a source that has samples over time. Text, `bound` and the
    /// lows (computed by the app over their own window) have none.
    pub fn of(source: &Source) -> Option<Self> {
        match source {
            Source::Sensor(id) => Some(Self::Sensor(id.clone())),
            Source::Frames(FrameMetric::Low1 | FrameMetric::Low01 | FrameMetric::Bound) => None,
            Source::Frames(m) => Some(Self::Frames(*m)),
            Source::Text(_) => None,
        }
    }
}

/// The frame metrics that rings can follow, with their value in a message.
const RING_METRICS: [FrameMetric; 9] = [
    FrameMetric::FpsDisplayed,
    FrameMetric::FpsRendered,
    FrameMetric::FpsPresented,
    FrameMetric::FrametimeDisplayed,
    FrameMetric::FrametimeApp,
    FrameMetric::FgMultiplier,
    FrameMetric::Stutter,
    FrameMetric::LatencyPc,
    FrameMetric::LatencyDisplay,
];

/// The numeric value of `metric` in `m`; `None` for the lows and `bound`.
pub fn metric_value(m: &FrameMetrics, metric: FrameMetric) -> Option<f64> {
    match metric {
        FrameMetric::FpsDisplayed => m.fps_displayed,
        FrameMetric::FpsRendered => m.fps_rendered,
        FrameMetric::FpsPresented => m.fps_presented,
        FrameMetric::FrametimeDisplayed => m.frametime_displayed_ms,
        FrameMetric::FrametimeApp => m.frametime_app_ms,
        FrameMetric::FgMultiplier => m.fg_multiplier,
        FrameMetric::Stutter => m.stutter_count.map(f64::from),
        FrameMetric::LatencyPc => m.latency_pc_ms,
        FrameMetric::LatencyDisplay => m.latency_display_ms,
        FrameMetric::Low1 | FrameMetric::Low01 | FrameMetric::Bound => None,
    }
}

fn is_frametime_graph(kind: Kind, mode: GraphMode) -> bool {
    kind == Kind::Graph && mode == GraphMode::Frametime
}

/// What a profile reads from each source.
#[derive(Debug, Default)]
struct SourcePlan {
    /// The rings: for each source, the longest window asked by a statistic
    /// other than `current` or by a chart.
    windows: HashMap<SourceKey, u32>,
    /// The sources a chart (`graph`, `sparkline`) reads.
    charted: HashSet<SourceKey>,
    /// The sources a block redrawn at `textHz` reads (`text`, `meter`,
    /// `gauge`), or a `visibleIf` compares.
    texted: HashSet<SourceKey>,
}

fn source_plan(profile: &Profile) -> SourcePlan {
    let mut windows: HashMap<SourceKey, u32> = HashMap::new();
    let mut charted = HashSet::new();
    let mut texted = HashSet::new();
    let mut need = |source: &Source, window: u32| {
        if let Some(key) = SourceKey::of(source) {
            let w = windows.entry(key).or_insert(0);
            *w = (*w).max(window);
        }
    };
    for b in &profile.blocks {
        let is_low = matches!(
            b.source,
            Source::Frames(FrameMetric::Low1 | FrameMetric::Low01)
        );
        if b.stat.op != StatOp::Current && !is_low {
            need(&b.source, b.stat.window);
        }
        let charts = matches!(b.kind, Kind::Graph | Kind::Sparkline)
            && !is_frametime_graph(b.kind, b.style.graph.mode);
        if charts {
            need(&b.source, b.style.graph.range_s);
        }
        if let Some(key) = SourceKey::of(&b.source) {
            if charts {
                charted.insert(key);
            } else if !is_frametime_graph(b.kind, b.style.graph.mode) {
                texted.insert(key);
            }
        }
        if let Some(VisibleIf::Compare(c)) = &b.visible_if {
            if c.stat.op != StatOp::Current {
                need(&c.source, c.stat.window);
            }
            texted.extend(SourceKey::of(&c.source));
        }
    }
    SourcePlan {
        windows,
        charted,
        texted,
    }
}

/// Most frames kept per second of frametime chart range: a frame rate no
/// display reaches, so the cap only bites on timestamps that stop advancing.
pub const MAX_FRAMES_PER_S: usize = 1000;

/// The longest `range_s` of the frametime charts; 0 without one.
fn frame_range_of(profile: &Profile) -> u32 {
    profile
        .blocks
        .iter()
        .filter(|b| is_frametime_graph(b.kind, b.style.graph.mode))
        .map(|b| b.style.graph.range_s)
        .max()
        .unwrap_or(0)
}

/// The drawing options before the first `SetProfile`: the settings defaults.
pub fn default_draw() -> DrawSettings {
    DrawSettings {
        chart_fps: 30,
        text_hz: 2,
        hide_from_capture: false,
        attach: "window".into(),
        decimal_comma: false,
        temperature_unit: "celsius".into(),
        throughput_unit: "bytes".into(),
    }
}

#[derive(Debug)]
pub struct OverlayState {
    /// The active profile; `None` until the first valid `SetProfile`.
    pub profile: Option<Profile>,
    pub profile_id: String,
    /// The profile's sensors (translated label and unit), by id.
    pub sensors: HashMap<String, SensorInfo>,
    /// The translated strings for the overlay.
    pub strings: BTreeMap<String, String>,
    /// One ring per source with a statistic other than `current` or a chart.
    pub rings: HashMap<SourceKey, StatRing>,
    /// The window of each ring, the charted and the texted sources.
    plan: SourcePlan,
    /// The frames of the frametime charts, oldest first.
    pub frame_times: VecDeque<WireFrameTime>,
    /// The longest `range_s` of the frametime charts; 0 keeps no frames.
    frame_range_s: u32,
    /// The last frame metrics.
    pub metrics: Option<FrameMetrics>,
    /// The last value and quality of each sensor.
    pub values: HashMap<String, (Option<f64>, String)>,
    /// The target client area and its DPI; `None` hides the overlay.
    pub placement: Option<(PxArea, u32)>,
    pub draw: DrawSettings,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            profile: None,
            profile_id: String::new(),
            sensors: HashMap::new(),
            strings: BTreeMap::new(),
            rings: HashMap::new(),
            plan: SourcePlan::default(),
            frame_times: VecDeque::new(),
            frame_range_s: 0,
            metrics: None,
            values: HashMap::new(),
            placement: None,
            draw: default_draw(),
        }
    }
}

impl OverlayState {
    /// The window of the ring of `key`, if the profile asks for one.
    #[cfg(test)]
    pub fn ring_window(&self, key: &SourceKey) -> Option<u32> {
        self.plan.windows.get(key).copied()
    }

    /// Applies one validated message received at `now_s` seconds (a monotonic
    /// clock of the overlay) and says what it changed.
    pub fn apply(&mut self, msg: OverlayMessage, now_s: f64) -> Changes {
        match msg {
            // The link answers the handshake; nothing to draw.
            OverlayMessage::Hello(_) => Changes::default(),
            OverlayMessage::SetProfile(p) => {
                let profile = match parse_profile(&p.profile_json) {
                    Ok(profile) => profile,
                    Err(e) => {
                        tracing::warn!(
                            profile_id = %p.profile_id,
                            error = %e,
                            "invalid profile from the app; keeping the previous one"
                        );
                        return Changes::default();
                    }
                };
                self.set_profile(profile);
                self.profile_id = p.profile_id;
                self.sensors = p.sensors.into_iter().map(|s| (s.id.clone(), s)).collect();
                // Values of sensors the new profile does not use are of no use.
                let sensors = &self.sensors;
                self.values.retain(|id, _| sensors.contains_key(id));
                self.strings = p.strings;
                self.draw = p.draw;
                Changes {
                    layout: true,
                    text: true,
                    charts: true,
                    placement: false,
                    settings: true,
                }
            }
            OverlayMessage::SetPlacement(p) => {
                let placement = p.area.map(|area| (area, p.dpi));
                let changed = placement != self.placement;
                self.placement = placement;
                Changes {
                    placement: changed,
                    ..Changes::default()
                }
            }
            OverlayMessage::Values(v) => {
                let (mut text, mut charts) = (false, false);
                // Only the profile's sensors are kept, so the map stays bounded.
                for w in v
                    .values
                    .into_iter()
                    .filter(|w| self.sensors.contains_key(&w.id))
                {
                    let key = SourceKey::Sensor(w.id.clone());
                    if let Some(ring) = self.rings.get_mut(&key) {
                        ring.push(now_s, w.value);
                    }
                    text |= self.plan.texted.contains(&key);
                    charts |= self.plan.charted.contains(&key);
                    self.values.insert(w.id, (w.value, w.quality));
                }
                Changes {
                    text,
                    charts,
                    ..Changes::default()
                }
            }
            OverlayMessage::FrameMetrics(m) => {
                let mut charts = false;
                for metric in RING_METRICS {
                    let key = SourceKey::Frames(metric);
                    if let Some(ring) = self.rings.get_mut(&key) {
                        ring.push(now_s, metric_value(&m, metric));
                        charts |= self.plan.charted.contains(&key);
                    }
                }
                self.metrics = Some(m);
                Changes {
                    text: true,
                    charts,
                    ..Changes::default()
                }
            }
            OverlayMessage::FrameTimes(f) => {
                if self.frame_range_s == 0 || f.frames.is_empty() {
                    return Changes::default();
                }
                self.frame_times.extend(f.frames);
                self.trim_frame_times();
                Changes {
                    charts: true,
                    ..Changes::default()
                }
            }
        }
    }

    /// Makes `profile` the active one and fits the rings to it. A ring whose
    /// window does not change keeps its samples, so re-sending the same
    /// profile (new settings) does not empty the charts.
    fn set_profile(&mut self, profile: Profile) {
        let plan = source_plan(&profile);
        let mut old = std::mem::take(&mut self.rings);
        self.rings = plan
            .windows
            .iter()
            .map(|(key, &window)| {
                let ring = match old.remove(key) {
                    Some(ring) if self.plan.windows.get(key) == Some(&window) => ring,
                    _ => StatRing::new(window),
                };
                (key.clone(), ring)
            })
            .collect();
        self.plan = plan;
        self.frame_range_s = frame_range_of(&profile);
        self.trim_frame_times();
        self.profile = Some(profile);
    }

    /// Keeps the frames of the last `frame_range_s` seconds before the newest
    /// one (inclusive), and at most `frame_range_s × MAX_FRAMES_PER_S` of them
    /// in case the timestamps stop advancing; none without a frametime chart.
    /// Frames arrive in order, so the newest is the last.
    fn trim_frame_times(&mut self) {
        if self.frame_range_s == 0 {
            self.frame_times.clear();
            return;
        }
        let Some(newest) = self.frame_times.back().map(|f| f.t_s) else {
            return;
        };
        let oldest = newest - f64::from(self.frame_range_s);
        while self.frame_times.front().is_some_and(|f| f.t_s < oldest) {
            self.frame_times.pop_front();
        }
        let cap = self.frame_range_s as usize * MAX_FRAMES_PER_S;
        let excess = self.frame_times.len().saturating_sub(cap);
        self.frame_times.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::overlay::Stat;
    use oma_ipc::overlay::{
        FrameTimes, SetPlacement, SetProfile, Values, WireValue, OVERLAY_PROTOCOL_VERSION,
    };
    use serde_json::json;

    const CPU: &str = "cpu/temperature/package";
    const GPU: &str = "gpu0/load/core";

    fn set_profile(profile: serde_json::Value) -> OverlayMessage {
        OverlayMessage::SetProfile(SetProfile {
            profile_id: "p".into(),
            profile_json: profile.to_string(),
            sensors: vec![
                SensorInfo {
                    id: CPU.into(),
                    label: "CPU".into(),
                    unit: "celsius".into(),
                },
                SensorInfo {
                    id: GPU.into(),
                    label: "GPU".into(),
                    unit: "percent".into(),
                },
            ],
            strings: BTreeMap::from([("sensorAbsent".to_owned(), "n/a".to_owned())]),
            draw: DrawSettings {
                chart_fps: 60,
                text_hz: 4,
                ..default_draw()
            },
        })
    }

    fn profile(blocks: serde_json::Value) -> serde_json::Value {
        json!({ "format": 1, "name": "test", "blocks": blocks })
    }

    fn rect() -> serde_json::Value {
        json!({ "x": 0, "y": 0, "w": 10, "h": 2 })
    }

    fn text_block(
        id: &str,
        source: serde_json::Value,
        stat: serde_json::Value,
    ) -> serde_json::Value {
        json!({ "id": id, "rect": rect(), "source": source, "kind": "text", "stat": stat })
    }

    fn graph_block(
        id: &str,
        source: serde_json::Value,
        mode: &str,
        range_s: u32,
    ) -> serde_json::Value {
        json!({
            "id": id, "rect": rect(), "source": source, "kind": "graph",
            "style": { "graph": { "mode": mode, "rangeS": range_s } }
        })
    }

    fn values(at_ms: u64, id: &str, v: Option<f64>) -> OverlayMessage {
        OverlayMessage::Values(Values {
            at_ms,
            values: vec![WireValue {
                id: id.into(),
                value: v,
                quality: "fresh".into(),
            }],
        })
    }

    fn frames(ts: &[f64]) -> OverlayMessage {
        OverlayMessage::FrameTimes(FrameTimes {
            frames: ts
                .iter()
                .map(|&t_s| WireFrameTime {
                    t_s,
                    displayed_ms: Some(16.6),
                    app_ms: None,
                })
                .collect(),
        })
    }

    fn avg(window: u32) -> Stat {
        Stat {
            op: StatOp::Avg,
            window,
            ..Stat::default()
        }
    }

    #[test]
    fn set_profile_parses_and_replaces_rings() {
        let mut s = OverlayState::default();
        let ch = s.apply(
            set_profile(profile(json!([
                text_block(
                    "a",
                    json!({ "sensor": CPU }),
                    json!({ "op": "avg", "window": 5 })
                ),
                text_block(
                    "b",
                    json!({ "sensor": CPU }),
                    json!({ "op": "max", "window": 20 })
                ),
                text_block("c", json!({ "sensor": GPU }), json!({})),
                graph_block("d", json!({ "frames": "fps-displayed" }), "line", 45),
                text_block(
                    "e",
                    json!({ "frames": "low-1" }),
                    json!({ "op": "avg", "window": 30 })
                ),
            ]))),
            0.0,
        );
        assert!(ch.layout && ch.text && ch.charts && ch.settings);
        assert_eq!(s.profile.as_ref().map(|p| p.blocks.len()), Some(5));
        assert_eq!(s.profile_id, "p");
        assert_eq!(s.sensors[CPU].label, "CPU");
        assert_eq!(s.strings["sensorAbsent"], "n/a");
        assert_eq!(s.draw.chart_fps, 60);
        // The longest window per source; `current` and the lows get no ring.
        assert_eq!(s.ring_window(&SourceKey::Sensor(CPU.into())), Some(20));
        assert_eq!(
            s.ring_window(&SourceKey::Frames(FrameMetric::FpsDisplayed)),
            Some(45)
        );
        assert_eq!(s.ring_window(&SourceKey::Sensor(GPU.into())), None);
        assert_eq!(s.ring_window(&SourceKey::Frames(FrameMetric::Low1)), None);
        assert_eq!(s.rings.len(), 2);

        // A new profile replaces the rings: the old sources go away.
        s.apply(
            set_profile(profile(json!([text_block(
                "x",
                json!({ "sensor": GPU }),
                json!({ "op": "min", "window": 3 })
            )]))),
            1.0,
        );
        assert_eq!(s.rings.len(), 1);
        assert_eq!(s.ring_window(&SourceKey::Sensor(GPU.into())), Some(3));
        assert!(!s.rings.contains_key(&SourceKey::Sensor(CPU.into())));
    }

    #[test]
    fn invalid_profile_json_keeps_previous() {
        let mut s = OverlayState::default();
        s.apply(
            set_profile(profile(json!([text_block(
                "a",
                json!({ "sensor": CPU }),
                json!({ "op": "avg", "window": 5 })
            )]))),
            0.0,
        );
        let before = s.profile.clone();
        assert!(before.is_some());

        let mut bad = set_profile(json!({}));
        if let OverlayMessage::SetProfile(p) = &mut bad {
            p.profile_json = "{ not json".into();
            p.profile_id = "q".into();
        }
        let ch = s.apply(bad, 1.0);
        assert_eq!(ch, Changes::default());
        assert_eq!(s.profile, before);
        assert_eq!(s.profile_id, "p");
        assert_eq!(s.ring_window(&SourceKey::Sensor(CPU.into())), Some(5));

        // Valid JSON that breaks a limit is refused the same way.
        let ch = s.apply(set_profile(json!({ "format": 2, "name": "x" })), 2.0);
        assert_eq!(ch, Changes::default());
        assert_eq!(s.profile, before);
    }

    #[test]
    fn values_feed_rings_and_mark_text_dirty() {
        let mut s = OverlayState::default();
        s.apply(
            set_profile(profile(json!([
                text_block(
                    "a",
                    json!({ "sensor": CPU }),
                    json!({ "op": "avg", "window": 5 })
                ),
                graph_block("g", json!({ "sensor": GPU }), "line", 60),
            ]))),
            0.0,
        );
        let ch = s.apply(values(1000, CPU, Some(40.0)), 1.0);
        assert!(ch.text);
        assert!(!ch.charts, "CPU feeds no chart");
        s.apply(values(2000, CPU, Some(50.0)), 2.0);
        assert_eq!(s.values[CPU], (Some(50.0), "fresh".to_owned()));
        let ring = &s.rings[&SourceKey::Sensor(CPU.into())];
        assert_eq!(ring.value(&avg(5)), Some(45.0));

        // GPU feeds only a chart: the charts are dirty, the texts are not.
        let ch = s.apply(values(3000, GPU, Some(10.0)), 3.0);
        assert!(ch.charts && !ch.text);

        // An absent value is stored as absent and leaves the ring alone.
        s.apply(values(4000, CPU, None), 4.0);
        assert_eq!(s.values[CPU].0, None);
        let ring = &s.rings[&SourceKey::Sensor(CPU.into())];
        assert_eq!(ring.value(&avg(5)), Some(45.0));

        // Frame metrics feed the frame rings too.
        s.apply(
            set_profile(profile(json!([text_block(
                "f",
                json!({ "frames": "fps-displayed" }),
                json!({ "op": "max", "window": 10 })
            )]))),
            5.0,
        );
        let mut m = sample_metrics();
        m.fps_displayed = Some(90.0);
        let ch = s.apply(OverlayMessage::FrameMetrics(m.clone()), 6.0);
        assert!(ch.text);
        m.fps_displayed = Some(60.0);
        s.apply(OverlayMessage::FrameMetrics(m), 7.0);
        let ring = &s.rings[&SourceKey::Frames(FrameMetric::FpsDisplayed)];
        let max = Stat {
            op: StatOp::Max,
            window: 10,
            ..Stat::default()
        };
        assert_eq!(ring.value(&max), Some(90.0));
        assert_eq!(s.metrics.as_ref().and_then(|m| m.fps_displayed), Some(60.0));
    }

    fn sample_metrics() -> FrameMetrics {
        FrameMetrics {
            state: "running".into(),
            fps_displayed: None,
            fps_rendered: None,
            fps_presented: None,
            rendered_source: None,
            fg_suspected: false,
            frametime_displayed_ms: None,
            frametime_app_ms: None,
            fg_multiplier: None,
            stutter_count: None,
            stutter_percent: None,
            latency_pc_ms: None,
            latency_display_ms: None,
            bound: None,
            lows: vec![],
        }
    }

    #[test]
    fn frame_times_append_and_trim_to_largest_graph_range() {
        let mut s = OverlayState::default();
        // Without a frametime chart no frame is kept.
        s.apply(set_profile(profile(json!([]))), 0.0);
        let ch = s.apply(frames(&[1.0, 2.0]), 0.0);
        assert!(!ch.charts);
        assert!(s.frame_times.is_empty());

        s.apply(
            set_profile(profile(json!([
                graph_block(
                    "a",
                    json!({ "frames": "frametime-displayed" }),
                    "frametime",
                    10
                ),
                graph_block(
                    "b",
                    json!({ "frames": "frametime-displayed" }),
                    "frametime",
                    20
                ),
            ]))),
            0.0,
        );
        let ch = s.apply(frames(&[1.0, 5.0, 10.0]), 0.0);
        assert!(ch.charts);
        assert_eq!(s.frame_times.len(), 3);
        // Trimmed to 20 s before the newest frame (inclusive).
        s.apply(frames(&[21.0, 25.0]), 0.0);
        let ts: Vec<f64> = s.frame_times.iter().map(|f| f.t_s).collect();
        assert_eq!(ts, vec![5.0, 10.0, 21.0, 25.0]);
        // An empty batch changes nothing.
        assert!(!s.apply(frames(&[]), 0.0).charts);

        // A shorter range on the next profile trims at once.
        s.apply(
            set_profile(profile(json!([graph_block(
                "a",
                json!({ "frames": "frametime-displayed" }),
                "frametime",
                5
            )]))),
            0.0,
        );
        let ts: Vec<f64> = s.frame_times.iter().map(|f| f.t_s).collect();
        assert_eq!(ts, vec![21.0, 25.0]);
    }

    #[test]
    fn values_ignore_unknown_ids() {
        let mut s = OverlayState::default();
        s.apply(
            set_profile(profile(json!([text_block(
                "a",
                json!({ "sensor": CPU }),
                json!({})
            )]))),
            0.0,
        );
        let ch = s.apply(values(1000, "disk0/temperature/drive", Some(30.0)), 1.0);
        assert!(!ch.any());
        assert!(s.values.is_empty());
        let ch = s.apply(values(2000, CPU, Some(40.0)), 2.0);
        assert!(ch.text);
        assert_eq!(s.values.len(), 1);
    }

    #[test]
    fn frame_times_are_capped_without_advancing_time() {
        let mut s = OverlayState::default();
        s.apply(
            set_profile(profile(json!([graph_block(
                "a",
                json!({ "frames": "frametime-displayed" }),
                "frametime",
                5
            )]))),
            0.0,
        );
        let cap = 5 * MAX_FRAMES_PER_S;
        let batch = vec![1.0; 4096];
        for _ in 0..(cap / 4096 + 2) {
            s.apply(frames(&batch), 0.0);
        }
        assert_eq!(s.frame_times.len(), cap);
    }

    #[test]
    fn placement_none_hides() {
        let mut s = OverlayState::default();
        let area = PxArea {
            x: 10,
            y: 20,
            width: 1920,
            height: 1080,
        };
        let ch = s.apply(
            OverlayMessage::SetPlacement(SetPlacement {
                area: Some(area),
                dpi: 144,
            }),
            0.0,
        );
        assert!(ch.placement);
        assert_eq!(s.placement, Some((area, 144)));
        // The same placement again changes nothing.
        let ch = s.apply(
            OverlayMessage::SetPlacement(SetPlacement {
                area: Some(area),
                dpi: 144,
            }),
            0.0,
        );
        assert!(!ch.any());
        let ch = s.apply(
            OverlayMessage::SetPlacement(SetPlacement {
                area: None,
                dpi: 144,
            }),
            0.0,
        );
        assert!(ch.placement);
        assert_eq!(s.placement, None);
        // A Hello is the link's business: no change.
        let ch = s.apply(
            OverlayMessage::Hello(oma_ipc::overlay::OverlayHello {
                protocol_version: OVERLAY_PROTOCOL_VERSION,
                version: "0.5.0".into(),
            }),
            0.0,
        );
        assert!(!ch.any());
    }
}
