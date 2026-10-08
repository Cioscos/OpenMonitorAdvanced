//! Tray icon: the menu (open, views, log, overlay, anti-cheat mode, quit), a dynamic icon
//! in the color of the health level (with a red dot while the log records) and
//! a tooltip led by its verdict, refreshed every tick, and the labels' language.

use std::sync::{Arc, Mutex, PoisonError};

use oma_core::load::{Component, Objective, RunState, RunStatus};
use oma_core::model::{Schema, Snapshot, Unit};
use oma_core::roles::{role_sensor, Role};
use oma_core::rules::HealthReport;
use oma_core::scores::{BenchState, BenchStatus};
use oma_core::settings::{Language, Settings, ViewKind};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::hotkeys::OverlayActions;
use crate::i18n::{resolve, t, Lang};
use crate::log::session::{LogState, LogStatus};
use crate::log::LogService;
use crate::notifier::SystemToaster;
use crate::performance::runner::PerformanceRunner;
use crate::service::ServiceShell;
use crate::settings::SettingsStore;
use crate::tray_icon::{
    icon_content, render, style_for, tooltip, verdict, IconContent, IconMarks, IconStyle,
    TooltipItem, ICON_SIZE, PRODUCT_NAME,
};
use crate::window::{self, PerformanceNav};

/// What the controller needs from the real tray, so its decisions can be
/// tested without a window. Implementations must not block the caller.
pub trait TrayBackend: Send + Sync {
    /// A 32x32 RGBA image.
    fn set_icon(&self, rgba: Vec<u8>);
    fn set_tooltip(&self, text: String);
    /// Rebuilds the menu in `lang`, with the log items of `log` and, while a
    /// stress test runs (`test`), its items.
    fn set_menu(&self, lang: Lang, log: LogState, test: bool);
    /// The overlay's «show/hide» item: clickable and ticked.
    fn set_overlay_item(&self, enabled: bool, checked: bool);
}

/// The overlay's «show/hide» check item.
pub const OVERLAY_ITEM_ID: &str = "overlay-visible";
pub const OVERLAY_ITEM_LABEL: &str = "tray.overlay.toggle";

/// A click on the menu item `id`: the overlay item hides or shows the
/// overlay (not saved, DP11). Whether `id` was the overlay item.
pub fn overlay_item_clicked(id: &str, overlay: &dyn OverlayActions) -> bool {
    if id != OVERLAY_ITEM_ID {
        return false;
    }
    overlay.toggle_hidden();
    true
}

/// The overlay editor's item, always enabled (M7d).
pub const EDITOR_ITEM_ID: &str = "overlay-editor";
pub const EDITOR_ITEM_LABEL: &str = "tray.overlay.editor";

/// A click on the menu item `id`: the editor item calls `open`. Whether `id`
/// was the editor item.
pub fn editor_item_clicked(id: &str, open: impl FnOnce()) -> bool {
    if id != EDITOR_ITEM_ID {
        return false;
    }
    open();
    true
}

/// The stress test items of the tray menu (DA17): stop it, or show it.
pub const PERF_STOP_ID: &str = "perf_stop";
pub const PERF_OPEN_ID: &str = "perf_open";

/// The menu items `(id, label key)` to show: none without a test.
pub fn test_menu(test: bool) -> &'static [(&'static str, &'static str)] {
    if test {
        &[
            (PERF_STOP_ID, "tray.performance.stop"),
            (PERF_OPEN_ID, "tray.performance.open"),
        ]
    } else {
        &[]
    }
}

/// The stress test or CPU benchmark in progress, as the tray shows it: for a
/// test the ids of the status (`cpu`, `normal`), translated when the tooltip
/// is built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestMark {
    Stress {
        component: String,
        objective: String,
    },
    /// The CPU benchmark (DB9).
    Bench,
    /// The GPU benchmark (DH12).
    GpuBench,
}

/// The page of the benchmark in progress: its GPU's, or the CPU one (DH12).
fn bench_nav(status: Option<&BenchStatus>) -> PerformanceNav {
    match status {
        Some(s) if s.category == "gpu" => s
            .device_id
            .as_deref()
            .map_or_else(PerformanceNav::score_cpu, PerformanceNav::score_gpu),
        _ => PerformanceNav::score_cpu(),
    }
}

impl TestMark {
    /// The mark for a benchmark `status`: only while it runs or stops.
    pub fn from_bench(status: &BenchStatus) -> Option<Self> {
        matches!(
            status.state,
            BenchState::Starting | BenchState::Running | BenchState::Stopping
        )
        .then_some(if status.category == "gpu" {
            Self::GpuBench
        } else {
            Self::Bench
        })
    }

