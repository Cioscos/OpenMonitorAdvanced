#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod commands;
mod hotkeys;
mod i18n;
mod interval;
mod log;
mod notifier;
mod report;
mod rules;
mod service;
mod settings;
mod tray;
mod tray_icon;
mod updates;
mod window;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use oma_core::engine::Engine;
use oma_core::provider::Provider;
use oma_core::sampler::{history_capacity, sample_interval, IntervalHandle, Sampler};
use tauri::{Emitter, Manager, RunEvent};

use crate::commands::{
    follow_vendor_libraries, quality_codes, vendor_mask, DiskStateTable, GpuProcessState,
    GpuProcessTable, SnapshotEvent, StartupState, StartupStatus, VendorSwitch, EVENT_DISK_STATES,
};
use crate::service::ServiceShell;
use crate::settings::{RealFs, SettingsStore, EVENT_SETTINGS};

const EVENT_SCHEMA: &str = "oma:schema";
const EVENT_SNAPSHOT: &str = "oma:snapshot";

pub struct AppState {
    pub engine: Arc<Mutex<Engine>>,
    /// The live sampling interval, reported by `get_session`.
    pub interval: IntervalHandle,
    /// The power state of each disk, published by the storage provider.
    pub disk_states: DiskStateTable,
}

/// Whether `generation` differs from the last one seen, which it then becomes.
fn generation_changed(last: &mut u64, generation: u64) -> bool {
    let changed = generation != *last;
    *last = generation;
    changed
}

/// Owns the sampler so it can be stopped cleanly on exit.
struct SamplerGuard(Mutex<Option<Sampler>>);

/// What a second launch asks of the running instance.
#[derive(Debug, PartialEq, Eq)]
enum SecondLaunch {
    /// `--quit`: exit the running instance (the installer's way to close it).
    Quit,
    /// A plain or `--safe` launch: bring the window up.
    ShowWindow,
    /// `--minimized` (autostart racing a running instance): stay quiet.
    Nothing,
}

/// Whether the arguments carry `--quit`, as a whole argument.
fn is_quit(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--quit")
}

/// What a second launch with these arguments asks for. `--quit` wins over
/// `--minimized`. A second `--minimized` launch stays quiet; one without it,
/// like `measure-footprint.ps1`'s, opens the window.
fn second_launch(args: &[String]) -> SecondLaunch {
    if is_quit(args) {
        SecondLaunch::Quit
    } else if args.iter().any(|arg| arg == "--minimized") {
        SecondLaunch::Nothing
    } else {
        SecondLaunch::ShowWindow
    }
}

/// `--quit` with no instance running: only the single-instance plugin is
/// built, so a running peer still gets the arguments forwarded (and this
/// process ends inside the plugin), but nothing else starts. No log file, no
/// crash marker, no settings, no window, tray, sampler, service link or
/// autostart write; the app asks to exit as soon as the event loop runs.
fn run_quit_only() -> ! {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if is_quit(&args) {
                app.exit(0);
            }
        }))
        .setup(|app| {
            app.handle().exit(0);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Tauri application");
    std::process::exit(app.run_return(|_, _| {}));
}

/// Whether closing the last window keeps the app running in the tray.
fn keep_running_on_last_close(close_to_tray: bool) -> bool {
    close_to_tray
}

#[cfg(windows)]
fn providers(
    vendor: VendorSwitch,
    processes: GpuProcessTable,
    service: oma_win::ServiceHandles,
) -> Vec<Box<dyn Provider>> {
    oma_win::default_providers(vendor, processes, service)
}

#[cfg(not(windows))]
fn providers(vendor: VendorSwitch, processes: GpuProcessTable) -> Vec<Box<dyn Provider>> {
    let _ = (vendor, processes);
    Vec::new()
}

/// `%LOCALAPPDATA%\OpenMonitorAdvanced\crash.txt`; `None` without LOCALAPPDATA.
#[cfg(windows)]
fn crash_marker_path() -> Option<std::path::PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    Some(
        std::path::PathBuf::from(local_app_data)
            .join("OpenMonitorAdvanced")
            .join("crash.txt"),
    )
}

