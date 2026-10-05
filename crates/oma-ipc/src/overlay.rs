//! App <-> overlay protocol (version 1): the messages the app and `oma-overlay.exe`
//! exchange over the overlay pipe, framed with the generic framing of this crate.
//!
//! Same conventions as the service protocol: every field is always present on the wire
//! (`nil` for "absent"; no `skip_serializing_if`), enumerated values travel as strings,
//! and unknown fields are ignored. A receiver calls [`OverlayMessage::validate`] after
//! decoding.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::IpcError;

/// Overlay protocol version, sent in [`OverlayHello::protocol_version`] by both sides.
pub const OVERLAY_PROTOCOL_VERSION: u32 = 1;

/// Prefix of the overlay pipe name; the app appends a random UUID v4.
pub const OVERLAY_PIPE_PREFIX: &str = r"\\.\pipe\OpenMonitorAdvanced-Overlay-";

/// Maximum entries in [`Values::values`].
pub const MAX_OVERLAY_VALUES: usize = 1024;
/// Maximum entries in [`FrameTimes::frames`].
pub const MAX_FRAME_TIMES: usize = 4096;
/// Maximum entries in [`SetProfile::sensors`].
pub const MAX_SENSOR_INFOS: usize = 1024;
/// Maximum entries in [`SetProfile::strings`].
pub const MAX_STRINGS: usize = 256;
/// Maximum entries in [`FrameMetrics::lows`].
pub const MAX_LOWS: usize = 16;

/// Handshake, sent by both sides right after the connection is up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayHello {
    pub protocol_version: u32,
    /// The sender application version (`X.Y.Z`).
    pub version: String,
}

/// A sensor the profile uses: its translated label and unit (the snake_case string of
/// the core `Unit`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorInfo {
    pub id: String,
    pub label: String,
    pub unit: String,
}

/// Drawing options that are not part of the profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawSettings {
    pub chart_fps: u32,
    pub text_hz: u32,
    pub hide_from_capture: bool,
    pub attach: String,
    pub decimal_comma: bool,
    pub temperature_unit: String,
    pub throughput_unit: String,
}

/// App to overlay: the active profile (as JSON text, re-parsed by the overlay with the
/// same parser), its sensors, the translated strings and the drawing options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetProfile {
    pub profile_id: String,
    pub profile_json: String,
    pub sensors: Vec<SensorInfo>,
    pub strings: BTreeMap<String, String>,
    pub draw: DrawSettings,
}

/// A rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PxArea {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// App to overlay: where the target client area is; `nil` means hidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPlacement {
    pub area: Option<PxArea>,
    pub dpi: u32,
}

/// One sensor value; `quality` is `fresh`, `held` or `suspended`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireValue {
    pub id: String,
    pub value: Option<f64>,
    pub quality: String,
}

/// App to overlay: a sensor tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Values {
    pub at_ms: u64,
    pub values: Vec<WireValue>,
}

/// Low FPS for one window and definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireLow {
    pub window_s: u32,
    pub definition: String,
    pub one_percent: Option<f64>,
    pub point_one_percent: Option<f64>,
}

/// App to overlay: the frame metrics. `state` is one of the frames states or
/// `unavailable` (service missing or incompatible); `bound` is `gpu`, `cpu` or `unknown`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameMetrics {
    pub state: String,
    pub fps_displayed: Option<f64>,
    pub fps_rendered: Option<f64>,
    pub fps_presented: Option<f64>,
    pub rendered_source: Option<String>,
    pub fg_suspected: bool,
    pub frametime_displayed_ms: Option<f64>,
    pub frametime_app_ms: Option<f64>,
    pub fg_multiplier: Option<f64>,
    pub stutter_count: Option<u32>,
    pub stutter_percent: Option<f64>,
    pub latency_pc_ms: Option<f64>,
    pub latency_display_ms: Option<f64>,
    pub bound: Option<String>,
    pub lows: Vec<WireLow>,
}

/// One frame for the frametime chart.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireFrameTime {
    pub t_s: f64,
    pub displayed_ms: Option<f64>,
    pub app_ms: Option<f64>,
}

/// App to overlay: new frames for the frametime chart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameTimes {
    pub frames: Vec<WireFrameTime>,
}

/// A message on the overlay pipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
pub enum OverlayMessage {
    Hello(OverlayHello),
    SetProfile(SetProfile),
    SetPlacement(SetPlacement),
    Values(Values),
    FrameMetrics(FrameMetrics),
    FrameTimes(FrameTimes),
}

fn check_len(what: &str, n: usize, max: usize) -> Result<(), IpcError> {
    if n > max {
        Err(IpcError::Decode(format!(
            "{what} has {n} entries, the maximum is {max}"
        )))
    } else {
        Ok(())
    }
}

fn check_finite(what: &str, v: f64) -> Result<(), IpcError> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(IpcError::Decode(format!("{what} is not finite")))
    }
}

fn check_opt_finite(what: &str, v: Option<f64>) -> Result<(), IpcError> {
    v.map_or(Ok(()), |v| check_finite(what, v))
}

