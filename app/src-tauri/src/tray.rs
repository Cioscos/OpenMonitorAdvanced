//! Minimal tray icon (milestone 1): open the window or quit.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

use crate::window;

pub struct TrayLabels {
    pub open: String,
    pub quit: String,
}

/// The webview may be destroyed, so tray labels are localized in Rust.
pub fn labels_for(locale: &str) -> TrayLabels {
    let en: serde_json::Value =
        serde_json::from_str(include_str!("../../src/lib/i18n/en.json")).expect("en catalog");
    let it: serde_json::Value =
        serde_json::from_str(include_str!("../../src/lib/i18n/it.json")).expect("it catalog");
    let base = locale.split(['-', '_']).next().unwrap_or("");
    let catalog = if base.eq_ignore_ascii_case("it") {
        &it
    } else {
        &en
    };
    let text = |key: &str| {
        catalog[key]
            .as_str()
            .or_else(|| en[key].as_str())
            .expect("tray key")
            .to_owned()
    };
    TrayLabels {
        open: text("tray.open"),
        quit: text("tray.quit"),
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let labels = labels_for(&sys_locale::get_locale().unwrap_or_default());
    let open = MenuItem::with_id(app, "open", labels.open, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", labels.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().expect("bundle icon").clone())
        .tooltip("OpenMonitor Advanced")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => window::show_main(app),
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn italian_locales_get_italian_labels() {
        assert_eq!(labels_for("it-IT").open, "Apri");
        assert_eq!(labels_for("it").quit, "Esci");
    }

    #[test]
    fn other_locales_fall_back_to_english() {
        assert_eq!(labels_for("en-US").open, "Open");
        assert_eq!(labels_for("de-DE").quit, "Quit");
        assert_eq!(labels_for("").open, "Open");
    }
}