/// Returns (and deletes) the crash marker of the previous run, then arms the
/// marker for this run so a native crash (e.g. inside a GPU vendor DLL) puts
/// the next start in safe mode (spec §8).
fn previous_crash() -> Option<String> {
    #[cfg(windows)]
    {
        let path = crash_marker_path()?;
        let crash = oma_win::crash::take_crash_marker(&path);
        if let Some(dir) = path.parent() {
            // The exception filter cannot create folders while the process dies.
            if let Err(err) = std::fs::create_dir_all(dir) {
                tracing::warn!(%err, "cannot create the crash marker folder");
            }
        }
        oma_win::crash::install_crash_marker(path);
        crash
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// `%LOCALAPPDATA%\OpenMonitorAdvanced\logs`; `None` without LOCALAPPDATA.
pub(crate) fn logs_dir() -> Option<std::path::PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    Some(
        std::path::PathBuf::from(local_app_data)
            .join("OpenMonitorAdvanced")
            .join("logs"),
    )
}

/// Sets up file logging under [`logs_dir`]. In a
/// release build the app has no console (windows subsystem), so a panic here
/// would fail silently; missing `LOCALAPPDATA` or a rolling appender that
/// cannot be built just leaves the app without a file log instead.
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let logs = logs_dir()?;
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("oma-app")
        .max_log_files(7)
        .build(logs)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(writer)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "oma_core=debug,oma_win=info,oma_app=info".into()),
        )
        .init();
    Some(guard)
}

