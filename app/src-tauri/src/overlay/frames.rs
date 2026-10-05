//! Frame diagnostics, on request only: with `OMA_FRAMES_DEBUG` set to `1`
//! (displayed FPS), `pcl` (plus PC latency) or `all` (plus GPU busy), the app
//! turns the service's frame engine on, follows the foreground game with a
//! [`TargetPicker`] and writes one `frames:` line per second to its log.
//! Any other value, or none, starts nothing.
//!
//! The line is `key=value` pairs separated by spaces, always in this order,
//! with `-` for a value that is not available:
//!
//! ```text
//! frames: target=game.exe pid=4242 state=running detail=- fps=143.8 rendered=72.1 source=Reflex mult=1.99 low1=98.4 low01=61.0 ft_ms=6.95 stutter=2 stutter_pct=0.41 pc_lat_ms=31.2 disp_lat_ms=8.1 bottleneck=gpu dropped=0
//! ```
//!
//! - `fps`, `rendered`, `mult`, `ft_ms` (mean displayed frametime), `pc_lat_ms`
//!   and `disp_lat_ms` are over the last second ([`FPS_WINDOW_S`]); `low1`,
//!   `low01` (integral lows), `stutter`, `stutter_pct` and `bottleneck` over
//!   the last ten ([`LOWS_WINDOW_S`]), all on the main swapchain.
//! - `source` is the rendered-FPS origin (`Reflex`, `XeSS-FG`, `AFMF`, `FG`),
//!   or `FG?` when frame generation is only suspected (then `rendered=-`).
//! - `state` is the service's frame-engine state; an unknown one is `failed`.
//!   `detail` is the status's detail (for example why it failed), as one
//!   token.
//! - `bottleneck` is `-` unless `all` turned GPU tracking on.
//! - `dropped` is the running total of frames the service left out, summed
//!   over every batch received (whatever its process).
//! - The windows are anchored on the newest frame, not on the clock: while
//!   no new frames arrive, the line repeats the last figures.

use std::fmt::Write as _;

use oma_core::frames::metrics::{
    bottleneck, displayed_fps, lows, mean_display_latency, mean_pc_latency, stutter, Bottleneck,
    LowDefinition,
};
use oma_core::frames::{
    fg_multiplier, fg_suspected, pick_swapchain, rendered_fps, source_label, FrameKind,
    FrameSample, FrameWindow, Rendered, FPS_WINDOW_S, LOWS_WINDOW_S,
};
use oma_ipc::{frames_state, FrameBatch, FramesConfigure, FramesStatus, PresentingProcesses};

use super::target::{ProcessInfo, TargetPicker, SYSTEM_EXCLUDED};

/// The environment variable that turns the diagnostics on.
pub const ENV_VAR: &str = "OMA_FRAMES_DEBUG";

/// How often a `frames:` line is written (and the configuration re-sent).
const REPORT_MS: u64 = 1_000;

