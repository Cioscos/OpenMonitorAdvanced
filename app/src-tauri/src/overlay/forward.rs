//! The messages the app sends to `oma-overlay.exe`, built by pure functions
//! from the profile, the schema, the sampler's output and the frame readout.

use std::collections::{BTreeMap, BTreeSet};

use oma_core::frames::metrics::{Bottleneck, LowDefinition};
use oma_core::frames::{FrameReadout, FrameWindow, Rendered, LOWS_WINDOW_S};
use oma_core::model::{Schema, Snapshot};
use oma_core::overlay::{FrameMetric, Profile, Source, Stat, VisibleIf};
use oma_core::provider::Quality;
use oma_core::settings::Settings;
use oma_ipc::overlay::{
    DrawSettings, FrameMetrics, FrameTimes, OverlayMessage, SensorInfo, SetProfile, Values,
    WireFrameTime, WireLow, WireValue, MAX_FRAME_TIMES, MAX_LOWS, MAX_OVERLAY_VALUES,
    MAX_SENSOR_INFOS,
};

use crate::i18n::{sensor_label, t, Lang};

/// Every frame metric, in profile order; `metric.<name>` is sent for each.
pub const ALL_METRICS: [FrameMetric; 12] = [
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
];

/// The sensor ids the profile reads, from blocks and `visibleIf`, unique and
/// sorted.
pub fn used_sensors(profile: &Profile) -> Vec<String> {
    let ids: BTreeSet<&String> = sources(profile)
        .filter_map(|(source, _)| match source {
            Source::Sensor(id) => Some(id),
            _ => None,
        })
        .collect();
    ids.into_iter().cloned().collect()
}

/// Every source the profile reads, with its stat: each block's, then its
/// `visibleIf` comparison's.
fn sources(profile: &Profile) -> impl Iterator<Item = (&Source, &Stat)> {
    profile.blocks.iter().flat_map(|b| {
        let condition = match &b.visible_if {
            Some(VisibleIf::Compare(c)) => Some((&c.source, &c.stat)),
            _ => None,
        };
        std::iter::once((&b.source, &b.stat)).chain(condition)
    })
}

/// The `(window, definition)` pairs of the profile's `low-*` sources for
/// [`oma_core::frames::read`], unique, without the `(10, Integral)` entry it
/// always computes.
pub fn low_windows(profile: &Profile) -> Vec<(u32, LowDefinition)> {
    let always = (LOWS_WINDOW_S as u32, LowDefinition::Integral);
    let mut out: Vec<(u32, LowDefinition)> = Vec::new();
    for (source, stat) in sources(profile) {
        if !matches!(
            source,
            Source::Frames(FrameMetric::Low1 | FrameMetric::Low01)
        ) {
            continue;
        }
        let pair = (stat.window, LowDefinition::from(stat.definition));
        // `read` adds the `always` entry: the readout stays within MAX_LOWS.
        if pair != always && !out.contains(&pair) && out.len() < MAX_LOWS - 1 {
            out.push(pair);
        }
    }
    out
}

/// The sensors and lows windows of several profiles together (the active
/// one and the preview): sensors unique and sorted, lows unique and within
/// the limit [`low_windows`] keeps.
pub fn union_needs<'a>(
    profiles: impl IntoIterator<Item = &'a Profile>,
) -> (Vec<String>, Vec<(u32, LowDefinition)>) {
    let mut used = BTreeSet::new();
    let mut lows: Vec<(u32, LowDefinition)> = Vec::new();
    for p in profiles {
        used.extend(used_sensors(p));
        for pair in low_windows(p) {
            if !lows.contains(&pair) && lows.len() < MAX_LOWS - 1 {
                lows.push(pair);
            }
        }
    }
    (used.into_iter().collect(), lows)
}

