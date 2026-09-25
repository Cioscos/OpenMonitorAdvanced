fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["get_schema", "get_history"])),
    )
    .expect("Tauri build")
}