    /// The mark for `status`: none while idle or once finished.
    pub fn from_status(status: &RunStatus) -> Option<Self> {
        if matches!(status.state, RunState::Idle | RunState::Finished) {
            return None;
        }
        let component = match status.component {
            Component::Cpu => "cpu",
            Component::Ram => "ram",
            Component::Gpu => "gpu",
            Component::Disk => "disk",
        };
        let objective = match status.objective {
            Objective::Normal => "normal",
            Objective::Overclock => "overclock",
        };
        Some(Self::Stress {
            component: component.to_owned(),
            objective: objective.to_owned(),
        })
    }

    /// `Stress test in progress: CPU · Normal check`, or `CPU benchmark running`.
    fn text(&self, lang: Lang) -> String {
        let (component, objective) = match self {
            Self::Stress {
                component,
                objective,
            } => (component, objective),
            Self::Bench => return t(lang, "tray.benchRunning", &[]),
            Self::GpuBench => return t(lang, "tray.gpuBenchRunning", &[]),
        };
        let component = t(lang, &format!("tray.tooltip.{component}"), &[]);
        let objective = t(lang, &format!("performance.objective.{objective}"), &[]);
        t(
            lang,
            "tray.performance.tooltip",
            &[("component", &component), ("objective", &objective)],
        )
    }
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

/// Position of the sensor playing `role`, if the schema has one.
fn role_index(schema: &Schema, role: Role) -> Option<usize> {
    role_sensor(schema, role).and_then(|id| index_of(schema, id))
}

/// The GPU temperature of a dedicated GPU. `role_sensor` falls back to an
/// integrated GPU when it is alone; the tray prefers the CPU in that case.
fn dedicated_gpu_temperature(schema: &Schema) -> Option<usize> {
    role_index(schema, Role::GpuTemperature).filter(|&index| {
        schema
            .devices
            .iter()
            .find(|device| device.id == schema.sensors[index].device_id)
            .is_some_and(|device| {
                device.properties.get("integrated").map(String::as_str) != Some("true")
            })
    })
}

fn icon_index(schema: &Schema, chosen: Option<&str>) -> Option<usize> {
    chosen
        .and_then(|id| index_of(schema, id))
        .or_else(|| dedicated_gpu_temperature(schema))
        .or_else(|| role_index(schema, Role::CpuTemperature))
        .or_else(|| role_index(schema, Role::CpuLoad))
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
            cpu: role_index(schema, Role::CpuTemperature)
                .or_else(|| role_index(schema, Role::CpuLoad)),
            gpu: dedicated_gpu_temperature(schema),
            ram: role_index(schema, Role::RamLoad),
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
    /// What the icon last sent shows, its style and its dots.
    icon: Option<(IconContent, IconStyle, IconMarks)>,
    /// The stress test in progress, if any.
    test: Option<TestMark>,
    tooltip: Option<String>,
    /// What the overlay item last got: (enabled, checked).
    overlay: Option<(bool, bool)>,
}

impl State {
    fn marks(&self) -> IconMarks {
        IconMarks {
            recording: self.log == LogState::Recording,
            testing: self.test.is_some(),
        }
    }
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
                test: None,
                tooltip: None,
                overlay: None,
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
        let marks = state.marks();
        if !state
            .icon
            .as_ref()
            .is_some_and(|(sent, sent_style, sent_marks)| {
                *sent == content && *sent_style == style && *sent_marks == marks
            })
        {
            self.backend.set_icon(render(&content, style, marks));
            state.icon = Some((content, style, marks));
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
        // The test leads the tooltip, the verdict follows.
        let verdict = match (&state.test, verdict) {
            (Some(test), Some(verdict)) => {
                Some(format!("{} \u{b7} {verdict}", test.text(state.lang)))
            }
            (Some(test), None) => Some(test.text(state.lang)),
            (None, verdict) => verdict,
        };
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
        self.backend.set_menu(lang, state.log, state.test.is_some());
    }