/// Translated label and unit of each `used` sensor present in `schema`.
pub fn sensor_infos(schema: &Schema, used: &[String], lang: Lang) -> Vec<SensorInfo> {
    used.iter()
        .filter_map(|id| schema.sensors.iter().find(|s| &s.id == id))
        .take(MAX_SENSOR_INFOS)
        .map(|s| SensorInfo {
            id: s.id.clone(),
            label: sensor_label(lang, &s.label),
            unit: serde_name(s.unit),
        })
        .collect()
}

/// The serde spelling of a unit-like enum (`bytes_per_second`, `low-01`...).
fn serde_name<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The translated texts the overlay draws.
///
/// The keys are those `oma-overlay`'s `render::layout` looks up. The low
/// suffixes are trimmed: the overlay joins them to the metric name with a
/// space of its own.
pub fn overlay_strings(lang: Lang) -> BTreeMap<String, String> {
    let text = |key: &str| t(lang, &format!("overlay.text.{key}"), &[]);
    let mut out = BTreeMap::new();
    for key in [
        "sensorAbsent",
        "fgSuspected",
        "bound.gpu",
        "bound.cpu",
        "previewTitle",
    ] {
        out.insert(key.to_owned(), text(key));
    }
    for key in ["low.integral", "low.percentile"] {
        out.insert(key.to_owned(), text(key).trim().to_owned());
    }
    for key in ["flag.on", "flag.off"] {
        out.insert(key.to_owned(), t(lang, key, &[]));
    }
    for m in ALL_METRICS {
        let key = format!("metric.{}", serde_name(m));
        let value = text(&key);
        out.insert(key, value);
    }
    out
}

/// `SetProfile` for the profile `id`.
pub fn set_profile(
    id: &str,
    profile: &Profile,
    schema: &Schema,
    settings: &Settings,
    lang: Lang,
) -> OverlayMessage {
    let general = &settings.general;
    let overlay = &settings.overlay;
    OverlayMessage::SetProfile(SetProfile {
        profile_id: id.to_owned(),
        // A validated profile always serializes; the overlay re-parses it.
        profile_json: serde_json::to_string(profile).unwrap_or_default(),
        sensors: sensor_infos(schema, &used_sensors(profile), lang),
        strings: overlay_strings(lang),
        draw: DrawSettings {
            chart_fps: overlay.chart_fps.as_u32(),
            text_hz: overlay.text_hz,
            hide_from_capture: overlay.hide_from_capture,
            attach: overlay.attach.as_str().to_owned(),
            decimal_comma: lang == Lang::It,
            temperature_unit: general.temperature_unit.as_str().to_owned(),
            throughput_unit: general.throughput_unit.as_str().to_owned(),
        },
    })
}

/// `Values` with the `used` sensors of one sampler tick.
pub fn values_message(
    schema: &Schema,
    snapshot: &Snapshot,
    quality: &[Quality],
    used: &[String],
    at_ms: u64,
) -> OverlayMessage {
    let values = used
        .iter()
        .filter_map(|id| {
            let i = schema.sensors.iter().position(|s| &s.id == id)?;
            let quality = match quality.get(i).copied().unwrap_or(Quality::Fresh) {
                Quality::Fresh => "fresh",
                Quality::Held => "held",
                Quality::Suspended => "suspended",
            };
            Some(WireValue {
                id: id.clone(),
                value: finite(snapshot.values.get(i).copied().flatten()),
                quality: quality.to_owned(),
            })
        })
        .take(MAX_OVERLAY_VALUES)
        .collect();
    OverlayMessage::Values(Values { at_ms, values })
}

/// `v` without NaN and infinities, which the overlay would reject.
fn finite(v: Option<f64>) -> Option<f64> {
    v.filter(|v| v.is_finite())
}

