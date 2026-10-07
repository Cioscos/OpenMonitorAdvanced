//! Rule alerts as Windows toasts (spec §3.5, R3): one toast per level entry
//! whose rule notifies that level, within the 5-minute cooldown.

use oma_core::engine::TickOutput;
use oma_core::model::Schema;
use oma_core::rules::{Alert, Cooldown, HealthReport, LevelEntry};
use oma_core::settings::Settings;
use serde::Deserialize;

use crate::i18n::{sensor_label, t, Lang};
use crate::tray::language_for;
use crate::tray_icon::alert_text;

/// Where the toasts go: the Windows toaster, or a recorder in the tests.
/// `show` must not block.
pub trait ToastSink: Send {
    fn show(&self, title: String, body: String, launch: String);
}

/// Turns level entries into toasts. One lives for the whole session, window
/// or not, so its cooldown survives rule changes and reopened windows.
pub struct Notifier<S: ToastSink> {
    cooldown: Cooldown,
    sink: S,
}

impl<S: ToastSink> Notifier<S> {
    pub fn new(sink: S) -> Self {
        Self {
            cooldown: Cooldown::new(),
            sink,
        }
    }

    /// One toast per entry whose rule notifies its level and that the
    /// cooldown admits at `now_ms` (the monotonic clock of the rules); the
    /// attempt counts even if the toast then fails. Texts come from the
    /// entry's alert in `report`, in the language of `settings`.
    pub fn on_entries(
        &mut self,
        entries: &[LevelEntry],
        report: &HealthReport,
        schema: &Schema,
        settings: &Settings,
        now_ms: u64,
    ) {
        let mut lang = None;
        for entry in entries.iter().filter(|entry| entry.notify) {
            let Some(alert) = report
                .alerts
                .iter()
                .find(|a| a.rule_id == entry.rule_id && a.sensor_id == entry.sensor_id)
            else {
                tracing::debug!(rule = %entry.rule_id, "level entry without its alert: no toast");
                continue;
            };
            if !self
                .cooldown
                .admit(&entry.rule_id, &entry.sensor_id, entry.level, now_ms)
            {
                continue;
            }
            // Read once, and only when a toast is due.
            let lang = *lang.get_or_insert_with(|| language_for(settings.general.language));
            let body = alert_text(
                lang,
                alert,
                schema,
                settings.general.temperature_unit,
                settings.general.throughput_unit,
            );
            self.sink
                .show(rule_title(lang, alert), body, launch_for(&entry.device_id));
        }
    }
}

/// `rule.<id>.name` for a built-in rule; for a custom one, the device and
/// the sensor as the tray and the banner name them ("CPU · Total load"),
/// or `rule.custom.name` when the sensor's label has no translation.
fn rule_title(lang: Lang, alert: &Alert) -> String {
    let key = format!("rule.{}.name", alert.rule_id);
    let title = t(lang, &key, &[]);
    // `t` gives the key back for an id without a name: a custom rule.
    if title != key {
        return title;
    }
    let label = sensor_label(lang, &alert.sensor_label);
    if label == format!("sensor.{}", alert.sensor_label.key) {
        return t(lang, "rule.custom.name", &[]);
    }
    match alert.params.get("device") {
        Some(device) if !device.is_empty() => format!("{device} · {label}"),
        _ => label,
    }
}

/// The alert side of every tick: keeps the latest schema and health report
/// (a tick carries them only when they change), which the tray reads too,
/// and hands the tick's entries to the notifier.
pub struct AlertFeed<S: ToastSink> {
    notifier: Notifier<S>,
    schema: Option<Schema>,
    health: HealthReport,
}

impl<S: ToastSink> AlertFeed<S> {
    pub fn new(sink: S) -> Self {
        Self {
            notifier: Notifier::new(sink),
            schema: None,
            health: HealthReport::default(),
        }
    }

    pub fn tick(&mut self, out: &TickOutput, settings: &Settings) {
        if let Some(schema) = &out.schema {
            self.schema = Some(schema.clone());
        }
        if let Some(health) = &out.health {
            self.health = health.clone();
        }
        if out.entries.is_empty() {
            return;
        }
        if let Some(schema) = &self.schema {
            self.notifier.on_entries(
                &out.entries,
                &self.health,
                schema,
                settings,
                out.monotonic_ms,
            );
        }
    }