/// The window frame generation is suspected over (SD5).
const FG_WINDOW_S: f64 = 2.0;

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
fn sample_of(frame: &oma_ipc::WireFrame, qpc_frequency: u64) -> FrameSample {
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
fn state_label(status: Option<&FramesStatus>) -> &str {
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

/// The frames of `frames` (ordered by `t_s`) in the trailing `seconds`.
fn trailing(frames: &[FrameSample], seconds: f64) -> &[FrameSample] {
    let Some(newest) = frames.last().map(|f| f.t_s) else {
        return frames;
    };
    let start = frames.partition_point(|f| f.t_s < newest - seconds);
    &frames[start..]
}

/// What one [`Diagnostics::step`] asks the caller to do.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Step {
    /// Send `SetFramesTarget` with this value.
    pub target_changed: Option<Option<u32>>,
    /// Write this `frames:` line (and re-send the configuration and target).
    pub report: Option<String>,
}

/// The diagnostics' state, fed by the `oma-frames` thread. Pure: the caller
/// supplies the feed's data, the foreground PID and a monotonic clock.
pub(crate) struct Diagnostics {
    config: FramesConfigure,
    picker: TargetPicker,
    target: Option<u32>,
    window: FrameWindow,
    qpc_frequency: u64,
    state: String,
    detail: String,
    dropped: u64,
    last_report_ms: u64,
}

impl Diagnostics {
    pub fn new(config: FramesConfigure, own_pids: Vec<u32>, qpc_frequency: u64) -> Self {
        Self {
            config,
            picker: TargetPicker::new(
                own_pids,
                SYSTEM_EXCLUDED.iter().map(|s| (*s).to_owned()).collect(),
            ),
            target: None,
            window: FrameWindow::new(LOWS_WINDOW_S),
            qpc_frequency,
            state: "-".to_owned(),
            detail: "-".to_owned(),
            dropped: 0,
            last_report_ms: 0,
        }
    }

    pub fn target(&self) -> Option<u32> {
        self.target
    }

    pub fn step(
        &mut self,
        status: Option<&FramesStatus>,
        processes: Option<&PresentingProcesses>,
        batches: &[FrameBatch],
        foreground: Option<u32>,
        now_ms: u64,
    ) -> Step {
        let list: Vec<ProcessInfo> = processes
            .map(|list| {
                list.processes
                    .iter()
                    .map(|p| ProcessInfo {
                        pid: p.pid,
                        name: p.name.clone(),
                        displayed_fps: p.displayed_fps,
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.picker.on_processes(&list, now_ms);
        if let Some(pid) = foreground {
            self.picker.on_foreground(pid, now_ms);
        }
        let target_changed = self.picker.tick(now_ms);
        if let Some(target) = target_changed {
            self.target = target;
            self.window.clear();
        }
        self.state = state_label(status).to_owned();
        self.detail = status
            .and_then(|s| s.detail.as_deref())
            .map_or_else(|| "-".to_owned(), token);
        for batch in batches {
            self.dropped = self.dropped.saturating_add(u64::from(batch.dropped));
            // A batch of the previous target may still arrive after a change.
            if self.qpc_frequency == 0 || Some(batch.pid) != self.target {
                continue;
            }
            for frame in &batch.frames {
                self.window.push(sample_of(frame, self.qpc_frequency));
            }
        }
        let report = (now_ms.saturating_sub(self.last_report_ms) >= REPORT_MS).then(|| {
            self.last_report_ms = now_ms;
            self.line()
        });
        Step {
            target_changed,
            report,
        }
    }

    fn line(&self) -> String {
        let all = self.window.last(LOWS_WINDOW_S);
        let main: Vec<FrameSample> = match pick_swapchain(&all) {
            Some(swapchain) => all
                .into_iter()
                .filter(|f| f.swapchain == swapchain)
                .collect(),
            None => Vec::new(),
        };
        let second = trailing(&main, FPS_WINDOW_S);

        let fps = displayed_fps(second);
        let rendered = match rendered_fps(second) {
            Rendered::Unavailable | Rendered::FgSuspected => {
                if fg_suspected(trailing(&main, FG_WINDOW_S)) {
                    Rendered::FgSuspected
                } else {
                    Rendered::Unavailable
                }
            }
            figure => figure,
        };
        let (rendered_value, source) = match rendered {
            Rendered::Fps { fps, source } => (Some(fps), source_label(source, second)),
            Rendered::FgSuspected => (None, "FG?"),
            Rendered::Unavailable => (None, "-"),
        };
        let frametimes: Vec<f64> = main
            .iter()
            .filter(|f| f.displayed)
            .filter_map(|f| f.ms_between_display_change)
            .collect();
        let low = lows(&frametimes, LowDefinition::Integral);
        let stutters = (!frametimes.is_empty()).then(|| stutter(&main));
        let bottleneck = if self.config.track_gpu {
            match bottleneck(&main, rendered == Rendered::FgSuspected) {
                Bottleneck::Gpu => "gpu",
                Bottleneck::Cpu => "cpu",
                Bottleneck::Unknown => "unknown",
            }
        } else {
            "-"
        };
        let current = self.picker.current().filter(|p| Some(p.pid) == self.target);

        let mut line = String::from("frames:");
        let _ = write!(
            line,
            " target={} pid={} state={} detail={}",
            current.map_or_else(|| "-".to_owned(), |p| token(&p.name)),
            current.map_or_else(|| "-".to_owned(), |p| p.pid.to_string()),
            self.state,
            self.detail,
        );
        let _ = write!(
            line,
            " fps={} rendered={} source={source} mult={}",
            num(fps, 1),
            num(rendered_value, 1),
            num(fg_multiplier(fps, &rendered), 2),
        );
        let _ = write!(
            line,
            " low1={} low01={} ft_ms={}",
            num(low.map(|l| l.one_percent), 1),
            num(low.map(|l| l.point_one_percent), 1),
            num(fps.map(|f| 1000.0 / f), 2),
        );
        let _ = write!(
            line,
            " stutter={} stutter_pct={} pc_lat_ms={} disp_lat_ms={}",
            stutters.map_or_else(|| "-".to_owned(), |s| s.count.to_string()),
            num(stutters.map(|s| s.time_percent), 2),
            num(mean_pc_latency(second), 1),
            num(mean_display_latency(second), 1),
        );
        let _ = write!(line, " bottleneck={bottleneck} dropped={}", self.dropped);
        line
    }
}

#[cfg(windows)]
pub use runner::{start_if_requested, FramesDiagnostics};

#[cfg(windows)]
mod runner {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{mpsc, Arc};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use oma_ipc::FramesConfigure;
    use oma_win::foreground::ForegroundWatcher;
    use oma_win::svc::{FramesFeed, LinkCommand};

    use super::{options_from_env, Diagnostics, ENV_VAR};

    /// The `oma-frames` thread's period (4 Hz).
    const TICK: Duration = Duration::from_millis(250);

    /// Sends a command to the service link without blocking
    /// ([`crate::service::ServiceShell::link_commands`]).
    pub type LinkSink = Box<dyn Fn(LinkCommand) + Send + Sync>;

    /// The running diagnostics: dropping it (or [`Self::stop`]) stops the
    /// `oma-frames` thread, then closes the foreground watcher.
    pub struct FramesDiagnostics {
        stop: Option<mpsc::Sender<()>>,
        thread: Option<JoinHandle<()>>,
        /// Dropped after the thread has stopped (fields drop after `drop`).
        _watcher: Option<ForegroundWatcher>,
    }

    impl FramesDiagnostics {
        pub fn stop(self) {
            drop(self);
        }
    }

    impl Drop for FramesDiagnostics {
        fn drop(&mut self) {
            // A dropped sender wakes the thread's wait at once.
            self.stop.take();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// Starts the diagnostics if `OMA_FRAMES_DEBUG` asks for them; call it
    /// once the service link is spawned. Otherwise nothing starts.
    pub fn start_if_requested(link: LinkSink, feed: FramesFeed) -> Option<FramesDiagnostics> {
        let value = std::env::var(ENV_VAR).ok();
        let config = options_from_env(value.as_deref())?;
        let qpc_frequency = oma_win::qpc_frequency();
        if qpc_frequency == 0 {
            tracing::warn!("no QPC frequency: frame times cannot be converted, frames are ignored");
        }
        link(LinkCommand::ConfigureFrames(config.clone()));

        // 0 = no foreground reported yet. The sink only stores: it runs on
        // the `oma-foreground` thread and must never block.
        let foreground = Arc::new(AtomicU32::new(0));
        let watcher = {
            let foreground = Arc::clone(&foreground);
            match ForegroundWatcher::spawn(Box::new(move |pid| {
                foreground.store(pid, Ordering::Relaxed);
            })) {
                Ok(watcher) => Some(watcher),
                Err(err) => {
                    tracing::warn!(%err, "frame diagnostics: no foreground watcher, no target");
                    None
                }
            }
        };
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("oma-frames".into())
            .spawn(move || run(config, link, feed, foreground, stop_rx, qpc_frequency));
        match spawned {
            Ok(thread) => {
                tracing::info!(
                    value = value.as_deref().unwrap_or(""),
                    "frame diagnostics on"
                );
                Some(FramesDiagnostics {
                    stop: Some(stop_tx),
                    thread: Some(thread),
                    _watcher: watcher,
                })
            }
            Err(err) => {
                tracing::warn!(%err, "frame diagnostics: the thread could not start");
                None
            }
        }
    }

    fn run(
        config: FramesConfigure,
        link: LinkSink,
        feed: FramesFeed,
        foreground: Arc<AtomicU32>,
        stop: mpsc::Receiver<()>,
        qpc_frequency: u64,
    ) {
        let start = Instant::now();
        let mut diagnostics =
            Diagnostics::new(config.clone(), vec![std::process::id()], qpc_frequency);
        while let Err(mpsc::RecvTimeoutError::Timeout) = stop.recv_timeout(TICK) {
            let now_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            let update = feed.drain();
            let pid = foreground.load(Ordering::Relaxed);
            let step = diagnostics.step(
                update.status.as_ref(),
                update.processes.as_ref(),
                &update.batches,
                (pid != 0).then_some(pid),
                now_ms,
            );
            if let Some(target) = step.target_changed {
                link(LinkCommand::SetFramesTarget(target));
            }
            if let Some(line) = step.report {
                // A command that met a full link queue is lost: re-send both
                // once a second (the link drops unchanged values).
                link(LinkCommand::ConfigureFrames(config.clone()));
                link(LinkCommand::SetFramesTarget(diagnostics.target()));
                tracing::info!("{line}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_ipc::{PresentingProcess, WireFrame};

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

    const FREQ: u64 = 10_000_000;
    const GAME: u32 = 100;

    fn status(state: &str) -> FramesStatus {
        FramesStatus {
            state: state.to_owned(),
            detail: None,
            presentmon_version: None,
        }
    }

    fn processes() -> PresentingProcesses {
        PresentingProcesses {
            at_qpc: 0,
            processes: vec![PresentingProcess {
                pid: GAME,
                name: "my game.exe".to_owned(),
                displayed_fps: 100.0,
                present_mode: "Hardware: Independent Flip".to_owned(),
                swapchains: 1,
            }],
        }
    }

    /// `count` displayed app frames 10 ms apart, from `first_s`.
    fn batch(pid: u32, first_s: f64, count: u32, dropped: u32) -> FrameBatch {
        let frames = (0..count)
            .map(|i| WireFrame {
                qpc: ((first_s + f64::from(i) * 0.01) * FREQ as f64) as u64,
                swapchain: 0xabc,
                frame_type: "app".to_owned(),
                displayed: true,
                ms_between_presents: 10.0,
                ms_between_display_change: Some(10.0),
                ms_until_displayed: Some(5.0),
                ms_app_frametime: Some(10.0),
                ms_pc_latency: None,
                ms_gpu_busy: None,
                pcl_frame_id: None,
            })
            .collect();
        FrameBatch {
            pid,
            frames,
            dropped,
        }
    }

    fn field<'a>(line: &'a str, key: &str) -> &'a str {
        line.split(' ')
            .find_map(|kv| kv.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
            .unwrap_or_else(|| panic!("no {key} in {line}"))
    }

    #[test]
    fn line_without_data_has_every_key_with_dashes() {
        let mut d = Diagnostics::new(config(false, false), vec![], FREQ);
        let step = d.step(None, None, &[], None, 1_000);
        let line = step.report.expect("a line after one second");
        assert_eq!(
            line,
            "frames: target=- pid=- state=- detail=- fps=- rendered=- source=- mult=- low1=- low01=- \
             ft_ms=- stutter=- stutter_pct=- pc_lat_ms=- disp_lat_ms=- bottleneck=- dropped=0"
        );
    }

    #[test]
    fn line_reports_the_targets_frames_once_a_second() {
        let mut d = Diagnostics::new(config(false, false), vec![], FREQ);
        let first = d.step(
            Some(&status("running")),
            Some(&processes()),
            &[],
            Some(GAME),
            0,
        );
        assert_eq!(first.target_changed, Some(Some(GAME)));
        assert_eq!(first.report, None);
        // Another process's frames, distinguishable: they would pull the
        // displayed FPS far from 100 if they reached the window.
        let mut foreign = batch(999, 5.0, 50, 2);
        for frame in &mut foreign.frames {
            frame.ms_between_display_change = Some(50.0);
        }
        let step = d.step(
            Some(&status("running")),
            Some(&processes()),
            &[batch(GAME, 5.0, 101, 3), foreign],
            Some(GAME),
            1_000,
        );
        assert_eq!(step.target_changed, None);
        let line = step.report.expect("a line");
        assert_eq!(field(&line, "target"), "my_game.exe");
        assert_eq!(field(&line, "pid"), "100");
        assert_eq!(field(&line, "state"), "running");
        assert_eq!(field(&line, "fps"), "100.0");
        assert_eq!(field(&line, "ft_ms"), "10.00");
        assert_eq!(field(&line, "disp_lat_ms"), "5.0");
        assert_eq!(field(&line, "stutter"), "0");
        assert_eq!(field(&line, "dropped"), "5");
        assert_eq!(
            d.step(None, Some(&processes()), &[], Some(GAME), 1_250)
                .report,
            None
        );
    }

    #[test]
    fn unknown_state_is_logged_as_failed() {
        let mut d = Diagnostics::new(config(false, false), vec![], FREQ);
        let line = d
            .step(Some(&status("exploded")), None, &[], None, 1_000)
            .report
            .unwrap();
        assert_eq!(field(&line, "state"), "failed");
        assert_eq!(field(&line, "detail"), "-");
        let mut with_detail = status("failed");
        with_detail.detail = Some("bad columns".to_owned());
        let line = d
            .step(Some(&with_detail), None, &[], None, 2_000)
            .report
            .unwrap();
        assert_eq!(field(&line, "detail"), "bad_columns");
    }

    #[test]
    fn target_change_clears_the_window() {
        let mut d = Diagnostics::new(config(false, false), vec![], FREQ);
        d.step(None, Some(&processes()), &[], Some(GAME), 0);
        d.step(
            None,
            Some(&processes()),
            &[batch(GAME, 5.0, 50, 0)],
            Some(GAME),
            250,
        );
        // Elsewhere from 500 ms for 3 s: the target drops and its frames go
        // with it.
        assert_eq!(
            d.step(None, Some(&processes()), &[], Some(300), 500)
                .target_changed,
            None
        );
        let step = d.step(None, Some(&processes()), &[], Some(300), 3_500);
        assert_eq!(step.target_changed, Some(None));
        let line = step.report.unwrap();
        assert_eq!(field(&line, "fps"), "-");
        assert_eq!(field(&line, "target"), "-");
    }

    #[test]
    fn zero_qpc_frequency_ignores_frames() {
        let mut d = Diagnostics::new(config(false, false), vec![], 0);
        d.step(None, Some(&processes()), &[], Some(GAME), 0);
        let line = d
            .step(
                None,
                Some(&processes()),
                &[batch(GAME, 5.0, 50, 1)],
                Some(GAME),
                1_000,
            )
            .report
            .unwrap();
        assert_eq!(field(&line, "fps"), "-");
        assert_eq!(field(&line, "dropped"), "1");
    }
}
