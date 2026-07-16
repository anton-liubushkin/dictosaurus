//! Menu bar (tray) icon and menu.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

/// Resolves the "auto" UI language preference to a concrete tray language.
/// Keeps it KISS: no OS locale APIs, just the `LANG` environment variable.
fn resolve_tray_language(ui_language: &str) -> &'static str {
    if ui_language != "auto" {
        return if ui_language == "ru" { "ru" } else { "en" };
    }
    let lang = std::env::var("LANG").unwrap_or_default();
    if lang.starts_with("ru") {
        "ru"
    } else {
        "en"
    }
}

/// Returns `(settings_label, quit_label)` for the tray menu in the given language.
pub fn menu_labels(ui_language: &str) -> (&'static str, &'static str) {
    match resolve_tray_language(ui_language) {
        "ru" => ("Настройки…", "Выйти из Dictosaurus"),
        _ => ("Settings…", "Quit Dictosaurus"),
    }
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let ui_language = app
        .state::<crate::AppState>()
        .settings
        .lock()
        .ok()
        .map(|s| s.current().ui_language.clone())
        .unwrap_or_else(|| "auto".into());
    let (settings_label, quit_label) = menu_labels(&ui_language);

    let settings_item = MenuItem::with_id(app, "settings", settings_label, true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings_item, &separator, &quit_item])?;

    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;

    TrayIconBuilder::with_id("tray")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Dictosaurus")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

pub fn show_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn show_settings_section(app: &AppHandle, section: &str) {
    show_settings(app);
    let _ = app.emit("settings-open-section", section);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_labels_ru_uses_russian_strings() {
        assert_eq!(menu_labels("ru"), ("Настройки…", "Выйти из Dictosaurus"));
    }

    #[test]
    fn menu_labels_en_uses_english_strings() {
        assert_eq!(menu_labels("en"), ("Settings…", "Quit Dictosaurus"));
    }

    #[test]
    fn menu_labels_unknown_language_falls_back_to_english() {
        assert_eq!(menu_labels("fr"), ("Settings…", "Quit Dictosaurus"));
    }
}