    pub fn schema(&self) -> Option<&Schema> {
        self.schema.as_ref()
    }

    pub fn health(&self) -> &HealthReport {
        &self.health
    }
}

/// A clicked toast of a device: `{"device":"<id>"}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceLaunch {
    device: String,
}

/// A clicked toast that opens a window: `{"open":"main"}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenLaunch {
    open: String,
}

/// A clicked GPU benchmark toast: `{"open":"score-gpu","device":"<device id>"}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScoreGpuLaunch {
    open: String,
    device: String,
}

/// A clicked stress test toast: `{"performance":"<session id>"}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PerformanceLaunch {
    performance: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Launch {
    Device(DeviceLaunch),
    Open(OpenLaunch),
    Performance(PerformanceLaunch),
    ScoreGpu(ScoreGpuLaunch),
}

/// Where a clicked toast leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchTarget {
    /// The Advanced view on this device's page (rule alerts).
    Device(String),
    /// The main window (log toasts, L8).
    Main,
    /// Settings › About (update toasts).
    About,
    /// The result of this stress session (M8a1).
    Performance(String),
    /// The CPU benchmark page (M8a2, DB9).
    ScoreCpu,
    /// The benchmark page of this GPU (M8b2, DH12).
    ScoreGpu(String),
}

/// The launch string of a toast about `device_id`; the toast XML escapes it.
pub fn launch_for(device_id: &str) -> String {
    serde_json::json!({ "device": device_id }).to_string()
}

/// The launch string of a toast that opens the main window.
pub fn launch_for_main() -> String {
    serde_json::json!({ "open": "main" }).to_string()
}

/// The launch string of a toast that opens Settings › About (updates).
pub fn launch_for_about() -> String {
    serde_json::json!({ "open": "about" }).to_string()
}

/// The launch string of a toast that opens the CPU benchmark page.
pub fn launch_for_score_cpu() -> String {
    serde_json::json!({ "open": "score-cpu" }).to_string()
}

/// The launch string of a toast that opens the benchmark page of GPU `device_id`.
pub fn launch_for_score_gpu(device_id: &str) -> String {
    serde_json::json!({ "open": "score-gpu", "device": device_id }).to_string()
}

/// The shape of a GPU `device_id` (`gpu/pci-0000:01:00.0`, `gpu/ven-10de-dev-2704-0`):
/// a toast's launch string comes from outside the app, so nothing else goes through.
pub fn is_gpu_device_id(id: &str) -> bool {
    id.len() <= 64
        && id.strip_prefix("gpu/").is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'-'))
                && !rest.contains("..")
        })
}

/// The launch string of a toast that opens the result of a stress session.
pub fn launch_for_performance(session_id: &str) -> String {
    serde_json::json!({ "performance": session_id }).to_string()
}

/// The target of a clicked toast; `None` for anything but a launch string
/// made by [`launch_for`] or [`launch_for_main`].
pub fn launch_target(launch: &str) -> Option<LaunchTarget> {
    match serde_json::from_str::<Launch>(launch).ok()? {
        Launch::Device(DeviceLaunch { device }) if !device.is_empty() => {
            Some(LaunchTarget::Device(device))
        }
        Launch::Open(OpenLaunch { open }) if open == "main" => Some(LaunchTarget::Main),
        Launch::Open(OpenLaunch { open }) if open == "about" => Some(LaunchTarget::About),
        Launch::Open(OpenLaunch { open }) if open == "score-cpu" => Some(LaunchTarget::ScoreCpu),
        Launch::ScoreGpu(ScoreGpuLaunch { open, device })
            if open == "score-gpu" && is_gpu_device_id(&device) =>
        {
            Some(LaunchTarget::ScoreGpu(device))
        }
        Launch::Performance(PerformanceLaunch { performance })
            if oma_core::load::is_session_id(&performance) =>
        {
            Some(LaunchTarget::Performance(performance))
        }
        _ => None,
    }
}

/// A shared toaster (rules and log, L8) is a sink too.
impl<T: ToastSink + Sync> ToastSink for std::sync::Arc<T> {
    fn show(&self, title: String, body: String, launch: String) {
        (**self).show(title, body, launch);
    }
}

