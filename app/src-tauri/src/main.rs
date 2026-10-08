#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod commands;
mod hotkeys;
mod i18n;
mod interval;
mod log;
mod notifier;
#[cfg_attr(not(windows), allow(dead_code))]
mod overlay;
mod performance;
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

/// Owns the overlay controller's thread so it is stopped, in order, on exit.
#[cfg(windows)]
struct OverlayGuard(Mutex<Option<overlay::runner::OverlayRunner>>);

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
fn run_quit_only(context: tauri::Context<tauri::Wry>) -> ! {
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
        .build(context)
        .expect("failed to build the Tauri application");
    std::process::exit(app.run_return(|_, _| {}));
}

/// Whether closing the last window keeps the app running in the tray: when
/// the user wants it, and always while a stress test runs (DA16).
fn keep_running_on_last_close(close_to_tray: bool, test_running: bool) -> bool {
    close_to_tray || test_running
}

/// Whether the toast «the test keeps running in the tray» was shown for the
/// test in progress.
static CLOSE_TOAST_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True once per test: the first close of the last window while it runs.
fn close_toast_due(test_running: bool, shown: &std::sync::atomic::AtomicBool) -> bool {
    test_running && !shown.swap(true, std::sync::atomic::Ordering::Relaxed)
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
    // Built once (it embeds the frontend assets) for whichever path runs; no
    // side effects.
    let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
    // Before anything with side effects (see `run_quit_only`).
    if is_quit(&std::env::args().collect::<Vec<_>>()) {
        run_quit_only(context);
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
    // The disk tests read the drive and power tables too (DC6).
    #[cfg(windows)]
    let (perf_drives, perf_disk_states) = (svc_drives.clone(), disk_states.clone());

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
                SecondLaunch::Quit => window::quit(app, window::QuitSource::Flag),
                SecondLaunch::ShowWindow => {
                    tracing::info!(?args, "second launch: showing the window");
                    window::show_main(app)
                }
                SecondLaunch::Nothing => {}
            },
        ))
        // Rust side only: no `global-shortcut:*` permission reaches the UI.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .manage(window::NavState::default())
        .manage(window::EditorState::default())
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
            overlay::runner::get_overlay_status,
            overlay::runner::overlay_retry,
            overlay::runner::overlay_reload_profiles,
            overlay::runner::set_overlay_hidden,
            overlay::editor::overlay_load_profile,
            overlay::editor::overlay_save_profile,
            overlay::editor::overlay_delete_profile,
            overlay::editor::overlay_duplicate_profile,
            overlay::editor::overlay_import_profile,
            overlay::editor::overlay_export_profile,
            overlay::editor::overlay_font_families,
            overlay::runner::overlay_preview,
            overlay::runner::overlay_use_now,
            overlay::runner::overlay_editor_profile,
            overlay::runner::benchmark_toggle,
            overlay::benchmark::benchmark_list,
            overlay::benchmark::benchmark_open_csv,
            overlay::benchmark::benchmark_open_folder,
            overlay::benchmark::benchmark_delete,
            window::open_overlay_editor,
            window::overlay_editor_dirty,
            window::app_quit_confirmed,
            performance::commands::performance_system,
            performance::commands::performance_preview,
            performance::commands::performance_start,
            performance::commands::performance_stop,
            performance::commands::performance_status,
            performance::commands::performance_history,
            performance::commands::performance_session,
            performance::commands::performance_delete,
            performance::commands::performance_export,
            performance::commands::performance_quit_confirmed,
            performance::commands::performance_bench_start,
            performance::commands::performance_gpu_bench_start,
            performance::commands::performance_disk_bench_start,
            performance::commands::performance_disk_probe,
            performance::commands::performance_disk_pick,
            performance::commands::performance_bench_stop,
            performance::commands::performance_bench_status,
            performance::commands::performance_scores,
            performance::commands::performance_score,
            performance::commands::performance_score_delete,
            performance::commands::performance_baseline,
        ])
        .setup(move |app| {
            // Only the surviving instance gets here: a second launch has
            // already exited in the single-instance plugin while the app was
            // being built, so it never probes, starts or connects to the
            // service (final review M2).
            #[cfg(windows)]
            {
                let shell = app.state::<ServiceShell>();
                shell.spawn_link(svc_feed, svc_drives);
            }
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
                    if window::any_open(&settings_handle) {
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
            // The stress test runner: without a test it does no periodic work (§11).
            #[cfg(windows)]
            let perf = {
                let engine = engine.clone();
                let svc = svc_status.clone();
                let (open_handle, emit_handle) = (app.handle().clone(), app.handle().clone());
                let (visible_handle, bench_handle) = (app.handle().clone(), app.handle().clone());
                let (state_tray, bench_tray) = (tray.clone(), tray.clone());
                let runner = Arc::new(performance::runner::PerformanceRunner::new(
                    performance::runner::RunnerDeps {
                        store: Arc::new(performance::store::PerformanceStore::new(
                            performance::performance_dir(),
                        )),
                        settings: store.clone(),
                        machine: Box::new(performance::runner::WinMachine::new(
                            app.state::<GpuProcessState>().0.clone(),
                            perf_drives,
                            perf_disk_states,
                        )),
                        launcher: performance::runner::load_host_launcher(),
                        toaster: Box::new(toaster.clone()),
                        schema: Box::new(move || {
                            engine
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .schema()
                                .clone()
                        }),
                        service_available: Box::new(move || {
                            svc.get().1.state == oma_ipc::ServiceState::Connected
                        }),
                        // Only the main window has the Performance view.
                        window_open: Box::new(move || {
                            open_handle.get_webview_window(window::MAIN).is_some()
                        }),
                        window_visible: Box::new(move || {
                            visible_handle
                                .get_webview_window(window::MAIN)
                                .is_some_and(|w| {
                                    w.is_visible().unwrap_or(false)
                                        && !w.is_minimized().unwrap_or(false)
                                })
                        }),
                        emit: Box::new(move |status| {
                            let _ = emit_handle.emit_to(
                                window::MAIN,
                                performance::runner::EVENT_STATUS,
                                status,
                            );
                        }),
                        // The tray dot, tooltip and items follow the test (DA17).
                        on_state: Box::new(move |status| {
                            let mark = tray::TestMark::from_status(status);
                            if mark.is_none() || status.state == oma_core::load::RunState::Starting
                            {
                                // The next test may toast again.
                                CLOSE_TOAST_SHOWN
                                    .store(false, std::sync::atomic::Ordering::Relaxed);
                            }
                            state_tray.set_test(mark);
                        }),
                        emit_bench: Box::new(move |status| {
                            let _ = bench_handle.emit_to(
                                window::MAIN,
                                performance::bench::EVENT_BENCH,
                                status,
                            );
                        }),
                        // The same dot, its own tooltip (DB9).
                        on_bench: Box::new(move |status| {
                            let mark = tray::TestMark::from_bench(status);
                            if mark.is_none()
                                || status.state == oma_core::scores::BenchState::Starting
                            {
                                CLOSE_TOAST_SHOWN
                                    .store(false, std::sync::atomic::Ordering::Relaxed);
                            }
                            bench_tray.set_test(mark);
                        }),
                        app_version: app.package_info().version.to_string(),
                    },
                ));
                app.manage(runner.clone());
                // A test the last run left open becomes a session (DA14), off the main thread.
                let recover = runner.clone();
                std::thread::Builder::new()
                    .name("oma-perf-recover".into())
                    .spawn(move || recover.recover())?;
                runner
            };
            // The overlay controller: with the overlay off and without
            // `OMA_FRAMES_DEBUG` its thread only waits (§11).
            #[cfg(windows)]
            let overlay = {
                let shell = app.state::<ServiceShell>();
                let status_handle = app.handle().clone();
                let data_handle = app.handle().clone();
                let status_tray = tray.clone();
                // `overlay-preview` only when the preview opens or closes.
                let preview_open = std::sync::atomic::AtomicBool::new(false);
                let bench_log = log_service.clone();
                let runner = overlay::runner::OverlayRunner::start(overlay::runner::OverlayDeps {
                    store: store.clone(),
                    link: shell.link_commands(),
                    feed: shell.frames_feed(),
                    toaster: Box::new(toaster.clone()),
                    on_status: Box::new(move |status| {
                        status_tray.set_overlay(status.enabled, !status.hidden_by_user);
                        let _ = status_handle.emit(overlay::runner::EVENT_OVERLAY_STATUS, status);
                        let open = status.preview;
                        if preview_open.swap(open, std::sync::atomic::Ordering::Relaxed) != open {
                            let _ = status_handle.emit_to(
                                window::EDITOR,
                                overlay::runner::EVENT_PREVIEW,
                                serde_json::json!({ "open": open }),
                            );
                        }
                    }),
                    on_editor_data: Box::new(move |data| {
                        let _ = data_handle.emit_to(
                            window::EDITOR,
                            overlay::runner::EVENT_EDITOR_DATA,
                            data,
                        );
                    }),
                    benchmarks_dir: Box::new(move |settings| {
                        bench_log
                            .configured_dir(settings)
                            .map(|dir| dir.join(overlay::benchmark::BENCHMARKS_DIR))
                    }),
                })?;
                let handle = runner.handle();
                app.manage(handle.clone());
                app.manage(OverlayGuard(Mutex::new(Some(runner))));
                handle
            };
            // The log's and the overlay's settings drive the global hotkeys.
            #[cfg(windows)]
            let overlay_actions: Option<Arc<dyn hotkeys::OverlayActions>> =
                Some(Arc::new(overlay.clone()));
            #[cfg(not(windows))]
            let overlay_actions: Option<Arc<dyn hotkeys::OverlayActions>> = None;
            hotkeys::install_hotkeys(app.handle(), &store, log_service.clone(), overlay_actions);
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
                    // CPU readings for a stress test; never waits on it.
                    #[cfg(windows)]
                    perf.on_tick(out, schema);
                }
                // Sensor values for the overlay; never waits on it.
                #[cfg(windows)]
                overlay.on_tick(out, alerts.schema());
                // Nobody listens while the windows are closed: skip serialization.
                if !window::any_open(&handle) {
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
        .build(context)
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
            let test_running = app
                .try_state::<Arc<performance::runner::PerformanceRunner>>()
                .is_some_and(|runner| runner.is_running());
            if keep_running_on_last_close(close_to_tray, test_running) {
                api.prevent_exit();
                if close_toast_due(test_running, &CLOSE_TOAST_SHOWN) {
                    use notifier::ToastSink;
                    let lang = tray::language_for(
                        app.state::<Arc<SettingsStore>>()
                            .settings()
                            .general
                            .language,
                    );
                    app.state::<Arc<notifier::SystemToaster>>().show(
                        tray_icon::PRODUCT_NAME.to_owned(),
                        i18n::t(lang, "performance.closeToTray", &[]),
                        notifier::launch_for_run(),
                    );
                }
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
            // After the sampler (no more `Values`), before the link goes
            // away: the engine off, then the overlay closed (bounded, 2 s).
            #[cfg(windows)]
            if let Some(guard) = app.try_state::<OverlayGuard>() {
                if let Some(runner) = guard
                    .0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
                {
                    runner.stop();
                }
            }
            // A stress test in progress ends as `stopped_user` (bounded, DA16).
            #[cfg(windows)]
            if let Some(runner) = app.try_state::<Arc<performance::runner::PerformanceRunner>>() {
                runner.shutdown(Duration::from_secs(2));
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
    /// manifest and allowed by a window's capability, or the UI gets
    /// "not allowed. Command not found" at run time; and every command a
    /// capability allows must be registered.
    #[test]
    fn every_registered_command_is_in_the_manifest_and_the_capability() {
        let main_src = include_str!("main.rs");
        let build_src = include_str!("../build.rs");
        let capabilities = [
            ("default.json", include_str!("../capabilities/default.json")),
            ("editor.json", include_str!("../capabilities/editor.json")),
        ];

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
        let permission = |name: &str| format!("allow-{}", name.replace('_', "-"));

        let mut missing = Vec::new();
        for name in &commands {
            if !build_src.contains(&format!("\"{name}\"")) {
                missing.push(format!("{name}: missing from build.rs"));
            }
            let quoted = format!("\"{}\"", permission(name));
            if !capabilities.iter().any(|(_, json)| json.contains(&quoted)) {
                missing.push(format!("{name}: in no capability"));
            }
        }
        for (file, json) in capabilities {
            let parsed: serde_json::Value = serde_json::from_str(json).expect(file);
            for allowed in parsed["permissions"].as_array().expect(file) {
                let allowed = allowed.as_str().expect(file);
                if allowed.starts_with("allow-")
                    && !commands.iter().any(|name| permission(name) == allowed)
                {
                    missing.push(format!("{file}: {allowed} is not a registered command"));
                }
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
        assert!(keep_running_on_last_close(true, false));
        assert!(!keep_running_on_last_close(false, false));
    }

    #[test]
    fn closing_the_window_keeps_running_during_a_test() {
        assert!(keep_running_on_last_close(false, true));
        assert!(keep_running_on_last_close(true, true));
    }

    #[test]
    fn close_toast_comes_once_per_test() {
        let shown = std::sync::atomic::AtomicBool::new(false);
        assert!(!close_toast_due(false, &shown), "no test, no toast");
        assert!(close_toast_due(true, &shown));
        assert!(!close_toast_due(true, &shown));
        shown.store(false, std::sync::atomic::Ordering::Relaxed);
        assert!(close_toast_due(true, &shown));
    }
}
