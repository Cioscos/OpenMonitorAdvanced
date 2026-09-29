fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_schema",
            "get_history",
            "get_stats",
            "reset_stats",
            "get_session",
            "get_gpu_processes",
            "get_startup_status",
            "enable_vendor_libraries",
            "get_service_status",
            "set_anti_cheat",
            "start_service",
            "get_settings",
            "update_settings",
        ]),
    ))
    .expect("Tauri build")
}
