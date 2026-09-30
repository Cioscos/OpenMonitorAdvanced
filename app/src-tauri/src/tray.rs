//! Tray icon: the menu (open, views, log, anti-cheat mode, quit), a dynamic icon
//! in the color of the health level (with a red dot while the log records) and
//! a tooltip led by its verdict, refreshed every tick, and the labels' language.

use std::sync::{Arc, Mutex, PoisonError};

use oma_core::model::{DeviceKind, Schema, Snapshot, Unit};
use oma_core::rules::HealthReport;
use oma_core::settings::{Language, Settings, ViewKind};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::i18n::{resolve, t, Lang};
use crate::log::session::{LogState, LogStatus};
use crate::log::LogService;
use crate::notifier::SystemToaster;
use crate::service::ServiceShell;
use crate::settings::SettingsStore;
use crate::tray_icon::{
    icon_content, render, style_for, tooltip, verdict, IconContent, IconStyle, TooltipItem,
    ICON_SIZE, PRODUCT_NAME,
};
use crate::window;

const CPU_TEMPERATURES: [&str; 2] = ["cpu/0/temperature/package", "cpu/0/temperature/tctl"];
const CPU_LOAD: &str = "cpu/0/load/total";
const MEMORY_LOAD: &str = "memory/0/load/used";

/// What the controller needs from the real tray, so its decisions can be
/// tested without a window. Implementations must not block the caller.
pub trait TrayBackend: Send + Sync {
    /// A 32x32 RGBA image.
    fn set_icon(&self, rgba: Vec<u8>);
    fn set_tooltip(&self, text: String);
    /// Rebuilds the menu in `lang`, with the log items of `log`.
    fn set_menu(&self, lang: Lang, log: LogState);
}

/// The log items of the tray menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogMenuItem {
    Start,
    Pause,
    Resume,
    Stop,
}

impl LogMenuItem {
    pub fn id(self) -> &'static str {
        match self {
            Self::Start => "log_start",
            Self::Pause => "log_pause",
            Self::Resume => "log_resume",
            Self::Stop => "log_stop",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Start, Self::Pause, Self::Resume, Self::Stop]
            .into_iter()
            .find(|item| item.id() == id)
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Self::Start => "tray.log.start",
            Self::Pause => "tray.log.pause",
            Self::Resume => "tray.log.resume",
            Self::Stop => "tray.log.stop",
        }
    }
}

/// The log items to show while the log is in `state`.
pub fn log_menu(state: LogState) -> &'static [LogMenuItem] {
    match state {
        LogState::Idle | LogState::Error => &[LogMenuItem::Start],
        LogState::Recording => &[LogMenuItem::Pause, LogMenuItem::Stop],
        LogState::Paused => &[LogMenuItem::Resume, LogMenuItem::Stop],
    }
}

/// The toast for a log command started from the tray that ended in `error`,
/// when no window is open to show it (the coordinator toasts only errors of
/// its own, R6).
fn error_toast(lang: Lang, window_open: bool, status: &LogStatus) -> Option<(String, String)> {
    if window_open {
        return None;
    }
    crate::log::commands::failure_toast(lang, status)
}

fn index_of(schema: &Schema, id: &str) -> Option<usize> {
    schema.sensors.iter().position(|sensor| sensor.id == id)
}

/// Core temperature of the first dedicated (non-integrated) GPU that has one.
fn dedicated_gpu_temperature(schema: &Schema) -> Option<usize> {
    schema
        .devices
        .iter()
        .filter(|device| {
            device.kind == DeviceKind::Gpu
                && device.properties.get("integrated").map(String::as_str) != Some("true")
        })
        .find_map(|device| index_of(schema, &format!("{}/temperature/core", device.id)))
}

fn cpu_temperature(schema: &Schema) -> Option<usize> {
    CPU_TEMPERATURES.iter().find_map(|id| index_of(schema, id))
}

fn icon_index(schema: &Schema, chosen: Option<&str>) -> Option<usize> {
    chosen
        .and_then(|id| index_of(schema, id))
        .or_else(|| dedicated_gpu_temperature(schema))
        .or_else(|| cpu_temperature(schema))
        .or_else(|| index_of(schema, CPU_LOAD))
}

/// The sensor shown on the icon: `chosen` when the schema has it, otherwise
/// the dedicated GPU's core temperature, the CPU temperature, or the total CPU
/// load; `None` when the schema has none of them.
// The tray resolves positions through `icon_index` (same logic, no id
// allocation per tick); this id form is the specified, tested interface.
#[cfg_attr(not(test), allow(dead_code))]
pub fn icon_sensor(schema: &Schema, chosen: Option<&str>) -> Option<String> {
    icon_index(schema, chosen).map(|index| schema.sensors[index].id.clone())
}