/// `FrameMetrics` from `readout`; `state` is the frames state, or
/// `unavailable` without the service.
pub fn metrics_message(readout: Option<&FrameReadout>, state: &str) -> OverlayMessage {
    let Some(r) = readout else {
        return OverlayMessage::FrameMetrics(FrameMetrics {
            state: state.to_owned(),
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
            lows: Vec::new(),
        });
    };
    let lows = r
        .lows
        .iter()
        .take(MAX_LOWS)
        .map(|l| WireLow {
            window_s: l.window_s,
            definition: wire_definition(l.definition).to_owned(),
            one_percent: finite(l.lows.map(|x| x.one_percent)),
            point_one_percent: finite(l.lows.map(|x| x.point_one_percent)),
        })
        .collect();
    let bound = r.bottleneck.map(|b| match b {
        Bottleneck::Gpu => "gpu",
        Bottleneck::Cpu => "cpu",
        Bottleneck::Unknown => "unknown",
    });
    OverlayMessage::FrameMetrics(FrameMetrics {
        state: state.to_owned(),
        fps_displayed: finite(r.fps_displayed),
        fps_rendered: match r.rendered {
            Rendered::Fps { fps, .. } => finite(Some(fps)),
            Rendered::FgSuspected | Rendered::Unavailable => None,
        },
        fps_presented: finite(r.fps_presented),
        rendered_source: r.rendered_source.map(str::to_owned),
        fg_suspected: r.fg_suspected,
        frametime_displayed_ms: finite(r.frametime_displayed_ms),
        frametime_app_ms: finite(r.frametime_app_ms),
        fg_multiplier: finite(r.fg_multiplier),
        stutter_count: r.stutter.map(|s| s.count),
        stutter_percent: finite(r.stutter.map(|s| s.time_percent)),
        latency_pc_ms: finite(r.latency_pc_ms),
        latency_display_ms: finite(r.latency_display_ms),
        bound: bound.map(str::to_owned),
        lows,
    })
}

/// The protocol spelling of a lows definition.
fn wire_definition(d: LowDefinition) -> &'static str {
    match d {
        LowDefinition::Integral => "integral",
        LowDefinition::Percentile => "percentile",
    }
}

/// Keeps in `m` only the lows of `lows` and the `(10, Integral)` entry
/// [`oma_core::frames::read`] always computes.
pub fn keep_lows(m: &mut FrameMetrics, lows: &[(u32, LowDefinition)]) {
    let always = (LOWS_WINDOW_S as u32, LowDefinition::Integral);
    m.lows.retain(|l| {
        std::iter::once(&always)
            .chain(lows)
            .any(|&(w, d)| w == l.window_s && wire_definition(d) == l.definition)
    });
}

