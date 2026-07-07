//! Push-to-talk global shortcut registration.

use crate::dictation;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub fn register(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let shortcut: Shortcut = hotkey
        .parse()
        .map_err(|e| format!("invalid hotkey \"{hotkey}\": {e}"))?;

    app.global_shortcut()
        .on_shortcut(shortcut, |app, _shortcut, event| {
            log::debug!("[hotkey] event: {:?}", event.state);
            match event.state {
                ShortcutState::Pressed => dictation::hotkey_pressed(app),
                ShortcutState::Released => dictation::hotkey_released(app),
            }
        })
        .map_err(|e| format!("register hotkey \"{hotkey}\": {e}"))?;

    log::info!("[hotkey] registered \"{hotkey}\"");
    Ok(())
}

/// Swaps the registered hotkey; restores the old one if the new registration fails.
pub fn replace(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    let _: Shortcut = new
        .parse()
        .map_err(|e| format!("invalid hotkey \"{new}\": {e}"))?;

    if let Ok(old_shortcut) = old.parse::<Shortcut>() {
        let _ = app.global_shortcut().unregister(old_shortcut);
    }

    match register(app, new) {
        Ok(()) => Ok(()),
        Err(e) => {
            if let Err(restore) = register(app, old) {
                log::error!("[hotkey] failed to restore previous hotkey: {restore}");
            }
            Err(e)
        }
    }
}