fn main() {
    // Before anything with side effects (see `run_quit_only`).
    if is_quit(&std::env::args().collect::<Vec<_>>()) {
        run_quit_only();
    }
    // Held for the program's lifetime when present, so buffered log lines are
    // flushed on drop; the app still runs (without a file log) if this is None.
    let log_guard = init_logging();

    let start_minimized = std::env::args().any(|arg| arg == "--minimized");
    let safe_flag = std::env::args().any(|arg| arg == "--safe");
    let crash = previous_crash();
    if let Some(marker) = &crash {
        tracing::warn!(%marker, "the previous run crashed");
    }
    let status = StartupStatus::at_startup(safe_flag, crash.as_deref());
    if status.safe_mode {
        tracing::warn!(reason = ?status.reason, "safe mode: GPU vendor libraries are not loaded");
    }
    let processes = GpuProcessTable::new();

    #[cfg(windows)]
    let (svc_feed, svc_drives, svc_status) = (
        oma_win::svc::SvcFeed::default(),
        oma_win::storage::DriveIdTable::default(),
        oma_win::svc::ServiceStatusTable::default(),
    );
    let disk_states = DiskStateTable::default();

    // Opened before anything reads a preference, so the tray, the sampler and
    // the UI commands all see the same settings from the first moment. The
    // M4 anti-cheat file is folded in before the service shell reads the flag.
    let settings_fs = Arc::new(RealFs);
    let settings_path = settings::settings_path();
    let settings_file_existed = settings_path.as_deref().is_some_and(|path| path.exists());
    let settings_store = Arc::new(SettingsStore::open(settings_path, settings_fs.clone()));
    settings::migrate::migrate_service_v1(
        &settings_store,
        settings_fs.as_ref(),
        service::anti_cheat_path().as_deref(),
        settings_file_existed,
    );

    // Safe mode is the master of the vendor-library switches; the per-library
    // switches start from the stored settings and follow every later change
    // (a library already loaded stays loaded, D1).
    let initial_libraries = vendor_mask(&settings_store.settings().sources.vendor_libraries);
    let switch = VendorSwitch::new(!status.safe_mode, initial_libraries);
    follow_vendor_libraries(&settings_store, switch.clone(), initial_libraries);

    // The sampling interval starts from the stored setting (the default is 1 s);
    // a value outside the accepted range cannot come out of the store, but the
    // default is the safe fallback.
    let initial_interval =
        sample_interval(u64::from(settings_store.settings().general.interval_ms))
            .unwrap_or(Duration::from_secs(1));
    let interval = IntervalHandle::new(initial_interval);

    #[cfg(windows)]
    let engine = Arc::new(Mutex::new(Engine::new(
        providers(
            switch.clone(),
            processes.clone(),
            oma_win::ServiceHandles {
                feed: svc_feed.clone(),
                drives: svc_drives.clone(),
                disk_states: disk_states.clone(),
            },
        ),
        history_capacity(initial_interval),
    )));
    #[cfg(not(windows))]
    let engine = Arc::new(Mutex::new(Engine::new(
        providers(switch.clone(), processes.clone()),
        history_capacity(initial_interval),
    )));

    // The link to the service starts in `.setup()` below, not here.
    #[cfg(windows)]
    let service_shell = ServiceShell::new(settings_store.clone(), svc_status.clone());
    #[cfg(not(windows))]
    let service_shell = ServiceShell::new(settings_store.clone());

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            |app, args, _cwd| match second_launch(&args) {
                SecondLaunch::Quit => app.exit(0),
                SecondLaunch::ShowWindow => window::show_main(app),
                SecondLaunch::Nothing => {}
            },
        ))
        // Rust side only: no `global-shortcut:*` permission reaches the UI.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(window::NavState::default())
        .manage(AppState {
            engine: engine.clone(),
            interval: interval.clone(),
            disk_states: disk_states.clone(),
        })
        .manage(StartupState::new(switch, status))
        .manage(GpuProcessState(processes))
        .manage(service_shell)
        .manage(settings_store)
        .manage(report::ReportState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_schema,
            commands::get_history,
            commands::get_stats,
            commands::reset_stats,
            commands::get_session,
            commands::get_disk_states,
            commands::get_gpu_processes,
            commands::get_startup_status,
            commands::take_pending_view,
            commands::enable_vendor_libraries,
            commands::get_app_info,
            commands::open_known_path,
            commands::get_health,
            commands::get_rule_status,
            commands::get_default_rules,
            commands::get_health_clock,
            autostart::refresh_autostart,
            service::get_service_status,
            service::set_anti_cheat,
            service::start_service,
            settings::commands::get_settings,
            settings::commands::update_settings,
            settings::commands::reset_rule_override,
            settings::commands::import_webview_state,
            log::commands::log_start,
            log::commands::log_pause,
            log::commands::log_resume,
            log::commands::log_stop,
            log::commands::get_log_status,
            log::commands::open_log_folder,
            log::commands::pick_log_folder,
            hotkeys::set_log_hotkeys_suspended,
            updates::check_updates,
            updates::get_update_status,
            updates::open_release_page,
            report::export_sensor_report,
            report::reveal_sensor_report,
        ])
        .setup(move |app| {
            // Only the surviving instance gets here: a second launch has
            // already exited in the single-instance plugin while the app was
            // being built, so it never probes, starts or connects to the
            // service (final review M2).
            #[cfg(windows)]
            app.state::<ServiceShell>().spawn_link(svc_feed, svc_drives);
            // From here on, `rules` drives the rule engine; the stored rules
            // are installed now, before the first tick.
            rules::install_rules(app.state::<Arc<SettingsStore>>().inner(), engine.clone());
            // From here on, `general.intervalMs` drives history size, rule
            // engine, sampler and link.
            interval::follow_interval(
                app.state::<Arc<SettingsStore>>().inner(),
                engine.clone(),
                interval.clone(),
                app.state::<ServiceShell>().interval_sink(),
            );
            // Keeps an open window aligned after every settings change, whatever its origin.
            let settings_handle = app.handle().clone();
            app.state::<Arc<SettingsStore>>()
                .subscribe(Box::new(move |_, state| {
                    if settings_handle.get_webview_window(window::MAIN).is_some() {
                        let _ = settings_handle.emit(EVENT_SETTINGS, state);
                    }
                }));
            let store = app.state::<Arc<SettingsStore>>().inner().clone();
            // `tray.autostart` drives the user's Run entry from here on.
            app.manage(autostart::Autostart::follow(
                &store,
                autostart::system_entry()?,
            ));
            let tray = tray::build(app.handle())?;
            // Menu labels follow `general.language`. Listeners get only the new
            // state, so the last language seen is kept here and acted on when it
            // changes; relabelling is posted to the main thread and never blocks.
            let relabel_tray = tray.clone();
            let last_language = Mutex::new(store.settings().general.language);
            store.subscribe(Box::new(move |settings, _| {
                let language = settings.general.language;
                let mut last = last_language.lock().unwrap_or_else(PoisonError::into_inner);
                if *last != language {
                    *last = language;
                    relabel_tray.relabel(tray::language_for(language));
                }
            }));
            if !start_minimized {
                window::show_main(app.handle());
            }
            // The window stack (tao, WebView2) may have replaced the crash marker filter.
            #[cfg(windows)]
            oma_win::crash::rearm_crash_marker();
            // One toaster for the rule alerts and the log (L8); the hotkeys
            // fetch it from the managed state.
            let toaster = Arc::new(notifier::system_toaster(app.handle()));
            app.manage(toaster.clone());
            // Update checks: on request, or daily with `updates.checkAutomatically`.
            app.manage(updates::install(app.handle(), &store, toaster.clone()));
            let log_service = log::LogService::new(
                store.clone(),
                Arc::new(log::fs::RealFs),
                Arc::new(log::commands::TauriEnv::new(
                    app.handle().clone(),
                    toaster.clone(),
                )),
                log::CLOSE_TIMEOUT,
            );
            app.manage(log_service.clone());
            // `log.hotkeyToggle` and `log.hotkeyPause` drive the global hotkeys.
            hotkeys::install_hotkeys(app.handle(), &store, log_service.clone());
            // Listeners run on whichever thread changed the state; the tray
            // posts its own work to the main thread.
            let log_tray = tray.clone();
            log_service.on_state_change(Box::new(move |state| log_tray.set_log_state(state)));
            let handle = app.handle().clone();
            #[cfg(windows)]
            let mut last_service_version = 0u64;
            let mut last_disk_generation = 0u64;
            // The tray and the toasts follow every tick, window or not. One
            // feed (and one toast cooldown) lives for the whole session; it
            // keeps the latest schema and health report, which arrive only
            // when they change.
            let mut alerts = notifier::AlertFeed::new(toaster);
            let clock_engine = engine.clone();
            let mut clock_pacer = rules::ClockPacer::default();
            let sampler = Sampler::spawn(engine.clone(), interval.clone(), move |out| {
                let settings = store.snapshot();
                alerts.tick(out, &settings);
                if let Some(schema) = alerts.schema() {
                    tray.update(schema, &out.snapshot, alerts.health(), &settings);
                    // The log row, window or not; never waits on the writer.
                    log_service.on_tick(out, schema, &settings);
                }
                // Nobody listens while the window is closed: skip serialization.
                if handle.get_webview_window(window::MAIN).is_none() {
                    return;
                }
                if let Some(schema) = &out.schema {
                    let _ = handle.emit(EVENT_SCHEMA, schema);
                }
                let _ = handle.emit(
                    EVENT_SNAPSHOT,
                    SnapshotEvent {
                        snapshot: &out.snapshot,
                        quality: quality_codes(&out.quality),
                    },
                );
                // Always the full list: an empty one revokes the earlier states.
                let (generation, states) = disk_states.get();
                if generation_changed(&mut last_disk_generation, generation) {
                    let _ = handle.emit(EVENT_DISK_STATES, &commands::disk_state_entries(states));
                }
                if let Some(health) = &out.health {
                    let _ = handle.emit(rules::EVENT_HEALTH, health);
                }
                // The tick has released the engine; the clock is read under
                // a short lock of its own.
                let clock = clock_engine
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .health_clock();
                if clock_pacer.take(clock) {
                    let _ = handle.emit(rules::EVENT_HEALTH_CLOCK, clock);
                }
                #[cfg(windows)]
                {
                    // The status is copied only when it changed.
                    if svc_status.version() != last_service_version {
                        let (version, status) = svc_status.get();
                        last_service_version = version;
                        let _ = handle.emit(service::EVENT_SERVICE, &status);
                    }
                }
            });
            app.manage(SamplerGuard(Mutex::new(Some(sampler))));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Tauri application");

    // `run_return` (not `run`, which ends the process itself) so the log guard
    // below is dropped, and the last buffered log lines written, before exit.
    let exit_code = app.run_return(|app, event| match event {
        // Last window closed: keep sampling in the tray unless the user turned
        // close-to-tray off. Explicit exits carry a code and are not stopped.
        RunEvent::ExitRequested {
            code: None, api, ..
        } => {
            let close_to_tray = app
                .try_state::<Arc<SettingsStore>>()
                .is_none_or(|store| store.snapshot().tray.close_to_tray);
            if keep_running_on_last_close(close_to_tray) {
                api.prevent_exit();
            }
        }
        RunEvent::Exit => {
            if let Some(guard) = app.try_state::<SamplerGuard>() {
                if let Some(sampler) = guard
                    .0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
                {
                    sampler.stop();
                }
            }
            // No tick is running any more: stop the CSV log (bounded, L6).
            if let Some(log) = app.try_state::<Arc<log::LogService>>() {
                log.shutdown(log::CLOSE_TIMEOUT);
            }
            // Final save (bounded) before the service link goes away.
            if let Some(store) = app.try_state::<Arc<SettingsStore>>() {
                if let Err(reason) = store.shutdown(Duration::from_secs(2)) {
                    tracing::error!(%reason, "the settings could not be saved on exit");
                }
            }
            if let Some(shell) = app.try_state::<ServiceShell>() {
                shell.shutdown();
            }
        }
        _ => {}
    });
    drop(log_guard);
    std::process::exit(exit_code);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command in `generate_handler!` must be listed in the build
    /// manifest and allowed by the main window capability, or the UI gets
    /// "not allowed. Command not found" at run time.
    #[test]
    fn every_registered_command_is_in_the_manifest_and_the_capability() {
        let main_src = include_str!("main.rs");
        let build_src = include_str!("../build.rs");
        let capability = include_str!("../capabilities/default.json");

        // The first occurrence is the real invocation, not this test.
        let start = main_src
            .find("generate_handler![")
            .expect("generate_handler!")
            + 18;
        let end = start + main_src[start..].find(']').expect("closing bracket");
        let commands: Vec<&str> = main_src[start..end]
            .split(',')
            .map(|entry| entry.trim().rsplit("::").next().unwrap())
            .filter(|name| !name.is_empty())
            .collect();
        assert!(commands.len() > 20, "parsed too few commands: {commands:?}");

        let mut missing = Vec::new();
        for name in commands {
            if !build_src.contains(&format!("\"{name}\"")) {
                missing.push(format!("{name}: missing from build.rs"));
            }
            let permission = format!("\"allow-{}\"", name.replace('_', "-"));
            if !capability.contains(&permission) {
                missing.push(format!("{name}: missing from capabilities/default.json"));
            }
        }
        assert!(missing.is_empty(), "{missing:#?}");
    }

    #[test]
    fn generation_changed_reports_each_new_generation_once() {
        let mut last = 0;
        assert!(!generation_changed(&mut last, 0));
        assert!(generation_changed(&mut last, 1));
        assert!(!generation_changed(&mut last, 1));
        assert!(generation_changed(&mut last, 2));
        assert_eq!(last, 2);
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn second_launch_decides_quit_show_or_nothing() {
        assert_eq!(
            second_launch(&args(&["oma-app.exe", "--quit"])),
            SecondLaunch::Quit
        );
        assert_eq!(
            second_launch(&args(&["oma-app.exe", "--minimized", "--quit"])),
            SecondLaunch::Quit
        );
        assert_eq!(
            second_launch(&args(&["oma-app.exe", "--minimized"])),
            SecondLaunch::Nothing
        );
        assert_eq!(
            second_launch(&args(&["oma-app.exe"])),
            SecondLaunch::ShowWindow
        );
        assert_eq!(
            second_launch(&args(&["oma-app.exe", "--safe"])),
            SecondLaunch::ShowWindow
        );
    }

    #[test]
    fn quit_is_recognised_only_as_a_whole_argument() {
        assert!(is_quit(&args(&["oma-app.exe", "--quit"])));
        assert!(!is_quit(&args(&["oma-app.exe", "--quitter"])));
        assert!(!is_quit(&args(&["oma-app.exe"])));
    }

    #[test]
    fn last_close_exits_when_close_to_tray_is_off() {
        assert!(keep_running_on_last_close(true));
        assert!(!keep_running_on_last_close(false));
    }
}