/// The toaster the app shares between rule alerts and the log.
#[cfg(windows)]
pub type SystemToaster = oma_win::toast::Toaster;
#[cfg(not(windows))]
pub type SystemToaster = NoToasts;

/// The Windows toaster, created once in `setup`: a click posts the
/// navigation to the main thread.
#[cfg(windows)]
pub fn system_toaster(app: &tauri::AppHandle) -> SystemToaster {
    let app = app.clone();
    oma_win::toast::Toaster::spawn(Box::new(move |launch| {
        let Some(target) = launch_target(&launch) else {
            tracing::warn!("toast activation with an unknown payload: ignored");
            return;
        };
        let handle = app.clone();
        let result = app.run_on_main_thread(move || match target {
            LaunchTarget::Device(device) => crate::window::show_device(&handle, &device),
            LaunchTarget::Main => crate::window::show_main(&handle),
            LaunchTarget::About => crate::window::show_about(&handle),
            LaunchTarget::Performance(id) => {
                crate::window::show_performance(&handle, crate::window::PerformanceNav::result(&id))
            }
            LaunchTarget::ScoreCpu => {
                crate::window::show_performance(&handle, crate::window::PerformanceNav::score_cpu())
            }
            LaunchTarget::ScoreGpu(device) => crate::window::show_performance(
                &handle,
                crate::window::PerformanceNav::score_gpu(&device),
            ),
        });
        if let Err(err) = result {
            tracing::warn!(%err, "cannot open the window for a toast");
        }
    }))
}

#[cfg(windows)]
impl ToastSink for oma_win::toast::Toaster {
    fn show(&self, title: String, body: String, launch: String) {
        oma_win::toast::Toaster::show(self, title, body, launch);
    }
}

/// No toasts off Windows.
#[cfg(not(windows))]
pub struct NoToasts;

#[cfg(not(windows))]
impl ToastSink for NoToasts {
    fn show(&self, _title: String, _body: String, _launch: String) {}
}

