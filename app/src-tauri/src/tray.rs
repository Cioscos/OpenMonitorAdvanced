//! Tray icon: open the window, the anti-cheat compatible mode toggle, or quit.

use std::sync::Arc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::service::ServiceShell;
use crate::window;

pub struct TrayLabels {
    pub open: String,
    pub quit: String,
    pub anti_cheat: String,
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
        anti_cheat: text("tray.antiCheat"),
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let labels = labels_for(&sys_locale::get_locale().unwrap_or_default());
    let open = MenuItem::with_id(app, "open", labels.open, true, None::<&str>)?;
    let initial_anti_cheat = app.state::<ServiceShell>().anti_cheat_enabled();
    let anti_cheat = CheckMenuItem::with_id(
        app,
        "anti_cheat",
        labels.anti_cheat,
        true,
        initial_anti_cheat,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", labels.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &anti_cheat, &quit])?;
    // The checkbox follows the settings store (see `ToggleState`), whichever
    // way `sources.antiCheat` changes: this item, the `set_anti_cheat`
    // command or the settings view.
    app.state::<ServiceShell>()
        .set_tray_item(Arc::new(anti_cheat) as Arc<dyn crate::service::ToggleIndicator>);
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().expect("bundle icon").clone())
        .tooltip("OpenMonitor Advanced")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => window::show_main(app),
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn italian_locales_get_italian_labels() {
        assert_eq!(labels_for("it-IT").open, "Apri");
        assert_eq!(labels_for("it").quit, "Esci");
        assert_eq!(
            labels_for("it-IT").anti_cheat,
            "Modalità compatibile anti-cheat"
        );
    }

    #[test]
    fn other_locales_fall_back_to_english() {
        assert_eq!(labels_for("en-US").open, "Open");
        assert_eq!(labels_for("de-DE").quit, "Quit");
        assert_eq!(labels_for("").open, "Open");
        assert_eq!(labels_for("en").anti_cheat, "Anti-cheat compatible mode");
    }
}
