//! Frame diagnostics, on request only: with `OMA_FRAMES_DEBUG` set to `1`
//! (displayed FPS), `pcl` (plus PC latency) or `all` (plus GPU busy), the
//! overlay controller's thread ([`super::runner`]) turns the service's frame
//! engine on, follows the foreground game (the
//! [`Controller`](super::controller::Controller)'s target) and writes one
//! `frames:` line per second to the app's log. Any other value, or none,
//! starts nothing.
//!
//! The line is `key=value` pairs separated by spaces, always in this order,
//! with `-` for a value that is not available:
//!
//! ```text
//! frames: target=game.exe pid=4242 state=running detail=- fps=143.8 rendered=72.1 source=Reflex mult=1.99 low1=98.4 low01=61.0 ft_ms=6.95 stutter=2 stutter_pct=0.41 pc_lat_ms=31.2 disp_lat_ms=8.1 bottleneck=gpu dropped=0
//! ```
//!
//! - `fps`, `rendered`, `mult`, `ft_ms` (mean displayed frametime), `pc_lat_ms`
//!   and `disp_lat_ms` are over the last second
//!   ([`oma_core::frames::FPS_WINDOW_S`]); `low1`, `low01` (integral lows),
//!   `stutter`, `stutter_pct` and `bottleneck` over the last ten
//!   ([`oma_core::frames::LOWS_WINDOW_S`]), all on the main swapchain.
//! - `source` is the rendered-FPS origin (`Reflex`, `XeSS-FG`, `AFMF`, `FG`),
//!   or `FG?` when frame generation is only suspected (then `rendered=-`).
//! - `state` is the service's frame-engine state; an unknown one is `failed`.
//!   `detail` is the status's detail (for example why it failed), as one
//!   token.
//! - `bottleneck` is `-` unless GPU tracking is on (`all`, or the overlay's
//!   setting).
//! - `dropped` is the running total of frames the service left out, summed
//!   over every batch received (whatever its process).
//! - The windows are anchored on the newest frame, not on the clock: while
//!   no new frames arrive, the line repeats the last figures.

use std::fmt::Write as _;

use oma_core::frames::metrics::Bottleneck;
use oma_core::frames::{FrameKind, FrameReadout, FrameSample, Rendered};
use oma_ipc::{frames_state, FramesConfigure, FramesStatus};

/// The environment variable that turns the diagnostics on.
pub const ENV_VAR: &str = "OMA_FRAMES_DEBUG";

/// The frame-engine configuration `OMA_FRAMES_DEBUG` asks for, if any.
pub fn options_from_env(value: Option<&str>) -> Option<FramesConfigure> {
    let (track_pc_latency, track_gpu) = match value? {
        "1" => (false, false),
        "pcl" => (true, false),
        "all" => (true, true),
        _ => return None,
    };
    Some(FramesConfigure {
        enabled: true,
        track_pc_latency,
        track_gpu,
    })
}

/// A wire frame on the app's clock: `t_s` is the frame's own QPC time in
/// seconds, so late batches still land where they belong. `qpc_frequency`
/// must not be 0.
pub(crate) fn sample_of(frame: &oma_ipc::WireFrame, qpc_frequency: u64) -> FrameSample {
    FrameSample {
        t_s: frame.qpc as f64 / qpc_frequency as f64,
        swapchain: frame.swapchain,
        kind: FrameKind::from_wire(&frame.frame_type),
        displayed: frame.displayed,
        ms_between_presents: frame.ms_between_presents,
        ms_between_display_change: frame.ms_between_display_change,
        ms_until_displayed: frame.ms_until_displayed,
        ms_app_frametime: frame.ms_app_frametime,
        ms_pc_latency: frame.ms_pc_latency,
        ms_gpu_busy: frame.ms_gpu_busy,
        pcl_frame_id: frame.pcl_frame_id,
    }
}

