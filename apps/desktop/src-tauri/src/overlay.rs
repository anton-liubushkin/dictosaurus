//! Voice orb overlay window.
//!
//! On macOS this is a non-activating NSPanel at screen-saver level with
//! `canJoinAllSpaces + fullScreenAuxiliary`, so it renders above fullscreen
//! apps without stealing focus from the app the user is dictating into.
//! The panel is created once at startup (so the webview is warm) and only
//! shown/hidden afterwards.

use tauri::{AppHandle, Manager};

pub const LABEL: &str = "overlay";
// Wide enough for the live-transcription pill next to the orb; the window is
// transparent and click-through, so the extra area is invisible.
const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 270.0;
const BOTTOM_MARGIN: f64 = 40.0;

#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    panel!(DictationPanel {
        config: {
            can_become_key_window: false,
            is_floating_panel: true
        }
    })
}

pub fn init(app: &AppHandle) {
    let url = tauri::WebviewUrl::App("index.html?mode=overlay".into());

    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::{CollectionBehavior, PanelBuilder, PanelLevel, StyleMask};

        let collection_behavior = CollectionBehavior::new()
            .can_join_all_spaces()
            .full_screen_auxiliary()
            .stationary();
        let style_mask = StyleMask::empty().borderless().nonactivating_panel();

        let result = PanelBuilder::<_, DictationPanel>::new(app, LABEL)
            .url(url)
            .title("Dictation")
            .position(tauri::Position::Logical(tauri::LogicalPosition::new(0.0, 0.0)))
            .size(tauri::Size::Logical(tauri::LogicalSize::new(WIDTH, HEIGHT)))
            // NSScreenSaverWindowLevel — above fullscreen apps and games.
            .level(PanelLevel::Custom(1000))
            .collection_behavior(collection_behavior)
            .style_mask(style_mask)
            .floating(true)
            .hides_on_deactivate(false)
            .no_activate(true)
            .transparent(true)
            .opaque(false)
            .has_shadow(false)
            .with_window(|w| {
                w.transparent(true)
                    .decorations(false)
                    .resizable(false)
                    .focused(false)
                    .shadow(false)
                    .accept_first_mouse(false)
                    .visible(false)
                    .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
            })
            .build();

        match result {
            Ok(panel) => {
                panel.set_ignores_mouse_events(true);
                panel.hide();
            }
            Err(e) => log::error!("[overlay] failed to create NSPanel: {e}"),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let result = tauri::WebviewWindowBuilder::new(app, LABEL, url)
            .inner_size(WIDTH, HEIGHT)
            .transparent(true)
            .decorations(false)
            .resizable(false)
            .always_on_top(true)
            .visible_on_all_workspaces(true)
            .skip_taskbar(true)
            .focused(false)
            .shadow(false)
            .visible(false)
            .build();
        if let Err(e) = result {
            log::error!("[overlay] failed to create window: {e}");
        }
    }
}

pub fn show(app: &AppHandle) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        reposition(&app);

        #[cfg(target_os = "macos")]
        {
            use tauri_nspanel::{CollectionBehavior, ManagerExt};
            match app.get_webview_panel(LABEL) {
                Ok(panel) => {
                    // macOS can reset click-through on some calls; re-assert it.
                    panel.set_ignores_mouse_events(true);
                    // Re-assert the collection behavior on every show. Without
                    // this the panel created at startup stays bound to the
                    // Space it was created on and never follows the user to
                    // other virtual desktops / fullscreen Spaces.
                    panel.set_collection_behavior(
                        CollectionBehavior::new()
                            .can_join_all_spaces()
                            .full_screen_auxiliary()
                            .stationary()
                            .value(),
                    );
                    panel.show();
                    panel.order_front_regardless();
                }
                // Should not happen (the panel lives for the whole app run);
                // log loudly so a one-way "overlay never shows again" failure
                // is diagnosable instead of silent.
                Err(e) => log::error!("[overlay] show failed, panel not found: {e:?}"),
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            if let Some(window) = app.get_webview_window(LABEL) {
                let _ = window.show();
            }
        }
    });
}

pub fn hide(app: &AppHandle) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        {
            use tauri_nspanel::ManagerExt;
            if let Ok(panel) = app.get_webview_panel(LABEL) {
                panel.hide();
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            if let Some(window) = app.get_webview_window(LABEL) {
                let _ = window.hide();
            }
        }
    });
}

/// Bottom-center of the monitor the cursor is on.
fn reposition(app: &AppHandle) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let monitor = cursor_monitor(app).or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };

    let scale = monitor.scale_factor();
    let width = (WIDTH * scale).round() as i32;
    let height = (HEIGHT * scale).round() as i32;
    let pos = monitor.position();
    let size = monitor.size();

    let x = pos.x + (size.width as i32 - width) / 2;
    let y = pos.y + size.height as i32 - height - (BOTTOM_MARGIN * scale).round() as i32;
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

fn cursor_monitor(app: &AppHandle) -> Option<tauri::Monitor> {
    let point = app.cursor_position().ok()?;
    app.monitor_from_point(point.x, point.y).ok().flatten()
}
