//! Rule alerts as Windows toasts (spec §3.5, R3): one toast per level entry
//! whose rule notifies that level, within the 5-minute cooldown.

use oma_core::engine::TickOutput;
use oma_core::model::Schema;
use oma_core::rules::{Cooldown, HealthReport, LevelEntry};
use oma_core::settings::Settings;
use serde::Deserialize;

use crate::i18n::{t, Lang};
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
            self.sink.show(
                rule_title(lang, &entry.rule_id),
                body,
                launch_for(&entry.device_id),
            );
        }
    }
}

/// `rule.<id>.name` for a built-in rule, `rule.custom.name` otherwise.
fn rule_title(lang: Lang, rule_id: &str) -> String {
    let key = format!("rule.{rule_id}.name");
    let title = t(lang, &key, &[]);
    // `t` gives the key back for an id without a name: a custom rule.
    if title == key {
        t(lang, "rule.custom.name", &[])
    } else {
        title
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

/// What a toast hands back when clicked: `{"device":"<id>"}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Launch {
    device: String,
}

/// The launch string of a toast about `device_id`; the toast XML escapes it.
pub fn launch_for(device_id: &str) -> String {
    serde_json::json!({ "device": device_id }).to_string()
}

/// The device of a clicked toast; `None` for anything but a launch string
/// made by [`launch_for`].
pub fn device_from_launch(launch: &str) -> Option<String> {
    serde_json::from_str::<Launch>(launch)
        .ok()
        .map(|launch| launch.device)
        .filter(|device| !device.is_empty())
}

/// The Windows toaster: a click posts the navigation to the main thread.
#[cfg(windows)]
pub fn system_sink(app: &tauri::AppHandle) -> oma_win::toast::Toaster {
    let app = app.clone();
    oma_win::toast::Toaster::spawn(Box::new(move |launch| {
        let Some(device) = device_from_launch(&launch) else {
            tracing::warn!("toast activation with an unknown payload: ignored");
            return;
        };
        let handle = app.clone();
        if let Err(err) =
            app.run_on_main_thread(move || crate::window::show_device(&handle, &device))
        {
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
pub fn system_sink(_app: &tauri::AppHandle) -> NoToasts {
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
        assert_eq!(toasts[0].0, "Regola personalizzata");
        assert_eq!(toasts[0].1, "Tctl/Tdie sopra 95 °C (97 °C)");
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
    fn launch_string_carries_the_device() {
        let launch = launch_for("gpu/pci-0000:01:00.0");
        assert_eq!(launch, r#"{"device":"gpu/pci-0000:01:00.0"}"#);
        assert_eq!(
            device_from_launch(&launch).as_deref(),
            Some("gpu/pci-0000:01:00.0")
        );
    }

    #[test]
    fn launch_round_trips_special_characters() {
        for id in [
            "storage/\"quoted\"",
            "storage/a&b<c>'d'",
            "network/{\"device\":\"x\"}",
            "storage/back\\slash/é/✓",
        ] {
            assert_eq!(device_from_launch(&launch_for(id)).as_deref(), Some(id));
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
            assert_eq!(device_from_launch(launch), None, "{launch}");
        }
    }
}