/// `FrameTimes` with the frames of `swapchain` newer than `after_t_s`, and the
/// time of the newest one sent (`after_t_s` when there is none).
pub fn frame_times_since(
    window: &FrameWindow,
    swapchain: Option<u64>,
    after_t_s: f64,
) -> (OverlayMessage, f64) {
    let Some(swapchain) = swapchain else {
        return (
            OverlayMessage::FrameTimes(FrameTimes { frames: Vec::new() }),
            after_t_s,
        );
    };
    let mut frames: Vec<WireFrameTime> = window
        .since(after_t_s)
        .into_iter()
        .filter(|f| f.swapchain == swapchain && f.t_s.is_finite())
        .map(|f| WireFrameTime {
            t_s: f.t_s,
            displayed_ms: finite(f.ms_between_display_change.filter(|_| f.displayed)),
            app_ms: finite(f.ms_app_frametime),
        })
        .collect();
    let newest = frames.last().map_or(after_t_s, |f| f.t_s);
    // The chart only needs the newest frames; older ones would scroll out.
    let excess = frames.len().saturating_sub(MAX_FRAME_TIMES);
    frames.drain(..excess);
    (OverlayMessage::FrameTimes(FrameTimes { frames }), newest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::frames::metrics::{Bottleneck, Lows, Stutter};
    use oma_core::frames::{FrameKind, FrameSample, LowReadout, Rendered, RenderedSource};
    use oma_core::model::{Label, Sensor, SensorKind, Source as SensorSource, Unit};
    use oma_core::overlay::parse_profile;
    use oma_core::settings::{Attach, ChartFps, TemperatureUnit, ThroughputUnit};
    use oma_ipc::overlay::{FrameMetrics, FrameTimes, Values, WireLow, WireValue};
    use serde_json::json;

    use crate::i18n::{t, RUST_KEYS};

    /// The keys `oma-overlay`'s `render::layout` looks up in
    /// `SetProfile.strings` (`string(state, ..)`), besides `metric.<m>`.
    const OVERLAY_READS: &[&str] = &[
        "sensorAbsent",
        "fgSuspected",
        "bound.gpu",
        "bound.cpu",
        "flag.on",
        "flag.off",
        "low.integral",
        "low.percentile",
        // The preview window's title (`--preview`).
        "previewTitle",
    ];

    fn profile(blocks: serde_json::Value) -> Profile {
        parse_profile(&json!({ "format": 1, "name": "t", "blocks": blocks }).to_string()).unwrap()
    }

    fn block(id: &str, source: serde_json::Value) -> serde_json::Value {
        json!({ "id": id, "rect": { "x": 0, "y": 0, "w": 4, "h": 2 }, "source": source, "kind": "text" })
    }

    fn schema() -> Schema {
        Schema {
            revision: 3,
            devices: vec![],
            sensors: vec![
                Sensor::new(
                    "cpu/0",
                    SensorKind::Load,
                    "total",
                    Unit::Percent,
                    Label::new("cpu.load.total"),
                    SensorSource::Mock,
                ),
                Sensor::new(
                    "gpu0",
                    SensorKind::Throughput,
                    "pcie-rx",
                    Unit::BytesPerSecond,
                    Label::new("cpu.load.total"),
                    SensorSource::Mock,
                ),
                Sensor::new(
                    "gpu0",
                    SensorKind::Temperature,
                    "core",
                    Unit::Celsius,
                    Label::new("cpu.load.total"),
                    SensorSource::Mock,
                ),
            ],
        }
    }

    #[test]
    fn used_sensors_include_visible_if_sources() {
        let mut b = block("b", json!({ "sensor": "gpu0/temperature/core" }));
        b["visibleIf"] =
            json!({ "source": { "sensor": "cpu/0/load/total" }, "op": ">", "value": 50 });
        let p = profile(json!([
            b,
            block("a", json!({ "sensor": "gpu0/temperature/core" })),
            block("c", json!({ "frames": "fps-displayed" })),
            block("d", json!({ "text": "hello" })),
        ]));
        assert_eq!(
            used_sensors(&p),
            vec!["cpu/0/load/total", "gpu0/temperature/core"]
        );
    }

    #[test]
    fn low_windows_come_from_low_blocks() {
        let mut a = block("a", json!({ "frames": "low-1" }));
        a["stat"] = json!({ "window": 30, "definition": "percentile" });
        let mut b = block("b", json!({ "frames": "low-01" }));
        b["stat"] = json!({ "window": 30, "definition": "percentile" });
        let mut c = block("c", json!({ "frames": "low-1" }));
        c["stat"] = json!({ "window": 10 });
        let mut d = block("d", json!({ "frames": "fps-displayed" }));
        d["stat"] = json!({ "op": "avg", "window": 60 });
        let mut e = block("e", json!({ "text": "x" }));
        e["visibleIf"] = json!({ "source": { "frames": "low-01" }, "stat": { "window": 300 }, "op": "<", "value": 30 });
        let p = profile(json!([a, b, c, d, e]));
        assert_eq!(
            low_windows(&p),
            vec![
                (30, LowDefinition::Percentile),
                (300, LowDefinition::Integral)
            ]
        );
    }

    #[test]
    fn sensor_infos_carry_label_and_unit() {
        let used = vec![
            "cpu/0/load/total".to_owned(),
            "missing/load/x".to_owned(),
            "gpu0/throughput/pcie-rx".to_owned(),
        ];
        let infos = sensor_infos(&schema(), &used, Lang::It);
        assert_eq!(
            infos,
            vec![
                SensorInfo {
                    id: "cpu/0/load/total".into(),
                    label: t(Lang::It, "sensor.cpu.load.total", &[]),
                    unit: "percent".into(),
                },
                SensorInfo {
                    id: "gpu0/throughput/pcie-rx".into(),
                    label: t(Lang::It, "sensor.cpu.load.total", &[]),
                    unit: "bytes_per_second".into(),
                },
            ]
        );
    }

    #[test]
    fn overlay_strings_have_every_key_in_both_languages() {
        let mut want: Vec<String> = OVERLAY_READS.iter().map(|k| (*k).to_owned()).collect();
        for m in ALL_METRICS {
            let name = serde_json::to_value(m).unwrap();
            want.push(format!("metric.{}", name.as_str().unwrap()));
        }
        want.sort();
        for lang in [Lang::En, Lang::It] {
            let strings = overlay_strings(lang);
            let keys: Vec<String> = strings.keys().cloned().collect();
            assert_eq!(keys, want, "{lang:?}");
            for (key, text) in &strings {
                assert!(!text.is_empty(), "{key} empty for {lang:?}");
                assert!(
                    !text.contains("overlay."),
                    "{key} untranslated for {lang:?}"
                );
                assert_eq!(text.trim(), text, "{key} has outer spaces for {lang:?}");
            }
        }
        for key in RUST_KEYS.iter().filter(|k| k.starts_with("overlay.text.")) {
            let short = key.trim_start_matches("overlay.text.");
            assert!(want.iter().any(|w| w == short), "{key} not sent");
        }
        let it = overlay_strings(Lang::It);
        assert_eq!(it["metric.low-01"], "0,1% low");
        assert_eq!(it["low.percentile"], "(perc.)");
        assert_eq!(it["flag.on"], "Attivo");
        assert_eq!(overlay_strings(Lang::En)["sensorAbsent"], "sensor missing");
    }

    #[test]
    fn every_metric_is_listed() {
        // Fails to compile when a metric is added without a place in ALL_METRICS.
        for m in ALL_METRICS {
            match m {
                FrameMetric::FpsDisplayed
                | FrameMetric::FpsRendered
                | FrameMetric::FpsPresented
                | FrameMetric::FrametimeDisplayed
                | FrameMetric::FrametimeApp
                | FrameMetric::Low1
                | FrameMetric::Low01
                | FrameMetric::FgMultiplier
                | FrameMetric::Stutter
                | FrameMetric::LatencyPc
                | FrameMetric::LatencyDisplay
                | FrameMetric::Bound => {}
            }
        }
        let unique: std::collections::HashSet<FrameMetric> = ALL_METRICS.into_iter().collect();
        assert_eq!(unique.len(), ALL_METRICS.len());
    }

    #[test]
    fn set_profile_carries_profile_sensors_strings_and_settings() {
        let p = profile(json!([block(
            "a",
            json!({ "sensor": "gpu0/temperature/core" })
        )]));
        let mut settings = Settings::default();
        settings.overlay.chart_fps = ChartFps::Fps15;
        settings.overlay.text_hz = 4;
        settings.overlay.hide_from_capture = true;
        settings.overlay.attach = Attach::Monitor;
        settings.general.temperature_unit = TemperatureUnit::F;
        settings.general.throughput_unit = ThroughputUnit::Bytes;
        let OverlayMessage::SetProfile(sp) =
            set_profile("id-1", &p, &schema(), &settings, Lang::It)
        else {
            panic!("not SetProfile");
        };
        assert_eq!(sp.profile_id, "id-1");
        assert_eq!(parse_profile(&sp.profile_json).unwrap(), p);
        assert_eq!(sp.sensors.len(), 1);
        assert_eq!(sp.sensors[0].unit, "celsius");
        assert_eq!(sp.strings, overlay_strings(Lang::It));
        assert_eq!(sp.draw.chart_fps, 15);
        assert_eq!(sp.draw.text_hz, 4);
        assert!(sp.draw.hide_from_capture);
        assert_eq!(sp.draw.attach, "monitor");
        assert!(sp.draw.decimal_comma);
        assert_eq!(sp.draw.temperature_unit, "f");
        assert_eq!(sp.draw.throughput_unit, "bytes");
        assert!(OverlayMessage::SetProfile(sp).validate().is_ok());
    }

    #[test]
    fn values_message_only_has_used_sensors() {
        let snapshot = Snapshot {
            revision: 3,
            seq: 9,
            timestamp_ms: 1,
            values: vec![Some(42.0), Some(1.0), None],
        };
        let quality = [Quality::Fresh, Quality::Held, Quality::Suspended];
        let used = vec![
            "gpu0/temperature/core".to_owned(),
            "cpu/0/load/total".to_owned(),
            "missing/load/x".to_owned(),
        ];
        assert_eq!(
            values_message(&schema(), &snapshot, &quality, &used, 77),
            OverlayMessage::Values(Values {
                at_ms: 77,
                values: vec![
                    WireValue {
                        id: "gpu0/temperature/core".into(),
                        value: None,
                        quality: "suspended".into(),
                    },
                    WireValue {
                        id: "cpu/0/load/total".into(),
                        value: Some(42.0),
                        quality: "fresh".into(),
                    },
                ],
            })
        );
    }

    #[test]
    fn values_message_drops_non_finite_values() {
        let snapshot = Snapshot {
            revision: 3,
            seq: 1,
            timestamp_ms: 1,
            values: vec![Some(f64::NAN), Some(1.0), Some(2.0)],
        };
        let msg = values_message(&schema(), &snapshot, &[], &["cpu/0/load/total".into()], 0);
        let OverlayMessage::Values(v) = &msg else {
            panic!()
        };
        assert_eq!(v.values[0].value, None);
        assert_eq!(v.values[0].quality, "fresh");
        assert!(msg.validate().is_ok());
    }

    fn empty_metrics(state: &str) -> FrameMetrics {
        FrameMetrics {
            state: state.into(),
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
    fn metrics_message_unavailable_without_service() {
        assert_eq!(
            metrics_message(None, "unavailable"),
            OverlayMessage::FrameMetrics(empty_metrics("unavailable"))
        );
    }

    #[test]
    fn metrics_message_maps_the_readout() {
        let readout = FrameReadout {
            fps_displayed: Some(120.0),
            fps_presented: Some(60.0),
            rendered: Rendered::Fps {
                fps: 60.0,
                source: RenderedSource::FrameType,
            },
            rendered_source: Some("FG"),
            fg_multiplier: Some(2.0),
            fg_suspected: false,
            frametime_displayed_ms: Some(8.3),
            frametime_app_ms: Some(16.6),
            lows: vec![
                LowReadout {
                    window_s: 10,
                    definition: LowDefinition::Integral,
                    lows: Some(Lows {
                        one_percent: 90.0,
                        point_one_percent: 70.0,
                    }),
                },
                LowReadout {
                    window_s: 30,
                    definition: LowDefinition::Percentile,
                    lows: None,
                },
            ],
            stutter: Some(Stutter {
                count: 2,
                time_percent: 1.5,
            }),
            latency_pc_ms: Some(f64::NAN),
            latency_display_ms: Some(12.0),
            bottleneck: Some(Bottleneck::Gpu),
            swapchain: Some(5),
        };
        let mut want = empty_metrics("running");
        want.fps_displayed = Some(120.0);
        want.fps_rendered = Some(60.0);
        want.fps_presented = Some(60.0);
        want.rendered_source = Some("FG".into());
        want.frametime_displayed_ms = Some(8.3);
        want.frametime_app_ms = Some(16.6);
        want.fg_multiplier = Some(2.0);
        want.stutter_count = Some(2);
        want.stutter_percent = Some(1.5);
        want.latency_display_ms = Some(12.0);
        want.bound = Some("gpu".into());
        want.lows = vec![
            WireLow {
                window_s: 10,
                definition: "integral".into(),
                one_percent: Some(90.0),
                point_one_percent: Some(70.0),
            },
            WireLow {
                window_s: 30,
                definition: "percentile".into(),
                one_percent: None,
                point_one_percent: None,
            },
        ];
        let msg = metrics_message(Some(&readout), "running");
        assert_eq!(msg, OverlayMessage::FrameMetrics(want));
        assert!(msg.validate().is_ok());

        let suspected = FrameReadout {
            rendered: Rendered::FgSuspected,
            fg_suspected: true,
            bottleneck: None,
            ..readout
        };
        let OverlayMessage::FrameMetrics(m) = metrics_message(Some(&suspected), "running") else {
            panic!()
        };
        assert_eq!(m.fps_rendered, None);
        assert!(m.fg_suspected);
        assert_eq!(m.bound, None);
    }

    fn frame(t_s: f64, swapchain: u64, displayed: bool) -> FrameSample {
        FrameSample {
            t_s,
            swapchain,
            kind: FrameKind::App,
            displayed,
            ms_between_presents: 10.0,
            ms_between_display_change: Some(t_s * 10.0),
            ms_until_displayed: None,
            ms_app_frametime: Some(5.0),
            ms_pc_latency: None,
            ms_gpu_busy: None,
            pcl_frame_id: None,
        }
    }

    fn times(msg: &OverlayMessage) -> Vec<(f64, Option<f64>, Option<f64>)> {
        let OverlayMessage::FrameTimes(FrameTimes { frames }) = msg else {
            panic!("not FrameTimes")
        };
        frames
            .iter()
            .map(|f| (f.t_s, f.displayed_ms, f.app_ms))
            .collect()
    }

    #[test]
    fn frame_times_since_returns_only_new_frames_of_the_main_swapchain() {
        let mut w = FrameWindow::new(60.0);
        w.push(frame(1.0, 1, true));
        w.push(frame(2.0, 2, true));
        w.push(frame(3.0, 1, false));
        w.push(frame(4.0, 1, true));
        w.push(frame(5.0, 2, true));

        let (msg, after) = frame_times_since(&w, Some(1), 1.0);
        assert_eq!(
            times(&msg),
            vec![(3.0, None, Some(5.0)), (4.0, Some(40.0), Some(5.0))]
        );
        assert_eq!(after, 4.0);

        let (msg, after) = frame_times_since(&w, Some(1), after);
        assert!(times(&msg).is_empty());
        assert_eq!(after, 4.0);

        let (msg, after) = frame_times_since(&w, None, 0.5);
        assert!(times(&msg).is_empty());
        assert_eq!(after, 0.5);
    }

    #[test]
    fn frame_times_since_keeps_the_newest_frames_within_the_limit() {
        use oma_ipc::overlay::MAX_FRAME_TIMES;
        let mut w = FrameWindow::new(1e9);
        let n = MAX_FRAME_TIMES + 10;
        for i in 0..n {
            w.push(frame(i as f64, 1, true));
        }
        let (msg, after) = frame_times_since(&w, Some(1), -1.0);
        let got = times(&msg);
        assert_eq!(got.len(), MAX_FRAME_TIMES);
        assert_eq!(got[0].0, 10.0);
        assert_eq!(after, (n - 1) as f64);
        assert!(msg.validate().is_ok());
    }
}
