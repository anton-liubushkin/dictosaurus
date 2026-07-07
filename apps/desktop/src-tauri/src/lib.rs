mod audio;
mod commands;
mod dictation;
pub mod hf_catalog;
mod hotkey;
pub mod models;
mod overlay;
mod paste;
mod settings;
mod transcribe;
mod tray;

use std::sync::Mutex;
use tauri::Manager;

pub struct AppState {
    pub settings: Mutex<settings::SettingsStore>,
    pub dictation: dictation::Dictation,
}

pub fn run() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_settings(app);
        }))
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(if cfg!(debug_assertions) {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Info
                })
                .build(),
        )
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .plugin(tauri_nspanel::init())
            .plugin(tauri_plugin_macos_permissions::init());
    }

    builder
        .setup(|app| {
            // Menu bar app: no Dock icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            models::init_storage(app.handle());
            hf_catalog::init(app.handle());
            let store = settings::SettingsStore::load(app.handle());
            let hotkey_str = store.current().hotkey.clone();

            app.manage(AppState {
                settings: Mutex::new(store),
                dictation: dictation::Dictation::default(),
            });

            tray::init(app.handle())?;
            overlay::init(app.handle());

            if let Err(e) = hotkey::register(app.handle(), &hotkey_str) {
                log::warn!("[hotkey] {e}");
            }

            transcribe::preload_in_background(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            // The settings window hides instead of closing — the app lives in the tray.
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::update_settings,
            commands::list_models,
            commands::list_hf_models,
            commands::download_model,
            commands::delete_model,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Showing the window from `setup` is racy — the event loop is not
            // ready yet and the window stays off-screen.
            if let tauri::RunEvent::Ready = event {
                tray::show_settings(app);
            }
        });
}