    /// Follows the stress test: the dot, the tooltip's lead and the menu items
    /// appear while `test` is set. A no-op when nothing changed.
    pub fn set_test(&self, test: Option<TestMark>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.test == test {
            return;
        }
        let menu_changed = state.test.is_some() != test.is_some();
        state.test = test;
        // The tooltip is rebuilt on the next tick.
        state.tooltip = None;
        if menu_changed {
            self.backend
                .set_menu(state.lang, state.log, state.test.is_some());
        }
        let marks = state.marks();
        if let Some((content, style, sent)) = state.icon.take() {
            if sent != marks {
                self.backend.set_icon(render(&content, style, marks));
            }
            state.icon = Some((content, style, marks));
        }
    }

    /// Follows the overlay: its item is clickable only while the overlay is
    /// on (`overlay.enabled`) and ticked while the user has not hidden it.
    /// A no-op when nothing changed.
    pub fn set_overlay(&self, enabled: bool, visible: bool) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.overlay == Some((enabled, visible)) {
            return;
        }
        state.overlay = Some((enabled, visible));
        self.backend.set_overlay_item(enabled, visible);
    }

    /// Follows the log: rebuilds the menu, and redraws the icon when the dot
    /// appears or goes. A no-op when the state is unchanged.
    pub fn set_log_state(&self, log: LogState) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.log == log {
            return;
        }
        state.log = log;
        self.backend.set_menu(state.lang, log, state.test.is_some());
        let marks = state.marks();
        if let Some((content, style, sent)) = state.icon.take() {
            if sent != marks {
                self.backend.set_icon(render(&content, style, marks));
            }
            state.icon = Some((content, style, marks));
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
    overlay: CheckMenuItem<Wry>,
    editor: MenuItem<Wry>,
    anti_cheat: CheckMenuItem<Wry>,
    quit: MenuItem<Wry>,
}

impl MenuItems {
    /// A menu of the shared items and fresh log items for `log`. The shared
    /// items keep their handles (the anti-cheat checkbox follows the settings
    /// store through one, the overlay item the overlay's status), and may sit
    /// in the old and the new menu at once.
    fn menu(
        &self,
        app: &AppHandle,
        lang: Lang,
        log: LogState,
        test: bool,
    ) -> tauri::Result<Menu<Wry>> {
        let test_items = test_menu(test)
            .iter()
            .map(|(id, key)| MenuItem::with_id(app, id, t(lang, key, &[]), true, None::<&str>))
            .collect::<tauri::Result<Vec<_>>>()?;
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
            &self.overlay,
            &self.editor,
            &self.anti_cheat,
        ]);
        entries.extend(test_items.iter().map(|item| item as &dyn IsMenuItem<Wry>));
        entries.extend([&separators[2] as &dyn IsMenuItem<Wry>, &self.quit]);
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

    fn set_menu(&self, lang: Lang, log: LogState, test: bool) {
        let (app, tray, items) = (self.app.clone(), self.tray.clone(), self.items.clone());
        let _ = self.app.run_on_main_thread(move || {
            let _ = items.open.set_text(t(lang, "tray.open", &[]));
            let _ = items.simple.set_text(t(lang, "tray.viewSimple", &[]));
            let _ = items.advanced.set_text(t(lang, "tray.viewAdvanced", &[]));
            let _ = items.overlay.set_text(t(lang, OVERLAY_ITEM_LABEL, &[]));
            let _ = items.editor.set_text(t(lang, EDITOR_ITEM_LABEL, &[]));
            let _ = items.anti_cheat.set_text(t(lang, "tray.antiCheat", &[]));
            let _ = items.quit.set_text(t(lang, "tray.quit", &[]));
            if let Ok(menu) = items.menu(&app, lang, log, test) {
                let _ = tray.set_menu(Some(menu));
            }
        });
    }