/// Sensor positions for one schema and one chosen icon sensor, so a tick does
/// not search the schema again.
struct Resolved {
    revision: u64,
    chosen: Option<String>,
    icon: Option<usize>,
    cpu: Option<usize>,
    gpu: Option<usize>,
    ram: Option<usize>,
}

impl Resolved {
    fn new(schema: &Schema, chosen: Option<&str>) -> Self {
        Self {
            revision: schema.revision,
            chosen: chosen.map(str::to_owned),
            icon: icon_index(schema, chosen),
            cpu: cpu_temperature(schema).or_else(|| index_of(schema, CPU_LOAD)),
            gpu: dedicated_gpu_temperature(schema),
            ram: index_of(schema, MEMORY_LOAD),
        }
    }

    fn matches(&self, schema: &Schema, chosen: Option<&str>) -> bool {
        self.revision == schema.revision && self.chosen.as_deref() == chosen
    }
}

struct State {
    lang: Lang,
    resolved: Option<Resolved>,
    log: LogState,
    /// What the icon last sent shows, its style and whether it has the dot.
    icon: Option<(IconContent, IconStyle, bool)>,
    tooltip: Option<String>,
}

/// Keeps the tray icon, tooltip and labels in line with the readings and the
/// settings, and talks to the tray only when something changed.
pub struct TrayController<B: TrayBackend> {
    backend: B,
    state: Mutex<State>,
}

impl<B: TrayBackend> TrayController<B> {
    pub fn new(backend: B, lang: Lang) -> Self {
        Self {
            backend,
            state: Mutex::new(State {
                lang,
                log: LogState::Idle,
                resolved: None,
                icon: None,
                tooltip: None,
            }),
        }
    }

    /// Called every tick with the latest health report, whose level colors the
    /// icon and whose verdict leads the tooltip. The verdict is rebuilt every
    /// time, so a change of language or units shows at once even while the
    /// report stays the same. A snapshot of another schema revision is
    /// skipped: the next tick brings the matching pair.
    pub fn update(
        &self,
        schema: &Schema,
        snapshot: &Snapshot,
        health: &HealthReport,
        settings: &Settings,
    ) {
        if snapshot.revision != schema.revision {
            return;
        }
        let chosen = settings.tray.icon_sensor.as_deref();
        let temperature = settings.general.temperature_unit;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !state
            .resolved
            .as_ref()
            .is_some_and(|resolved| resolved.matches(schema, chosen))
        {
            state.resolved = Some(Resolved::new(schema, chosen));
        }
        let resolved = state.resolved.as_ref().expect("resolved above");
        let (icon, cpu, gpu, ram) = (resolved.icon, resolved.cpu, resolved.gpu, resolved.ram);
        let reading = |index: Option<usize>| -> (Option<f64>, Unit) {
            let Some(index) = index else {
                return (None, Unit::Percent);
            };
            let value = snapshot.values.get(index).copied().flatten();
            let unit = schema.sensors.get(index).map_or(Unit::Percent, |s| s.unit);
            (value, unit)
        };

        let (value, unit) = reading(icon);
        let content = icon_content(value, unit, temperature);
        let style = style_for(health.level);
        let recording = state.log == LogState::Recording;
        if !state.icon.as_ref().is_some_and(|(sent, sent_style, dot)| {
            *sent == content && *sent_style == style && *dot == recording
        }) {
            self.backend.set_icon(render(&content, style, recording));
            state.icon = Some((content, style, recording));
        }

        let item = |label_key, index| {
            let (value, unit) = reading(index);
            TooltipItem {
                label_key,
                value,
                unit,
            }
        };
        let verdict = verdict(
            state.lang,
            health,
            schema,
            temperature,
            settings.general.throughput_unit,
        );
        let text = tooltip(
            state.lang,
            verdict.as_deref(),
            &[
                item("tray.tooltip.cpu", cpu),
                item("tray.tooltip.gpu", gpu),
                item("tray.tooltip.ram", ram),
            ],
            temperature,
        );
        if state.tooltip.as_deref() != Some(text.as_str()) {
            self.backend.set_tooltip(text.clone());
            state.tooltip = Some(text);
        }
    }

    /// Switches the menu and tooltip language; a no-op when it is unchanged.
    pub fn relabel(&self, lang: Lang) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.lang == lang {
            return;
        }
        state.lang = lang;
        // The tooltip words change with the language: rebuild it on the next tick.
        state.tooltip = None;
        self.backend.set_menu(lang, state.log);
    }

    /// Follows the log: rebuilds the menu, and redraws the icon when the dot
    /// appears or goes. A no-op when the state is unchanged.
    pub fn set_log_state(&self, log: LogState) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.log == log {
            return;
        }
        state.log = log;
        self.backend.set_menu(state.lang, log);
        let recording = log == LogState::Recording;
        if let Some((content, style, dot)) = state.icon.take() {
            if dot != recording {
                self.backend.set_icon(render(&content, style, recording));
            }
            state.icon = Some((content, style, recording));
        }
    }
}