/// The engine state for the line: a known value as is, an unknown one as
/// `failed` (protocol README), `-` before any status.
pub(crate) fn state_label(status: Option<&FramesStatus>) -> &str {
    let Some(status) = status else { return "-" };
    match status.state.as_str() {
        known @ (frames_state::OFF
        | frames_state::STARTING
        | frames_state::RUNNING
        | frames_state::DENIED
        | frames_state::TAMPERED
        | frames_state::MISSING
        | frames_state::FAILED) => known,
        _ => frames_state::FAILED,
    }
}

/// The status's detail as one token, `-` without one.
pub(crate) fn detail_label(status: Option<&FramesStatus>) -> String {
    status
        .and_then(|s| s.detail.as_deref())
        .map_or_else(|| "-".to_owned(), token)
}

/// A name as one grep-friendly token.
fn token(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_whitespace() || c == '=' {
                '_'
            } else {
                c
            }
        })
        .collect()
}

fn num(value: Option<f64>, decimals: usize) -> String {
    match value {
        Some(v) if v.is_finite() => format!("{v:.decimals$}"),
        _ => "-".to_owned(),
    }
}

/// What the `frames:` line shows besides the readout.
pub(crate) struct LineContext<'a> {
    /// The followed process: name and PID.
    pub target: Option<(&'a str, u32)>,
    /// From [`state_label`].
    pub state: &'a str,
    /// From [`detail_label`].
    pub detail: &'a str,
    /// Frames the service left out, summed over every batch received.
    pub dropped: u64,
}

/// The `frames:` line (format in the module docs). `readout.lows` starts with
/// the integral lows over ten seconds, as [`oma_core::frames::read`] always
/// puts them.
pub(crate) fn line(readout: &FrameReadout, ctx: &LineContext<'_>) -> String {
    let fps = readout.fps_displayed;
    let source = readout.rendered_source.unwrap_or("-");
    let rendered_value = match readout.rendered {
        Rendered::Fps { fps, .. } => Some(fps),
        _ => None,
    };
    let low = readout.lows.first().and_then(|l| l.lows);
    let stutters = readout.stutter;
    let bottleneck = match readout.bottleneck {
        Some(Bottleneck::Gpu) => "gpu",
        Some(Bottleneck::Cpu) => "cpu",
        Some(Bottleneck::Unknown) => "unknown",
        None => "-",
    };

    let mut line = String::from("frames:");
    let _ = write!(
        line,
        " target={} pid={} state={} detail={}",
        ctx.target
            .map_or_else(|| "-".to_owned(), |(name, _)| token(name)),
        ctx.target
            .map_or_else(|| "-".to_owned(), |(_, pid)| pid.to_string()),
        ctx.state,
        ctx.detail,
    );
    let _ = write!(
        line,
        " fps={} rendered={} source={source} mult={}",
        num(fps, 1),
        num(rendered_value, 1),
        num(readout.fg_multiplier, 2),
    );
    let _ = write!(
        line,
        " low1={} low01={} ft_ms={}",
        num(low.map(|l| l.one_percent), 1),
        num(low.map(|l| l.point_one_percent), 1),
        num(readout.frametime_displayed_ms, 2),
    );
    let _ = write!(
        line,
        " stutter={} stutter_pct={} pc_lat_ms={} disp_lat_ms={}",
        stutters.map_or_else(|| "-".to_owned(), |s| s.count.to_string()),
        num(stutters.map(|s| s.time_percent), 2),
        num(readout.latency_pc_ms, 1),
        num(readout.latency_display_ms, 1),
    );
    let _ = write!(line, " bottleneck={bottleneck} dropped={}", ctx.dropped);
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(track_pc_latency: bool, track_gpu: bool) -> FramesConfigure {
        FramesConfigure {
            enabled: true,
            track_pc_latency,
            track_gpu,
        }
    }

    #[test]
    fn env_value_maps_to_options() {
        assert_eq!(options_from_env(Some("1")), Some(config(false, false)));
        assert_eq!(options_from_env(Some("pcl")), Some(config(true, false)));
        assert_eq!(options_from_env(Some("all")), Some(config(true, true)));
        assert_eq!(options_from_env(Some("0")), None);
        assert_eq!(options_from_env(Some("")), None);
        assert_eq!(options_from_env(None), None);
    }
}
