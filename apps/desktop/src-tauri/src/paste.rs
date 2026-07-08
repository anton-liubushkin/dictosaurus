//! Clipboard + synthetic paste into the focused application.

use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

/// Copies `text` to the clipboard (it intentionally stays there) and pastes
/// it into the focused input via a synthetic Cmd+V.
pub fn insert_text(app: &AppHandle, text: &str) -> Result<(), String> {
    app.clipboard()
        .write_text(text.to_string())
        .map_err(|e| format!("clipboard: {e}"))?;
    // Give the pasteboard a moment to settle before the paste keystroke.
    std::thread::sleep(std::time::Duration::from_millis(40));
    send_paste_shortcut()
}

#[cfg(target_os = "macos")]
fn send_paste_shortcut() -> Result<(), String> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    // kVK_ANSI_V: virtual keycodes address physical keys, so Cmd+V works
    // regardless of the active keyboard layout. Requires the Accessibility
    // permission; without it the events are silently dropped by the system.
    const KEY_V: u16 = 9;

    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| "create CGEventSource".to_string())?;

    let key_down = CGEvent::new_keyboard_event(source.clone(), KEY_V, true)
        .map_err(|_| "create key-down event".to_string())?;
    key_down.set_flags(CGEventFlags::CGEventFlagCommand);
    key_down.post(CGEventTapLocation::HID);

    let key_up = CGEvent::new_keyboard_event(source, KEY_V, false)
        .map_err(|_| "create key-up event".to_string())?;
    key_up.set_flags(CGEventFlags::CGEventFlagCommand);
    key_up.post(CGEventTapLocation::HID);

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn send_paste_shortcut() -> Result<(), String> {
    Err(
        "auto-insert is not implemented on this platform yet; text was copied to the clipboard"
            .into(),
    )
}