/// The language to use for `language`: the setting, or the OS locale.
pub fn language_for(language: Language) -> Lang {
    resolve(language, &sys_locale::get_locale().unwrap_or_default())
}

#[derive(Clone)]
struct MenuItems {
    open: MenuItem<Wry>,
    simple: MenuItem<Wry>,
    advanced: MenuItem<Wry>,
    anti_cheat: CheckMenuItem<Wry>,
    quit: MenuItem<Wry>,
}

impl MenuItems {
    /// A menu of the shared items and fresh log items for `log`. The shared
    /// items keep their handles (the anti-cheat checkbox follows the settings
    /// store through one), and may sit in the old and the new menu at once.
    fn menu(&self, app: &AppHandle, lang: Lang, log: LogState) -> tauri::Result<Menu<Wry>> {
        let log_items = log_menu(log)
            .iter()
            .map(|item| {
                MenuItem::with_id(
                    app,
                    item.id(),
                    t(lang, item.label_key(), &[]),
                    true,
                    None::<&str>,
                )
            })
            .collect::<tauri::Result<Vec<_>>>()?;
        let separators = [
            PredefinedMenuItem::separator(app)?,
            PredefinedMenuItem::separator(app)?,
            PredefinedMenuItem::separator(app)?,
        ];
        let mut entries: Vec<&dyn IsMenuItem<Wry>> =
            vec![&self.open, &self.simple, &self.advanced, &separators[0]];
        entries.extend(log_items.iter().map(|item| item as &dyn IsMenuItem<Wry>));
        entries.extend([
            &separators[1] as &dyn IsMenuItem<Wry>,
            &self.anti_cheat,
            &separators[2],
            &self.quit,
        ]);
        Menu::with_items(app, &entries)
    }
}

/// The real tray. Every change is posted to the main thread instead of being
/// awaited, so the sampler and the settings writer never wait on it.
pub struct TauriBackend {
    app: AppHandle,
    tray: TrayIcon,
    items: MenuItems,
}

impl TrayBackend for TauriBackend {
    fn set_icon(&self, rgba: Vec<u8>) {
        let tray = self.tray.clone();
        let _ = self.app.run_on_main_thread(move || {
            let _ = tray.set_icon(Some(Image::new_owned(rgba, ICON_SIZE, ICON_SIZE)));
        });
    }

    fn set_tooltip(&self, text: String) {
        let tray = self.tray.clone();
        let _ = self.app.run_on_main_thread(move || {
            let _ = tray.set_tooltip(Some(text));
        });
    }

    fn set_menu(&self, lang: Lang, log: LogState) {
        let (app, tray, items) = (self.app.clone(), self.tray.clone(), self.items.clone());
        let _ = self.app.run_on_main_thread(move || {
            let _ = items.open.set_text(t(lang, "tray.open", &[]));
            let _ = items.simple.set_text(t(lang, "tray.viewSimple", &[]));
            let _ = items.advanced.set_text(t(lang, "tray.viewAdvanced", &[]));
            let _ = items.anti_cheat.set_text(t(lang, "tray.antiCheat", &[]));
            let _ = items.quit.set_text(t(lang, "tray.quit", &[]));
            if let Ok(menu) = items.menu(&app, lang, log) {
                let _ = tray.set_menu(Some(menu));
            }
        });
    }
}

pub type Tray = TrayController<TauriBackend>;

/// Opens the window on `view` and remembers it as the last view (P6).
fn open_view(app: &AppHandle, view: ViewKind) {
    window::show_main_on(app, view);
    app.state::<Arc<SettingsStore>>()
        .update_with(|settings| settings.view.last = Some(view));
}

/// Runs a log command of the menu on a worker thread (they wait for the
/// writer). A failure with no window open is toasted here, since the
/// coordinator does not toast what a command returns.
fn run_log_command(app: &AppHandle, item: LogMenuItem) {
    let app = app.clone();
    std::thread::spawn(move || {
        let Some(log) = app.try_state::<Arc<LogService>>() else {
            return;
        };
        let status = match item {
            LogMenuItem::Start => log.start(),
            LogMenuItem::Pause => log.pause(),
            LogMenuItem::Resume => log.resume(),
            LogMenuItem::Stop => log.stop(),
        };
        let window_open = app.get_webview_window(window::MAIN).is_some();
        let lang = language_for(
            app.state::<Arc<SettingsStore>>()
                .settings()
                .general
                .language,
        );
        if let Some((title, body)) = error_toast(lang, window_open, &status) {
            let toaster = app.state::<Arc<SystemToaster>>();
            crate::log::commands::toast_log(&toaster, title, body);
        }
    });
}

