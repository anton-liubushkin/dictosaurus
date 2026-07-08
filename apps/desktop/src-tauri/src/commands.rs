//! Tauri commands exposed to the frontend.

use crate::{hotkey, models, settings::AppSettings, transcribe, AppState};
use tauri::{AppHandle, State};

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    state.settings.lock().unwrap().current().clone()
}

#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<(), String> {
    let old = state.settings.lock().unwrap().current().clone();

    if old.hotkey != settings.hotkey {
        hotkey::replace(&app, &old.hotkey, &settings.hotkey)?;
    }

    let model_changed = old.model_id != settings.model_id;
    state.settings.lock().unwrap().update(settings)?;

    if model_changed {
        transcribe::preload_in_background(&app);
    }
    Ok(())
}

#[tauri::command]
pub fn list_models() -> Vec<models::ModelInfo> {
    models::catalog_status()
}

#[tauri::command]
pub async fn download_model(app: AppHandle, model_id: String) -> Result<(), String> {
    models::download_model(&app, model_id).await
}

#[tauri::command]
pub fn delete_model(model_id: String) -> Result<(), String> {
    models::delete_model(&model_id)
}
