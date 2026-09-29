//! Tray icon: the menu (open, views, anti-cheat mode, quit), a dynamic icon and
//! tooltip refreshed every tick, and the labels' language.

use std::sync::{Arc, Mutex, PoisonError};

use oma_core::model::{DeviceKind, Schema, Snapshot, Unit};
use oma_core::settings::{Language, Settings, ViewKind};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::i18n::{resolve, t, Lang};
use crate::service::ServiceShell;
use crate::settings::SettingsStore;
use crate::tray_icon::{
    icon_content, render, tooltip, IconContent, IconStyle, TooltipItem, ICON_SIZE, NEUTRAL,
    PRODUCT_NAME,
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
    /// Rewrites the menu labels in `lang`.
    fn set_labels(&self, lang: Lang);
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
    /// What the icon last sent shows, and its style.
    icon: Option<(IconContent, IconStyle)>,
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
                resolved: None,
                icon: None,
                tooltip: None,
            }),
        }
    }

    /// Called every tick. A snapshot of another schema revision is skipped: the
    /// next tick brings the matching pair.
    pub fn update(&self, schema: &Schema, snapshot: &Snapshot, settings: &Settings) {
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
        let style = NEUTRAL;
        if !state
            .icon
            .as_ref()
            .is_some_and(|(sent, sent_style)| *sent == content && *sent_style == style)
        {
            self.backend.set_icon(render(&content, style));
            state.icon = Some((content, style));
        }

        let item = |label_key, index| {
            let (value, unit) = reading(index);
            TooltipItem {
                label_key,
                value,
                unit,
            }
        };
        let text = tooltip(
            state.lang,
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
        self.backend.set_labels(lang);
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

    fn set_labels(&self, lang: Lang) {
        let items = self.items.clone();
        let _ = self.app.run_on_main_thread(move || {
            let _ = items.open.set_text(t(lang, "tray.open", &[]));
            let _ = items.simple.set_text(t(lang, "tray.viewSimple", &[]));
            let _ = items.advanced.set_text(t(lang, "tray.viewAdvanced", &[]));
            let _ = items.anti_cheat.set_text(t(lang, "tray.antiCheat", &[]));
            let _ = items.quit.set_text(t(lang, "tray.quit", &[]));
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
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &simple,
            &advanced,
            &PredefinedMenuItem::separator(app)?,
            &anti_cheat,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    // The checkbox follows the settings store (see `ToggleState`), whichever
    // way `sources.antiCheat` changes: this item, the `set_anti_cheat`
    // command or the settings view.
    app.state::<ServiceShell>()
        .set_tray_item(Arc::new(anti_cheat.clone()) as Arc<dyn crate::service::ToggleIndicator>);
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
            _ => {}
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
        items: MenuItems {
            open,
            simple,
            advanced,
            anti_cheat,
            quit,
        },
    };
    Ok(Arc::new(TrayController::new(backend, lang)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::model::{
        Device, DeviceKind, Label, Schema, Sensor, SensorKind, Snapshot, Source, Unit,
    };
    use oma_core::settings::{Settings, TemperatureUnit};
    use std::collections::BTreeMap;
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
        labels: Vec<Lang>,
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
        fn set_labels(&self, lang: Lang) {
            self.0.lock().unwrap().labels.push(lang);
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
        tray.update(&schema, &first, &settings);
        tray.update(&schema, &first, &settings);
        {
            let calls = backend.0.lock().unwrap();
            assert_eq!(calls.icons.len(), 1);
            assert_eq!(
                calls.tooltips,
                ["CPU 45 \u{b0}C \u{b7} GPU 62 \u{b0}C \u{b7} RAM 48 %"]
            );
            assert_eq!(
                calls.icons[0],
                render(&IconContent::Text("62".to_owned()), NEUTRAL)
            );
        }

        // Only the CPU changes: the icon (GPU) stays, the tooltip follows.
        let second = snapshot(
            &schema,
            &[(CPU_TEMP, 50.0), (DGPU_TEMP, 62.0), (RAM_LOAD, 48.0)],
        );
        tray.update(&schema, &second, &settings);
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
        tray.update(&schema, &snap, &settings);
        settings.general.temperature_unit = TemperatureUnit::F;
        settings.tray.icon_sensor = Some(RAM_LOAD.to_owned());
        tray.update(&schema, &snap, &settings);

        let calls = backend.0.lock().unwrap();
        assert_eq!(
            calls.icons,
            [
                render(&IconContent::Text("62".to_owned()), NEUTRAL),
                render(&IconContent::Bar(48), NEUTRAL)
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
        tray.update(&schema, &temperature, &settings);
        tray.update(&schema, &temperature, &settings);
        assert_eq!(count(), 1);
        settings.tray.icon_sensor = Some(RAM_LOAD.to_owned());
        tray.update(&schema, &temperature, &settings);
        assert_eq!(count(), 2);

        // A bar re-sends when its level changes, not when the reading does not.
        tray.update(&schema, &temperature, &settings);
        assert_eq!(count(), 2);
        tray.update(&schema, &values(48.0, 49.0), &settings);
        assert_eq!(count(), 3);
        tray.update(&schema, &values(90.0, 49.4), &settings);
        assert_eq!(count(), 3, "49.4 still rounds to 49");

        let calls = backend.0.lock().unwrap();
        assert_eq!(calls.icons[2], render(&IconContent::Bar(49), NEUTRAL));
    }

    #[test]
    fn update_ignores_a_snapshot_of_another_schema_revision() {
        let schema = full_schema();
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        let mut snap = snapshot(&schema, &[(CPU_TEMP, 45.0)]);
        snap.revision += 1;
        tray.update(&schema, &snap, &Settings::default());
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
        tray.update(&schema, &snap, &settings);

        tray.relabel(Lang::En);
        assert!(backend.0.lock().unwrap().labels.is_empty());
        tray.relabel(Lang::It);
        tray.relabel(Lang::It);
        assert_eq!(backend.0.lock().unwrap().labels, [Lang::It]);

        // The tooltip is rebuilt in the new language on the next tick.
        tray.update(&schema, &snap, &settings);
        assert_eq!(backend.0.lock().unwrap().tooltips.len(), 2);
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
