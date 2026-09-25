#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod tray;
mod window;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use oma_core::engine::Engine;
use oma_core::provider::Provider;
use oma_core::sampler::{history_capacity, Sampler};
use tauri::{Emitter, Manager, RunEvent};

/// Default sampling interval (spec §4.1).
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const EVENT_SCHEMA: &str = "oma:schema";
const EVENT_SNAPSHOT: &str = "oma:snapshot";

pub struct AppState {
    pub engine: Arc<Mutex<Engine>>,
}

/// Owns the sampler so it can be stopped cleanly on exit.
struct SamplerGuard(Mutex<Option<Sampler>>);

fn providers() -> Vec<Box<dyn Provider>> {
    #[cfg(windows)]
    {
        // Task 12 replaces this with the safe-mode switch (--safe, crash marker).
        oma_win::default_providers(oma_win::gpu::VendorSwitch::new(true))
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Sets up file logging under `%LOCALAPPDATA%\OpenMonitorAdvanced\logs`. In a
/// release build the app has no console (windows subsystem), so a panic here
/// would fail silently; missing `LOCALAPPDATA` or a rolling appender that
/// cannot be built just leaves the app without a file log instead.
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    let logs = std::path::PathBuf::from(local_app_data)
        .join("OpenMonitorAdvanced")
        .join("logs");
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
                .unwrap_or_else(|_| "oma_core=debug,oma_app=info".into()),
        )
        .init();
    Some(guard)
}

fn main() {
    // Held for the program's lifetime when present, so buffered log lines are
    // flushed on drop; the app still runs (without a file log) if this is None.
    let _log_guard = init_logging();

    let start_minimized = std::env::args().any(|arg| arg == "--minimized");
    let engine = Arc::new(Mutex::new(Engine::new(
        providers(),
        history_capacity(SAMPLE_INTERVAL),
    )));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main(app)
        }))
        .manage(AppState {
            engine: engine.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_schema,
            commands::get_history
        ])
        .setup(move |app| {
            tray::build(app.handle())?;
            if !start_minimized {
                window::show_main(app.handle());
            }
            let handle = app.handle().clone();
            let sampler = Sampler::spawn(engine, SAMPLE_INTERVAL, move |out| {
                // Nobody listens while the window is closed: skip serialization.
                if handle.get_webview_window(window::MAIN).is_none() {
                    return;
                }
                if let Some(schema) = &out.schema {
                    let _ = handle.emit(EVENT_SCHEMA, schema);
                }
                let _ = handle.emit(EVENT_SNAPSHOT, &out.snapshot);
            });
            app.manage(SamplerGuard(Mutex::new(Some(sampler))));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Tauri application");

    app.run(|app, event| match event {
        // Last window closed: keep sampling in the tray. Explicit exits carry a code.
        RunEvent::ExitRequested {
            code: None, api, ..
        } => api.prevent_exit(),
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
        }
        _ => {}
    });
}