pub fn build(app: &AppHandle) -> tauri::Result<Arc<Tray>> {
    // The webview may be destroyed, so tray labels are localized in Rust.
    let lang = language_for(
        app.state::<Arc<SettingsStore>>()
            .settings()
            .general
            .language,
    );
    let open = MenuItem::with_id(app, "open", t(lang, "tray.open", &[]), true, None::<&str>)?;
    let simple = MenuItem::with_id(
        app,
        "view_simple",
        t(lang, "tray.viewSimple", &[]),
        true,
        None::<&str>,
    )?;
    let advanced = MenuItem::with_id(
        app,
        "view_advanced",
        t(lang, "tray.viewAdvanced", &[]),
        true,
        None::<&str>,
    )?;
    let initial_anti_cheat = app.state::<ServiceShell>().anti_cheat_enabled();
    let anti_cheat = CheckMenuItem::with_id(
        app,
        "anti_cheat",
        t(lang, "tray.antiCheat", &[]),
        true,
        initial_anti_cheat,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", t(lang, "tray.quit", &[]), true, None::<&str>)?;
    let items = MenuItems {
        open,
        simple,
        advanced,
        anti_cheat,
        quit,
    };
    let menu = items.menu(app, lang, LogState::Idle)?;
    // The checkbox follows the settings store (see `ToggleState`), whichever
    // way `sources.antiCheat` changes: this item, the `set_anti_cheat`
    // command or the settings view.
    app.state::<ServiceShell>().set_tray_item(
        Arc::new(items.anti_cheat.clone()) as Arc<dyn crate::service::ToggleIndicator>
    );
    let tray = TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().expect("bundle icon").clone())
        .tooltip(PRODUCT_NAME)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => window::show_main(app),
            "view_simple" => open_view(app, ViewKind::Simple),
            "view_advanced" => open_view(app, ViewKind::Advanced),
            "anti_cheat" => {
                let shell = app.state::<ServiceShell>();
                // Read here, so this can race a concurrent `set_anti_cheat`
                // command or a settings-view change: acceptable (last
                // writer wins, ruling). `set_anti_cheat` only changes the
                // store; the checkbox and the link command follow the store
                // listener, in store order, so both always end on the value
                // the store holds, never on one merely requested here.
                let enabled = !shell.anti_cheat_enabled();
                let _ = shell.set_anti_cheat(enabled);
            }
            "quit" => app.exit(0),
            id => {
                if let Some(item) = LogMenuItem::from_id(id) {
                    run_log_command(app, item);
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                window::show_main(tray.app_handle());
            }
        })
        .build(app)?;
    let backend = TauriBackend {
        app: app.clone(),
        tray,
        items,
    };
    Ok(Arc::new(TrayController::new(backend, lang)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::model::{
        Device, DeviceKind, Label, Schema, Sensor, SensorKind, Snapshot, Source, Unit,
    };
    use oma_core::rules::{Alert, Coverage, Level, OverallLevel};
    use oma_core::settings::{Settings, TemperatureUnit};
    use std::collections::BTreeMap;

    use crate::log::session::LogError;
    use crate::tray_icon::{CRIT, NEUTRAL, OK, WARN};
    use std::sync::{Arc, Mutex};

    fn device(id: &str, kind: DeviceKind, integrated: Option<bool>) -> Device {
        let mut properties = BTreeMap::new();
        if let Some(integrated) = integrated {
            properties.insert("integrated".to_owned(), integrated.to_string());
        }
        Device {
            id: id.to_owned(),
            kind,
            name: id.to_owned(),
            vendor: None,
            properties,
        }
    }

    fn sensor(device_id: &str, kind: SensorKind, name: &str, unit: Unit) -> Sensor {
        Sensor::new(
            device_id,
            kind,
            name,
            unit,
            Label::new("test"),
            Source::Mock,
        )
    }

    const CPU_TEMP: &str = "cpu/0/temperature/package";
    const CPU_LOAD: &str = "cpu/0/load/total";
    const DGPU_TEMP: &str = "gpu/pci-0000:01:00.0/temperature/core";
    const RAM_LOAD: &str = "memory/0/load/used";

    /// CPU (package temperature + load), an iGPU listed first, a dGPU, RAM.
    fn full_schema() -> Schema {
        Schema {
            revision: 7,
            devices: vec![
                device("cpu/0", DeviceKind::Cpu, None),
                device("gpu/pci-0000:11:00.0", DeviceKind::Gpu, Some(true)),
                device("gpu/pci-0000:01:00.0", DeviceKind::Gpu, Some(false)),
                device("memory/0", DeviceKind::Memory, None),
            ],
            sensors: vec![
                sensor("cpu/0", SensorKind::Temperature, "package", Unit::Celsius),
                sensor("cpu/0", SensorKind::Load, "total", Unit::Percent),
                sensor(
                    "gpu/pci-0000:11:00.0",
                    SensorKind::Temperature,
                    "core",
                    Unit::Celsius,
                ),
                sensor(
                    "gpu/pci-0000:01:00.0",
                    SensorKind::Temperature,
                    "core",
                    Unit::Celsius,
                ),
                sensor("memory/0", SensorKind::Load, "used", Unit::Percent),
            ],
        }
    }

    fn snapshot(schema: &Schema, values: &[(&str, f64)]) -> Snapshot {
        Snapshot {
            revision: schema.revision,
            seq: 1,
            timestamp_ms: 0,
            values: schema
                .sensors
                .iter()
                .map(|s| values.iter().find(|(id, _)| *id == s.id).map(|(_, v)| *v))
                .collect(),
        }
    }

    #[test]
    fn icon_sensor_prefers_the_chosen_one() {
        let schema = full_schema();
        assert_eq!(
            icon_sensor(&schema, Some(RAM_LOAD)).as_deref(),
            Some(RAM_LOAD)
        );
    }

    #[test]
    fn icon_sensor_auto_picks_the_dedicated_gpu() {
        let schema = full_schema();
        assert_eq!(icon_sensor(&schema, None).as_deref(), Some(DGPU_TEMP));
    }

    #[test]
    fn icon_sensor_falls_back_to_cpu_then_load() {
        let mut schema = full_schema();
        // Only an integrated GPU: the CPU temperature wins.
        schema.devices.remove(2);
        schema.sensors.remove(3);
        assert_eq!(icon_sensor(&schema, None).as_deref(), Some(CPU_TEMP));
        // A Ryzen reports Tctl instead of the package temperature.
        schema.sensors[0] = sensor("cpu/0", SensorKind::Temperature, "tctl", Unit::Celsius);
        assert_eq!(
            icon_sensor(&schema, None).as_deref(),
            Some("cpu/0/temperature/tctl")
        );
        // No temperature at all: total CPU load.
        schema.sensors.remove(0);
        assert_eq!(icon_sensor(&schema, None).as_deref(), Some(CPU_LOAD));
        // Nothing usable.
        schema.sensors.clear();
        assert_eq!(icon_sensor(&schema, None), None);
    }

    #[test]
    fn missing_icon_sensor_falls_back_to_auto() {
        let mut schema = full_schema();
        assert_eq!(
            icon_sensor(&schema, Some("gpu/gone/temperature/core")).as_deref(),
            Some(DGPU_TEMP)
        );
        // The chosen sensor appears later (a device is plugged in): it wins.
        schema.sensors.push(sensor(
            "gpu/gone",
            SensorKind::Temperature,
            "core",
            Unit::Celsius,
        ));
        assert_eq!(
            icon_sensor(&schema, Some("gpu/gone/temperature/core")).as_deref(),
            Some("gpu/gone/temperature/core")
        );
    }

    #[derive(Default)]
    struct Calls {
        icons: Vec<Vec<u8>>,
        tooltips: Vec<String>,
        menus: Vec<(Lang, LogState)>,
    }

    #[derive(Clone, Default)]
    struct FakeBackend(Arc<Mutex<Calls>>);

    impl TrayBackend for FakeBackend {
        fn set_icon(&self, rgba: Vec<u8>) {
            self.0.lock().unwrap().icons.push(rgba);
        }
        fn set_tooltip(&self, text: String) {
            self.0.lock().unwrap().tooltips.push(text);
        }
        fn set_menu(&self, lang: Lang, log: LogState) {
            self.0.lock().unwrap().menus.push((lang, log));
        }
    }

    #[test]
    fn update_skips_unchanged_icon_and_tooltip() {
        let schema = full_schema();
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        let settings = Settings::default();
        let values = [(CPU_TEMP, 45.0), (DGPU_TEMP, 62.0), (RAM_LOAD, 48.0)];

        let first = snapshot(&schema, &values);
        tray.update(&schema, &first, &HealthReport::default(), &settings);
        tray.update(&schema, &first, &HealthReport::default(), &settings);
        {
            let calls = backend.0.lock().unwrap();
            assert_eq!(calls.icons.len(), 1);
            assert_eq!(
                calls.tooltips,
                ["CPU 45 \u{b0}C \u{b7} GPU 62 \u{b0}C \u{b7} RAM 48 %"]
            );
            assert_eq!(
                calls.icons[0],
                render(&IconContent::Text("62".to_owned()), NEUTRAL, false)
            );
        }

        // Only the CPU changes: the icon (GPU) stays, the tooltip follows.
        let second = snapshot(
            &schema,
            &[(CPU_TEMP, 50.0), (DGPU_TEMP, 62.0), (RAM_LOAD, 48.0)],
        );
        tray.update(&schema, &second, &HealthReport::default(), &settings);
        let calls = backend.0.lock().unwrap();
        assert_eq!(calls.icons.len(), 1);
        assert_eq!(calls.tooltips.len(), 2);
    }

    #[test]
    fn update_follows_the_temperature_unit_and_the_chosen_sensor() {
        let schema = full_schema();
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        let snap = snapshot(
            &schema,
            &[(CPU_TEMP, 45.0), (DGPU_TEMP, 62.0), (RAM_LOAD, 48.0)],
        );

        let mut settings = Settings::default();
        tray.update(&schema, &snap, &HealthReport::default(), &settings);
        settings.general.temperature_unit = TemperatureUnit::F;
        settings.tray.icon_sensor = Some(RAM_LOAD.to_owned());
        tray.update(&schema, &snap, &HealthReport::default(), &settings);

        let calls = backend.0.lock().unwrap();
        assert_eq!(
            calls.icons,
            [
                render(&IconContent::Text("62".to_owned()), NEUTRAL, false),
                render(&IconContent::Bar(48), NEUTRAL, false)
            ]
        );
        assert_eq!(
            calls.tooltips[1],
            "CPU 113 \u{b0}F \u{b7} GPU 144 \u{b0}F \u{b7} RAM 48 %"
        );
    }

    #[test]
    fn update_redraws_the_icon_when_what_is_drawn_changes() {
        let schema = full_schema();
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        let mut settings = Settings::default();
        let values = |gpu: f64, ram: f64| {
            snapshot(
                &schema,
                &[(CPU_TEMP, 45.0), (DGPU_TEMP, gpu), (RAM_LOAD, ram)],
            )
        };
        let count = || backend.0.lock().unwrap().icons.len();

        // The same number 48, first as a temperature, then as a load: a re-send.
        let temperature = values(48.0, 48.0);
        tray.update(&schema, &temperature, &HealthReport::default(), &settings);
        tray.update(&schema, &temperature, &HealthReport::default(), &settings);
        assert_eq!(count(), 1);
        settings.tray.icon_sensor = Some(RAM_LOAD.to_owned());
        tray.update(&schema, &temperature, &HealthReport::default(), &settings);
        assert_eq!(count(), 2);

        // A bar re-sends when its level changes, not when the reading does not.
        tray.update(&schema, &temperature, &HealthReport::default(), &settings);
        assert_eq!(count(), 2);
        tray.update(
            &schema,
            &values(48.0, 49.0),
            &HealthReport::default(),
            &settings,
        );
        assert_eq!(count(), 3);
        tray.update(
            &schema,
            &values(90.0, 49.4),
            &HealthReport::default(),
            &settings,
        );
        assert_eq!(count(), 3, "49.4 still rounds to 49");

        let calls = backend.0.lock().unwrap();
        assert_eq!(
            calls.icons[2],
            render(&IconContent::Bar(49), NEUTRAL, false)
        );
    }

    #[test]
    fn update_ignores_a_snapshot_of_another_schema_revision() {
        let schema = full_schema();
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        let mut snap = snapshot(&schema, &[(CPU_TEMP, 45.0)]);
        snap.revision += 1;
        tray.update(
            &schema,
            &snap,
            &HealthReport::default(),
            &Settings::default(),
        );
        let calls = backend.0.lock().unwrap();
        assert!(calls.icons.is_empty() && calls.tooltips.is_empty());
    }

    #[test]
    fn relabel_changes_labels_only_on_change_and_refreshes_the_tooltip() {
        let schema = full_schema();
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        let settings = Settings::default();
        let snap = snapshot(&schema, &[(CPU_TEMP, 45.0)]);
        tray.update(&schema, &snap, &HealthReport::default(), &settings);

        tray.relabel(Lang::En);
        assert!(backend.0.lock().unwrap().menus.is_empty());
        tray.relabel(Lang::It);
        tray.relabel(Lang::It);
        assert_eq!(
            backend.0.lock().unwrap().menus,
            [(Lang::It, LogState::Idle)]
        );

        // The tooltip is rebuilt in the new language on the next tick.
        tray.update(&schema, &snap, &HealthReport::default(), &settings);
        assert_eq!(backend.0.lock().unwrap().tooltips.len(), 2);
    }

    #[test]
    fn log_menu_follows_the_state() {
        use LogMenuItem::*;
        assert_eq!(log_menu(LogState::Idle), [Start]);
        assert_eq!(log_menu(LogState::Error), [Start]);
        assert_eq!(log_menu(LogState::Recording), [Pause, Stop]);
        assert_eq!(log_menu(LogState::Paused), [Resume, Stop]);
        for item in [Start, Pause, Resume, Stop] {
            assert_eq!(LogMenuItem::from_id(item.id()), Some(item));
        }
        assert_eq!(LogMenuItem::from_id("quit"), None);
        assert_eq!(Start.id(), "log_start");
        assert_eq!(Stop.label_key(), "tray.log.stop");
    }

    #[test]
    fn menu_is_rebuilt_on_state_change_and_on_language_change() {
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        tray.set_log_state(LogState::Idle);
        assert!(backend.0.lock().unwrap().menus.is_empty(), "no change");
        tray.set_log_state(LogState::Recording);
        tray.set_log_state(LogState::Recording);
        tray.set_log_state(LogState::Paused);
        // A new language rebuilds the menu for the state the log is in.
        tray.relabel(Lang::It);
        tray.relabel(Lang::It);
        assert_eq!(
            backend.0.lock().unwrap().menus,
            [
                (Lang::En, LogState::Recording),
                (Lang::En, LogState::Paused),
                (Lang::It, LogState::Paused),
            ]
        );
    }

    #[test]
    fn icon_redraws_when_recording_flips() {
        let schema = full_schema();
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        let settings = Settings::default();
        let snap = snapshot(&schema, &HOT);
        let icon = IconContent::Text("92".to_owned());
        let count = || backend.0.lock().unwrap().icons.len();

        // Before any reading there is nothing to redraw.
        tray.set_log_state(LogState::Recording);
        assert_eq!(count(), 0);
        tray.update(&schema, &snap, &HealthReport::default(), &settings);
        assert_eq!(count(), 1);
        // Paused: the dot goes; recording again: it comes back; an error
        // after idle keeps it off.
        tray.set_log_state(LogState::Paused);
        assert_eq!(count(), 2);
        tray.set_log_state(LogState::Idle);
        assert_eq!(count(), 2, "paused and idle look the same");
        tray.set_log_state(LogState::Recording);
        assert_eq!(count(), 3);
        tray.update(&schema, &snap, &HealthReport::default(), &settings);
        assert_eq!(count(), 3, "unchanged content, style and dot");
        tray.set_log_state(LogState::Error);
        assert_eq!(count(), 4);

        let calls = backend.0.lock().unwrap();
        assert_eq!(calls.icons[0], render(&icon, NEUTRAL, true));
        assert_eq!(calls.icons[1], render(&icon, NEUTRAL, false));
        assert_eq!(calls.icons[2], render(&icon, NEUTRAL, true));
        assert_eq!(calls.icons[3], render(&icon, NEUTRAL, false));
    }

    fn status(state: LogState, error: Option<LogError>) -> LogStatus {
        LogStatus {
            revision: 1,
            state,
            session: 1,
            path: None,
            part: 1,
            part_bytes: 0,
            recorded_ms: 0,
            rows: 0,
            bytes: 0,
            dropped: 0,
            error,
            hotkeys: Default::default(),
        }
    }

    fn errored(key: &str, detail: Option<&str>) -> LogStatus {
        let error = LogError {
            key: key.to_owned(),
            detail: detail.map(str::to_owned),
        };
        status(LogState::Error, Some(error))
    }

    #[test]
    fn a_failed_tray_command_toasts_only_without_a_window() {
        let disk = errored("log.error.diskFull", None);
        assert_eq!(
            error_toast(Lang::En, false, &disk),
            Some(("Recording stopped".to_owned(), "Disk full".to_owned()))
        );
        assert_eq!(error_toast(Lang::En, true, &disk), None);
        assert_eq!(
            error_toast(Lang::It, false, &errored("log.error.other", Some("boom")))
                .map(|(_, body)| body),
            Some("Errore di scrittura: boom".to_owned())
        );
        let fine = status(LogState::Recording, None);
        assert_eq!(error_toast(Lang::En, false, &fine), None);
    }

    fn health(level: OverallLevel, alerts: Vec<Alert>) -> HealthReport {
        HealthReport {
            level,
            since_ms: 1_000,
            revision: 2,
            coverage: Coverage::Complete,
            unavailable_targets: Vec::new(),
            alerts,
        }
    }

    fn gpu_hot() -> Alert {
        Alert {
            rule_id: "gpu-temp".to_owned(),
            sensor_id: DGPU_TEMP.to_owned(),
            device_id: "gpu/pci-0000:01:00.0".to_owned(),
            unit: Unit::Celsius,
            sensor_label: Label::new("gpu.temperature.core"),
            level: Level::Crit,
            value: Some(92.0),
            threshold: Some(90.0),
            since_ms: 1_000,
            valid: true,
            last_valid_ms: Some(1_000),
            message_key: "rule.gpu-temp.message".to_owned(),
            params: BTreeMap::from([("device".to_owned(), "RTX 4080".to_owned())]),
        }
    }

    const HOT: [(&str, f64); 3] = [(CPU_TEMP, 45.0), (DGPU_TEMP, 92.0), (RAM_LOAD, 48.0)];

    #[test]
    fn icon_color_follows_the_health_level() {
        let schema = full_schema();
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        let settings = Settings::default();
        let snap = snapshot(&schema, &HOT);
        let drawn = |style| render(&IconContent::Text("92".to_owned()), style, false);

        // Only the level changes between these ticks: each change redraws.
        for level in [
            OverallLevel::Ok,
            OverallLevel::Ok,
            OverallLevel::Warn,
            OverallLevel::Crit,
            OverallLevel::Crit,
            OverallLevel::Neutral,
        ] {
            let alerts = match level {
                OverallLevel::Warn | OverallLevel::Crit => vec![gpu_hot()],
                _ => Vec::new(),
            };
            tray.update(&schema, &snap, &health(level, alerts), &settings);
        }
        let calls = backend.0.lock().unwrap();
        assert_eq!(
            calls.icons,
            [drawn(OK), drawn(WARN), drawn(CRIT), drawn(NEUTRAL)]
        );
    }

    #[test]
    fn tooltip_starts_with_the_verdict() {
        let schema = full_schema();
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        let settings = Settings::default();
        let snap = snapshot(&schema, &HOT);

        tray.update(
            &schema,
            &snap,
            &health(OverallLevel::Ok, Vec::new()),
            &settings,
        );
        let mut warn = gpu_hot();
        warn.level = Level::Warn;
        tray.update(
            &schema,
            &snap,
            &health(OverallLevel::Warn, vec![warn]),
            &settings,
        );
        tray.update(
            &schema,
            &snap,
            &health(OverallLevel::Crit, vec![gpu_hot(), gpu_hot()]),
            &settings,
        );
        let calls = backend.0.lock().unwrap();
        assert_eq!(
            calls.tooltips,
            [
                "CPU 45 °C · GPU 92 °C · RAM 48 %",
                "RTX 4080 overheating (92 °C) · CPU 45 °C · GPU 92 °C · RAM 48 %",
                "2 problems · CPU 45 °C · GPU 92 °C · RAM 48 %",
            ]
        );
    }

    #[test]
    fn language_and_units_refresh_unchanged_verdict() {
        let schema = full_schema();
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        let mut settings = Settings::default();
        let snap = snapshot(&schema, &HOT);
        // One report for the whole test: only the language and the unit change.
        let report = health(OverallLevel::Crit, vec![gpu_hot()]);

        tray.update(&schema, &snap, &report, &settings);
        tray.relabel(Lang::It);
        tray.update(&schema, &snap, &report, &settings);
        settings.general.temperature_unit = TemperatureUnit::F;
        tray.update(&schema, &snap, &report, &settings);
        let calls = backend.0.lock().unwrap();
        assert_eq!(
            calls.tooltips,
            [
                "RTX 4080 overheating (92 °C) · CPU 45 °C · GPU 92 °C · RAM 48 %",
                "RTX 4080 surriscaldata (92 °C) · CPU 45 °C · GPU 92 °C · RAM 48 %",
                "RTX 4080 surriscaldata (198 °F) · CPU 113 °F · GPU 198 °F · RAM 48 %",
            ]
        );
    }

    fn text(locale: &str, key: &str) -> String {
        t(resolve(Language::System, locale), key, &[])
    }

    #[test]
    fn italian_locales_get_italian_labels() {
        assert_eq!(text("it-IT", "tray.open"), "Apri");
        assert_eq!(text("it", "tray.quit"), "Esci");
        assert_eq!(
            text("it-IT", "tray.antiCheat"),
            "Modalità compatibile anti-cheat"
        );
    }

    #[test]
    fn other_locales_fall_back_to_english() {
        assert_eq!(text("en-US", "tray.open"), "Open");
        assert_eq!(text("de-DE", "tray.quit"), "Quit");
        assert_eq!(text("", "tray.open"), "Open");
        assert_eq!(text("en", "tray.antiCheat"), "Anti-cheat compatible mode");
    }
}
