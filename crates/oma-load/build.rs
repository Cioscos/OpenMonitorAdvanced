//! Embeds the version resource of `oma-load.exe` (plan DP8), so the file
//! properties show the product and the `X.Y.Z` version like the app's.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    let mut res = tauri_winres::WindowsResource::new();
    res.set("ProductName", "OpenMonitor Advanced")
        .set("FileDescription", "OpenMonitor Advanced load generator")
        .set("FileVersion", &version)
        .set("ProductVersion", &version)
        .set("OriginalFilename", "oma-load.exe")
        .set("InternalName", "oma-load");
    res.compile().expect("compile the version resource");
}
