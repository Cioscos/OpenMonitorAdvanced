//! Tray icon: open the window, the anti-cheat compatible mode toggle, or quit.

use std::sync::Arc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use oma_core::settings::Language;

use crate::i18n::{resolve, t};
use crate::service::ServiceShell;
use crate::window;

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    // The webview may be destroyed, so tray labels are localized in Rust.
    let lang = resolve(
        Language::System,
        &sys_locale::get_locale().unwrap_or_default(),
    );
    let open = MenuItem::with_id(app, "open", t(lang, "tray.open", &[]), true, None::<&str>)?;
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