#[cfg(not(windows))]
pub fn system_toaster(_app: &tauri::AppHandle) -> SystemToaster {
    NoToasts
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use oma_core::engine::Quality;
    use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Snapshot, Source, Unit};
    use oma_core::rules::{
        default_rules, Alert, Coverage, Level, OverallLevel, RuleEngine, TOAST_COOLDOWN_MS,
    };
    use oma_core::settings::Language;

    use super::*;

    const CPU_TEMP: &str = "cpu/0/temperature/tctl";
    const WALL0: u64 = 1_700_000_000_000;

    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<(String, String, String)>>>);

    impl Recorder {
        fn toasts(&self) -> Vec<(String, String, String)> {
            self.0.lock().unwrap().clone()
        }
    }

    impl ToastSink for Recorder {
        fn show(&self, title: String, body: String, launch: String) {
            self.0.lock().unwrap().push((title, body, launch));
        }
    }

    fn settings(language: Language) -> Settings {
        let mut settings = Settings::default();
        settings.general.language = language;
        settings
    }

    fn cpu_schema(with_sensor: bool) -> Schema {
        Schema {
            revision: 1,
            devices: vec![Device {
                id: "cpu/0".into(),
                kind: DeviceKind::Cpu,
                name: "Ryzen 7".into(),
                vendor: None,
                properties: Default::default(),
            }],
            sensors: if with_sensor {
                vec![Sensor::new(
                    "cpu/0",
                    SensorKind::Temperature,
                    "tctl",
                    Unit::Celsius,
                    Label::new("cpu.temperature.tctl"),
                    Source::Lhm,
                )]
            } else {
                Vec::new()
            },
        }
    }

    /// The rule engine with the built-in rules, one tick a second, producing
    /// the ticks the sampler would.
    struct Rig {
        engine: RuleEngine,
        schema: Schema,
        changed: bool,
        ms: u64,
    }

    impl Rig {
        fn new(schema: Schema) -> Self {
            let mut engine = RuleEngine::new();
            engine.set_interval_ms(1000);
            engine.set_rules(default_rules());
            Self {
                engine,
                schema,
                changed: true,
                ms: 0,
            }
        }

        fn set_schema(&mut self, schema: Schema) {
            self.schema = schema;
            self.changed = true;
        }

        fn tick(&mut self, values: &[Option<f64>]) -> TickOutput {
            let quality = vec![Quality::Fresh; values.len()];
            let evaluation = self.engine.evaluate(
                &self.schema,
                self.changed,
                values,
                &quality,
                self.ms,
                WALL0 + self.ms,
            );
            let out = TickOutput {
                snapshot: Snapshot {
                    revision: self.schema.revision,
                    seq: self.ms / 1000,
                    timestamp_ms: WALL0 + self.ms,
                    values: values.to_vec(),
                },
                schema: self.changed.then(|| self.schema.clone()),
                quality,
                health: evaluation.report,
                entries: evaluation.entries,
                monotonic_ms: self.ms,
            };
            self.changed = false;
            self.ms += 1000;
            out
        }
    }

    fn alert(rule_id: &str, sensor_id: &str, device_id: &str, level: Level) -> Alert {
        Alert {
            rule_id: rule_id.into(),
            sensor_id: sensor_id.into(),
            device_id: device_id.into(),
            unit: Unit::Celsius,
            sensor_label: Label::new("cpu.temperature.tctl"),
            level,
            value: Some(97.0),
            threshold: Some(95.0),
            since_ms: WALL0,
            valid: true,
            last_valid_ms: Some(WALL0),
            message_key: if rule_id.starts_with("custom-") {
                "rule.custom.above".into()
            } else {
                format!("rule.{rule_id}.message")
            },
            params: BTreeMap::from([("device".to_owned(), "Ryzen 7".to_owned())]),
        }
    }

    fn entry(rule_id: &str, level: Level, notify: bool) -> LevelEntry {
        LevelEntry {
            rule_id: rule_id.into(),
            sensor_id: CPU_TEMP.into(),
            device_id: "cpu/0".into(),
            level,
            notify,
        }
    }

    fn report(alerts: Vec<Alert>) -> HealthReport {
        HealthReport {
            level: OverallLevel::Crit,
            since_ms: WALL0,
            revision: 2,
            coverage: Coverage::Complete,
            unavailable_targets: Vec::new(),
            alerts,
        }
    }

    #[test]
    fn notifier_shows_one_toast_per_admitted_entry() {
        let recorder = Recorder::default();
        let mut notifier = Notifier::new(recorder.clone());
        let report = report(vec![alert("cpu-temp", CPU_TEMP, "cpu/0", Level::Crit)]);
        let schema = cpu_schema(true);
        let en = settings(Language::En);
        let entries = [entry("cpu-temp", Level::Crit, true)];
        notifier.on_entries(&entries, &report, &schema, &en, 0);
        // Within the cooldown: no second toast.
        notifier.on_entries(&entries, &report, &schema, &en, TOAST_COOLDOWN_MS - 1);
        assert_eq!(
            recorder.toasts(),
            [(
                "CPU temperature".to_owned(),
                "Ryzen 7 overheating (97 °C)".to_owned(),
                launch_for("cpu/0"),
            )]
        );
        // Past it: the next entry toasts again.
        notifier.on_entries(&entries, &report, &schema, &en, TOAST_COOLDOWN_MS);
        assert_eq!(recorder.toasts().len(), 2);
    }

    #[test]
    fn notifier_respects_notify_flags() {
        let recorder = Recorder::default();
        let mut notifier = Notifier::new(recorder.clone());
        let report = report(vec![alert("cpu-temp", CPU_TEMP, "cpu/0", Level::Warn)]);
        let schema = cpu_schema(true);
        // `notify.warn` is off by default for the built-in rules.
        notifier.on_entries(
            &[entry("cpu-temp", Level::Warn, false)],
            &report,
            &schema,
            &settings(Language::En),
            0,
        );
        assert!(recorder.toasts().is_empty());
        // A refused entry did not consume the cooldown.
        notifier.on_entries(
            &[entry("cpu-temp", Level::Warn, true)],
            &report,
            &schema,
            &settings(Language::En),
            1,
        );
        assert_eq!(recorder.toasts().len(), 1);
    }

    #[test]
    fn custom_rule_has_a_localized_title() {
        let id = "custom-0b1c2d3e-4f50-4617-8293-a4b5c6d7e8f9";
        let recorder = Recorder::default();
        let mut notifier = Notifier::new(recorder.clone());
        let report = report(vec![alert(id, CPU_TEMP, "cpu/0", Level::Crit)]);
        notifier.on_entries(
            &[entry(id, Level::Crit, true)],
            &report,
            &cpu_schema(true),
            &settings(Language::It),
            0,
        );
        let toasts = recorder.toasts();
        // The device and the sensor, as the tray and the banner name them.
        assert_eq!(toasts[0].0, "Ryzen 7 · Tctl/Tdie");
        assert_eq!(toasts[0].1, "Tctl/Tdie sopra 95 °C (97 °C)");
    }

    #[test]
    fn custom_rule_without_a_known_label_uses_the_generic_title() {
        let id = "custom-0b1c2d3e-4f50-4617-8293-a4b5c6d7e8f9";
        let recorder = Recorder::default();
        let mut notifier = Notifier::new(recorder.clone());
        let mut unknown = alert(id, CPU_TEMP, "cpu/0", Level::Crit);
        unknown.sensor_label = Label::new("cpu.temperature.nope");
        notifier.on_entries(
            &[entry(id, Level::Crit, true)],
            &report(vec![unknown]),
            &cpu_schema(true),
            &settings(Language::It),
            0,
        );
        assert_eq!(recorder.toasts()[0].0, "Regola personalizzata");
    }

    #[test]
    fn service_reconnect_does_not_retoast() {
        let recorder = Recorder::default();
        let mut feed = AlertFeed::new(recorder.clone());
        let en = settings(Language::En);
        let mut rig = Rig::new(cpu_schema(true));
        let mut entries = 0;
        // CPU at 100 °C: critical after 10 s.
        for _ in 0..=10 {
            let out = rig.tick(&[Some(100.0)]);
            entries += out.entries.len();
            feed.tick(&out, &en);
        }
        assert_eq!(recorder.toasts().len(), 1);
        // The service goes away for longer than the cooldown, then comes back
        // with the CPU still hot.
        rig.set_schema(cpu_schema(false));
        for _ in 0..400 {
            let out = rig.tick(&[]);
            entries += out.entries.len();
            feed.tick(&out, &en);
        }
        rig.set_schema(cpu_schema(true));
        for _ in 0..20 {
            let out = rig.tick(&[Some(100.0)]);
            entries += out.entries.len();
            feed.tick(&out, &en);
        }
        assert_eq!(entries, 1, "the returning sensor is not a new entry");
        assert_eq!(recorder.toasts().len(), 1);
        assert_eq!(feed.health().level, OverallLevel::Crit);
    }

    #[test]
    fn notifier_runs_with_window_closed() {
        // The feed needs no window: the schema of an earlier tick is kept for
        // the entry of a later one, whose tick carries no schema.
        let recorder = Recorder::default();
        let mut feed = AlertFeed::new(recorder.clone());
        let en = settings(Language::En);
        let mut rig = Rig::new(cpu_schema(true));
        let first = rig.tick(&[Some(50.0)]);
        assert!(first.schema.is_some());
        feed.tick(&first, &en);
        for _ in 0..=10 {
            let out = rig.tick(&[Some(100.0)]);
            assert!(out.schema.is_none());
            feed.tick(&out, &en);
        }
        assert_eq!(recorder.toasts().len(), 1);
        assert_eq!(recorder.toasts()[0].2, launch_for("cpu/0"));
        assert!(feed.schema().is_some());
    }

    #[test]
    fn notifier_survives_settings_and_window_changes() {
        let recorder = Recorder::default();
        let mut notifier = Notifier::new(recorder.clone());
        let report = report(vec![alert("cpu-temp", CPU_TEMP, "cpu/0", Level::Crit)]);
        let schema = cpu_schema(true);
        let entries = [entry("cpu-temp", Level::Crit, true)];
        notifier.on_entries(&entries, &report, &schema, &settings(Language::En), 0);
        // New language, new rules, window reopened: the same notifier, the
        // same cooldown.
        let mut changed = settings(Language::It);
        changed
            .rules
            .overrides
            .insert("cpu-temp".into(), Default::default());
        notifier.on_entries(&entries, &report, &schema, &changed, 60_000);
        assert_eq!(recorder.toasts().len(), 1);
        notifier.on_entries(&entries, &report, &schema, &changed, TOAST_COOLDOWN_MS);
        let toasts = recorder.toasts();
        assert_eq!(toasts.len(), 2);
        assert_eq!(toasts[1].0, "Temperatura CPU");
    }

    #[test]
    fn about_launch_round_trips() {
        assert_eq!(
            launch_target(&launch_for_about()),
            Some(LaunchTarget::About)
        );
        assert_eq!(launch_target(r#"{"open":"other"}"#), None);
    }

    #[test]
    fn launch_target_performance_round_trips() {
        let id = "0b9f6c1e-7d2a-4c53-9a1e-3f5d8e2b7a10";
        assert_eq!(
            launch_for_performance(id),
            format!(r#"{{"performance":"{id}"}}"#)
        );
        assert_eq!(
            launch_target(&launch_for_performance(id)),
            Some(LaunchTarget::Performance(id.into()))
        );
        for bad in [r#"{"performance":"../x"}"#, r#"{"performance":""}"#] {
            assert_eq!(launch_target(bad), None, "{bad}");
        }
    }

    #[test]
    fn launch_targets() {
        let launch = launch_for("gpu/0");
        assert_eq!(launch, r#"{"device":"gpu/0"}"#);
        assert_eq!(
            launch_target(&launch),
            Some(LaunchTarget::Device("gpu/0".into()))
        );
        assert_eq!(launch_for_main(), r#"{"open":"main"}"#);
        assert_eq!(launch_target(&launch_for_main()), Some(LaunchTarget::Main));
        assert_eq!(
            launch_target(&launch_for_score_cpu()),
            Some(LaunchTarget::ScoreCpu)
        );
        assert_eq!(launch_for_about(), r#"{"open":"about"}"#);
        for unknown in [
            r#"{"open":"settings"}"#,
            r#"{"open":"main","device":"gpu/0"}"#,
            r#"{"close":"main"}"#,
        ] {
            assert_eq!(launch_target(unknown), None, "{unknown}");
        }
    }

    #[test]
    fn launch_target_refuses_a_malformed_device() {
        for id in ["gpu/pci-0000:01:00.0", "gpu/ven-10de-dev-2704-0", "gpu/0"] {
            assert_eq!(
                launch_target(&launch_for_score_gpu(id)),
                Some(LaunchTarget::ScoreGpu(id.into())),
                "{id}"
            );
        }
        let long = format!(
            r#"{{"open":"score-gpu","device":"gpu/{}"}}"#,
            "a".repeat(80)
        );
        for bad in [
            r#"{"open":"score-gpu"}"#,
            r#"{"open":"score-gpu","device":""}"#,
            r#"{"open":"score-gpu","device":"gpu/"}"#,
            r#"{"open":"score-gpu","device":"cpu/0"}"#,
            r#"{"open":"score-gpu","device":"gpu/../x"}"#,
            r#"{"open":"score-gpu","device":"gpu/a b"}"#,
            r#"{"open":"score-gpu","device":"gpu/a\"b"}"#,
            r#"{"open":"score-gpu","device":"gpu/é"}"#,
            r#"{"open":"score-gpu","device":42}"#,
            r#"{"open":"score-gpu","device":"gpu/0","extra":1}"#,
            r#"{"open":"score-cpu","device":"gpu/0"}"#,
            r#"{"open":"main","device":"gpu/0"}"#,
            long.as_str(),
        ] {
            assert_eq!(launch_target(bad), None, "{bad}");
        }
    }

    #[test]
    fn launch_round_trips_special_characters() {
        for id in [
            "storage/\"quoted\"",
            "storage/a&b<c>'d'",
            "network/{\"device\":\"x\"}",
            "storage/back\\slash/é/✓",
        ] {
            assert_eq!(
                launch_target(&launch_for(id)),
                Some(LaunchTarget::Device(id.into()))
            );
        }
    }

    #[test]
    fn invalid_launch_is_ignored() {
        for launch in [
            "",
            "cpu/0",
            "{}",
            "[]",
            r#"{"device":""}"#,
            r#"{"device":42}"#,
            r#"{"device":"cpu/0","extra":1}"#,
            r#"{"device":"cpu/0""#,
        ] {
            assert_eq!(launch_target(launch), None, "{launch}");
        }
    }
}