impl OverlayMessage {
    /// Rejects lists over their limits and non-finite numbers. The receiver calls it
    /// after decoding; the decoder alone lets both through.
    pub fn validate(&self) -> Result<(), IpcError> {
        match self {
            Self::SetProfile(p) => {
                check_len("sensors", p.sensors.len(), MAX_SENSOR_INFOS)?;
                check_len("strings", p.strings.len(), MAX_STRINGS)
            }
            Self::Values(v) => {
                check_len("values", v.values.len(), MAX_OVERLAY_VALUES)?;
                v.values
                    .iter()
                    .try_for_each(|w| check_opt_finite("value", w.value))
            }
            Self::FrameTimes(f) => {
                check_len("frames", f.frames.len(), MAX_FRAME_TIMES)?;
                f.frames.iter().try_for_each(|w| {
                    check_finite("t_s", w.t_s)?;
                    check_opt_finite("displayed_ms", w.displayed_ms)?;
                    check_opt_finite("app_ms", w.app_ms)
                })
            }
            Self::FrameMetrics(m) => {
                check_len("lows", m.lows.len(), MAX_LOWS)?;
                for (what, v) in [
                    ("fps_displayed", m.fps_displayed),
                    ("fps_rendered", m.fps_rendered),
                    ("fps_presented", m.fps_presented),
                    ("frametime_displayed_ms", m.frametime_displayed_ms),
                    ("frametime_app_ms", m.frametime_app_ms),
                    ("fg_multiplier", m.fg_multiplier),
                    ("stutter_percent", m.stutter_percent),
                    ("latency_pc_ms", m.latency_pc_ms),
                    ("latency_display_ms", m.latency_display_ms),
                ] {
                    check_opt_finite(what, v)?;
                }
                m.lows.iter().try_for_each(|l| {
                    check_opt_finite("one_percent", l.one_percent)?;
                    check_opt_finite("point_one_percent", l.point_one_percent)
                })
            }
            Self::Hello(_) | Self::SetPlacement(_) => Ok(()),
        }
    }
}