    fn set_overlay_item(&self, enabled: bool, checked: bool) {
        let item = self.items.overlay.clone();
        let _ = self.app.run_on_main_thread(move || {
            let _ = item.set_enabled(enabled);
            let _ = item.set_checked(checked);
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

/// The overlay item's click goes to the overlay's controller; the item's
/// tick then follows the status it publishes.
#[cfg(windows)]
fn overlay_menu_event(app: &AppHandle, id: &str) {
    if let Some(overlay) = app.try_state::<crate::overlay::runner::OverlayHandle>() {
        overlay_item_clicked(id, overlay.inner());
    }
}

/// No overlay off Windows.
#[cfg(not(windows))]
fn overlay_menu_event(_app: &AppHandle, _id: &str) {}

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
    // Shown until the overlay's status says otherwise (`set_overlay`).
    let overlay = CheckMenuItem::with_id(
        app,
        OVERLAY_ITEM_ID,
        t(lang, OVERLAY_ITEM_LABEL, &[]),
        app.state::<Arc<SettingsStore>>().settings().overlay.enabled,
        true,
        None::<&str>,
    )?;
    let editor = MenuItem::with_id(
        app,
        EDITOR_ITEM_ID,
        t(lang, EDITOR_ITEM_LABEL, &[]),
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
        overlay,
        editor,
        anti_cheat,
        quit,
    };
    let menu = items.menu(app, lang, LogState::Idle, false)?;
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
            "quit" => window::quit(app, window::QuitSource::Tray),
            PERF_STOP_ID => {
                if let Some(runner) = app.try_state::<Arc<PerformanceRunner>>() {
                    runner.stop();
                }
            }
            PERF_OPEN_ID => {
                let nav = app
                    .try_state::<Arc<PerformanceRunner>>()
                    .filter(|runner| runner.bench_running())
                    .map_or_else(PerformanceNav::run, |runner| {
                        bench_nav(runner.bench_status().as_ref())
                    });
                window::show_performance(app, nav);
            }
            id => {
                if let Some(item) = LogMenuItem::from_id(id) {
                    run_log_command(app, item);
                } else if !editor_item_clicked(id, || window::show_editor(app)) {
                    overlay_menu_event(app, id);
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
        /// Whether each menu rebuild had the stress test items.
        test_menus: Vec<bool>,
        /// (enabled, checked) of the overlay item.
        overlay: Vec<(bool, bool)>,
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
        fn set_menu(&self, lang: Lang, log: LogState, test: bool) {
            let mut calls = self.0.lock().unwrap();
            calls.menus.push((lang, log));
            calls.test_menus.push(test);
        }
        fn set_overlay_item(&self, enabled: bool, checked: bool) {
            self.0.lock().unwrap().overlay.push((enabled, checked));
        }
    }

    #[test]
    fn tray_overlay_item_disabled_when_overlay_off() {
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        // Off: greyed out, whatever the user's hide.
        tray.set_overlay(false, true);
        tray.set_overlay(false, true);
        // On and shown, then hidden by the user.
        tray.set_overlay(true, true);
        tray.set_overlay(true, false);
        tray.set_overlay(false, false);
        assert_eq!(
            backend.0.lock().unwrap().overlay,
            [(false, true), (true, true), (true, false), (false, false)]
        );
        // A rebuilt menu keeps the item: its handle is shared.
        tray.relabel(Lang::It);
        assert_eq!(backend.0.lock().unwrap().overlay.len(), 4);
        assert_eq!(OVERLAY_ITEM_ID, "overlay-visible");
        assert_eq!(text("it-IT", OVERLAY_ITEM_LABEL), "Mostra/nascondi overlay");
        assert_eq!(text("en-US", OVERLAY_ITEM_LABEL), "Show/hide overlay");
    }

    #[derive(Default)]
    struct FakeOverlay(Mutex<Vec<&'static str>>);

    impl crate::hotkeys::OverlayActions for FakeOverlay {
        fn toggle_hidden(&self) {
            self.0.lock().unwrap().push("toggle_hidden");
        }
        fn next_profile(&self) {
            self.0.lock().unwrap().push("next_profile");
        }
        fn toggle_benchmark(&self) {
            self.0.lock().unwrap().push("toggle_benchmark");
        }
        fn set_hotkeys(&self, _statuses: [crate::log::HotkeyStatus; 3]) {}
    }

    #[test]
    fn tray_overlay_item_toggles_hidden() {
        let overlay = FakeOverlay::default();
        assert!(overlay_item_clicked("overlay-visible", &overlay));
        assert_eq!(*overlay.0.lock().unwrap(), ["toggle_hidden"]);
        // Other items do not reach the overlay.
        for id in ["open", "anti_cheat", "log_start", "quit"] {
            assert!(!overlay_item_clicked(id, &overlay));
        }
        assert_eq!(overlay.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn tray_editor_item_opens_the_editor() {
        let opened = std::cell::Cell::new(0);
        assert!(editor_item_clicked("overlay-editor", || opened.set(opened.get() + 1)));
        assert_eq!(opened.get(), 1);
        for id in ["open", "overlay-visible", "log_start", "quit"] {
            assert!(!editor_item_clicked(id, || opened.set(opened.get() + 1)));
        }
        assert_eq!(opened.get(), 1);
        assert_eq!(EDITOR_ITEM_ID, "overlay-editor");
        assert_eq!(text("en-US", EDITOR_ITEM_LABEL), "Overlay editor");
        assert_eq!(text("it-IT", EDITOR_ITEM_LABEL), "Editor overlay");
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
                render(
                    &IconContent::Text("62".to_owned()),
                    NEUTRAL,
                    IconMarks::default()
                )
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
                render(
                    &IconContent::Text("62".to_owned()),
                    NEUTRAL,
                    IconMarks::default()
                ),
                render(&IconContent::Bar(48), NEUTRAL, IconMarks::default())
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
            render(&IconContent::Bar(49), NEUTRAL, IconMarks::default())
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
        assert_eq!(
            calls.icons[0],
            render(
                &icon,
                NEUTRAL,
                IconMarks {
                    recording: true,
                    testing: false
                }
            )
        );
        assert_eq!(calls.icons[1], render(&icon, NEUTRAL, IconMarks::default()));
        assert_eq!(
            calls.icons[2],
            render(
                &icon,
                NEUTRAL,
                IconMarks {
                    recording: true,
                    testing: false
                }
            )
        );
        assert_eq!(calls.icons[3], render(&icon, NEUTRAL, IconMarks::default()));
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
        let drawn = |style| {
            render(
                &IconContent::Text("92".to_owned()),
                style,
                IconMarks::default(),
            )
        };

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

    fn mark() -> TestMark {
        TestMark::Stress {
            component: "cpu".to_owned(),
            objective: "normal".to_owned(),
        }
    }

    #[test]
    fn bench_mark_leads_the_tooltip_only_while_it_runs() {
        let status = |state| BenchStatus {
            category: "cpu".into(),
            device_id: None,
            state,
            step: None,
            steps: vec![],
            segments: vec![],
            live_points: None,
            single: None,
            multi: None,
            compute: None,
            graphics: None,
            read_mbs: None,
            write_mbs: None,
            points: None,
            live_read: None,
            live_write: None,
            flags: vec![],
            score_id: None,
            error: None,
        };
        for (state, on) in [
            (BenchState::Starting, true),
            (BenchState::Running, true),
            (BenchState::Stopping, true),
            (BenchState::Done, false),
            (BenchState::Stopped, false),
            (BenchState::Failed, false),
        ] {
            assert_eq!(
                TestMark::from_bench(&status(state)).is_some(),
                on,
                "{state:?}"
            );
        }
        assert_eq!(TestMark::Bench.text(Lang::En), "CPU benchmark running");
        let gpu = BenchStatus {
            category: "gpu".into(),
            device_id: Some("gpu/0".into()),
            ..status(BenchState::Running)
        };
        assert_eq!(TestMark::from_bench(&gpu), Some(TestMark::GpuBench));
        assert_eq!(TestMark::GpuBench.text(Lang::En), "GPU benchmark running");
        assert_eq!(
            TestMark::GpuBench.text(Lang::It),
            "Benchmark della GPU in corso"
        );
        assert_eq!(bench_nav(Some(&gpu)), PerformanceNav::score_gpu("gpu/0"));
        assert_eq!(
            bench_nav(Some(&status(BenchState::Running))),
            PerformanceNav::score_cpu()
        );
        assert_eq!(
            TestMark::Bench.text(Lang::It),
            "Benchmark della CPU in corso"
        );
    }

    #[test]
    fn test_menu_items_only_while_running() {
        assert!(test_menu(false).is_empty());
        let ids: Vec<_> = test_menu(true).iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [PERF_STOP_ID, PERF_OPEN_ID]);
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        tray.set_test(Some(mark()));
        tray.set_test(Some(mark()));
        tray.set_test(None);
        assert_eq!(backend.0.lock().unwrap().test_menus, [true, false]);
    }

    #[test]
    fn test_mark_redraws_the_icon_and_leads_the_tooltip() {
        let schema = full_schema();
        let backend = FakeBackend::default();
        let tray = TrayController::new(backend.clone(), Lang::En);
        let settings = Settings::default();
        let snap = snapshot(&schema, &HOT);
        tray.update(&schema, &snap, &HealthReport::default(), &settings);
        tray.set_test(Some(mark()));
        tray.update(&schema, &snap, &HealthReport::default(), &settings);
        let calls = backend.0.lock().unwrap();
        assert_eq!(calls.icons.len(), 2);
        assert_ne!(calls.icons[0], calls.icons[1]);
        let tooltip = calls.tooltips.last().unwrap();
        assert!(
            tooltip.starts_with("Stress test running: CPU \u{b7} Normal check"),
            "{tooltip}"
        );
    }
}