/// True when the peer speaks the same overlay protocol version.
pub fn overlay_compatible(hello: &OverlayHello) -> bool {
    hello.protocol_version == OVERLAY_PROTOCOL_VERSION
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_payload_of, encode_frame_of, FrameDecoder};

    fn round_trip(msg: OverlayMessage) {
        let frame = encode_frame_of(&msg).unwrap();
        let mut decoder = FrameDecoder::new();
        decoder.push(&frame).unwrap();
        assert_eq!(decoder.next_of::<OverlayMessage>().unwrap(), Some(msg));
    }

    fn hello() -> OverlayHello {
        OverlayHello {
            protocol_version: OVERLAY_PROTOCOL_VERSION,
            version: "0.5.0".into(),
        }
    }

    fn draw() -> DrawSettings {
        DrawSettings {
            chart_fps: 30,
            text_hz: 4,
            hide_from_capture: true,
            attach: "game".into(),
            decimal_comma: false,
            temperature_unit: "celsius".into(),
            throughput_unit: "bytes".into(),
        }
    }

    fn metrics() -> FrameMetrics {
        FrameMetrics {
            state: "running".into(),
            fps_displayed: Some(120.0),
            fps_rendered: None,
            fps_presented: Some(60.0),
            rendered_source: None,
            fg_suspected: false,
            frametime_displayed_ms: None,
            frametime_app_ms: Some(16.6),
            fg_multiplier: None,
            stutter_count: Some(3),
            stutter_percent: None,
            latency_pc_ms: None,
            latency_display_ms: None,
            bound: Some("gpu".into()),
            lows: vec![WireLow {
                window_s: 10,
                definition: "integral".into(),
                one_percent: Some(50.0),
                point_one_percent: None,
            }],
        }
    }

    #[test]
    fn overlay_messages_round_trip() {
        round_trip(OverlayMessage::Hello(hello()));
        round_trip(OverlayMessage::SetProfile(SetProfile {
            profile_id: "builtin-gaming".into(),
            profile_json: "{}".into(),
            sensors: vec![SensorInfo {
                id: "cpu/temperature/package".into(),
                label: "CPU".into(),
                unit: "celsius".into(),
            }],
            strings: BTreeMap::from([("a".to_owned(), "b".to_owned())]),
            draw: draw(),
        }));
        round_trip(OverlayMessage::SetPlacement(SetPlacement {
            area: Some(PxArea {
                x: -10,
                y: 20,
                width: 1920,
                height: 1080,
            }),
            dpi: 144,
        }));
        round_trip(OverlayMessage::SetPlacement(SetPlacement {
            area: None,
            dpi: 96,
        }));
        round_trip(OverlayMessage::Values(Values {
            at_ms: 123_456,
            values: vec![
                WireValue {
                    id: "a".into(),
                    value: Some(1.5),
                    quality: "fresh".into(),
                },
                WireValue {
                    id: "b".into(),
                    value: None,
                    quality: "suspended".into(),
                },
            ],
        }));
        round_trip(OverlayMessage::FrameMetrics(metrics()));
        round_trip(OverlayMessage::FrameTimes(FrameTimes {
            frames: vec![WireFrameTime {
                t_s: 1.0,
                displayed_ms: Some(8.3),
                app_ms: None,
            }],
        }));
    }

    #[test]
    fn absent_values_are_nil_on_the_wire() {
        let bytes = rmp_serde::to_vec_named(&OverlayMessage::FrameMetrics(metrics())).unwrap();
        let v: serde_json::Value = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(v["type"], "frame_metrics");
        let body = v["body"].as_object().unwrap();
        for key in [
            "fps_rendered",
            "rendered_source",
            "frametime_displayed_ms",
            "fg_multiplier",
            "stutter_percent",
            "latency_pc_ms",
            "latency_display_ms",
        ] {
            assert!(body.contains_key(key), "{key} missing");
            assert!(body[key].is_null(), "{key} not nil");
        }
        let low = body["lows"][0].as_object().unwrap();
        assert!(low.contains_key("point_one_percent"));
        assert!(low["point_one_percent"].is_null());

        let bytes = rmp_serde::to_vec_named(&OverlayMessage::SetPlacement(SetPlacement {
            area: None,
            dpi: 96,
        }))
        .unwrap();
        let v: serde_json::Value = rmp_serde::from_slice(&bytes).unwrap();
        assert!(v["body"].as_object().unwrap().contains_key("area"));
        assert!(v["body"]["area"].is_null());
    }

    fn values(n: usize, v: Option<f64>) -> OverlayMessage {
        OverlayMessage::Values(Values {
            at_ms: 0,
            values: (0..n)
                .map(|i| WireValue {
                    id: i.to_string(),
                    value: v,
                    quality: "fresh".into(),
                })
                .collect(),
        })
    }

    #[test]
    fn validate_rejects_oversized_lists() {
        assert!(values(MAX_OVERLAY_VALUES, Some(1.0)).validate().is_ok());
        assert!(values(MAX_OVERLAY_VALUES + 1, Some(1.0))
            .validate()
            .is_err());

        let frames = |n| {
            OverlayMessage::FrameTimes(FrameTimes {
                frames: vec![
                    WireFrameTime {
                        t_s: 0.0,
                        displayed_ms: None,
                        app_ms: None,
                    };
                    n
                ],
            })
        };
        assert!(frames(MAX_FRAME_TIMES).validate().is_ok());
        assert!(frames(MAX_FRAME_TIMES + 1).validate().is_err());

        let mut m = metrics();
        m.lows = vec![m.lows[0].clone(); MAX_LOWS];
        assert!(OverlayMessage::FrameMetrics(m.clone()).validate().is_ok());
        m.lows.push(m.lows[0].clone());
        assert!(OverlayMessage::FrameMetrics(m).validate().is_err());

        let profile = |sensors: usize, strings: usize| {
            OverlayMessage::SetProfile(SetProfile {
                profile_id: "p".into(),
                profile_json: "{}".into(),
                sensors: vec![
                    SensorInfo {
                        id: "i".into(),
                        label: "l".into(),
                        unit: "u".into(),
                    };
                    sensors
                ],
                strings: (0..strings)
                    .map(|i| (i.to_string(), String::new()))
                    .collect(),
                draw: draw(),
            })
        };
        assert!(profile(MAX_SENSOR_INFOS, MAX_STRINGS).validate().is_ok());
        assert!(profile(MAX_SENSOR_INFOS + 1, 0).validate().is_err());
        assert!(profile(0, MAX_STRINGS + 1).validate().is_err());
    }

    #[test]
    fn validate_rejects_non_finite_values() {
        assert!(values(1, Some(f64::NAN)).validate().is_err());
        assert!(values(1, Some(f64::INFINITY)).validate().is_err());
        assert!(values(1, None).validate().is_ok());
        let ft = |t_s, d| {
            OverlayMessage::FrameTimes(FrameTimes {
                frames: vec![WireFrameTime {
                    t_s,
                    displayed_ms: d,
                    app_ms: None,
                }],
            })
        };
        assert!(ft(f64::NAN, None).validate().is_err());
        assert!(ft(0.0, Some(f64::NEG_INFINITY)).validate().is_err());
        assert!(ft(0.0, Some(1.0)).validate().is_ok());
        // A hostile sender's NaN survives decoding, so the receiver must validate.
        let bytes = rmp_serde::to_vec_named(&values(1, Some(f64::NAN))).unwrap();
        let decoded: OverlayMessage = decode_payload_of(&bytes).unwrap();
        assert!(decoded.validate().is_err());
    }

    #[test]
    fn overlay_hello_compatibility() {
        assert!(overlay_compatible(&hello()));
        let mut h = hello();
        h.protocol_version += 1;
        assert!(!overlay_compatible(&h));
        h.protocol_version = 0;
        assert!(!overlay_compatible(&h));
    }
}
