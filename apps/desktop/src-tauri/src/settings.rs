//! Persistent app settings, stored as JSON in the app data directory.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    /// Push-to-talk shortcut in `tauri-plugin-global-shortcut` format, e.g. "Alt+Space".
    pub hotkey: String,
    /// Model id from the catalog in `models.rs`.
    pub model_id: String,
    /// Recognition language: whisper language code or "auto".
    pub language: String,
    /// UI language preference: "auto", "en" or "ru".
    pub ui_language: String,
    /// Set after first-run onboarding finishes or after minimum readiness dismiss.
    pub onboarding_completed: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hotkey: "Alt+Space".into(),
            model_id: "base".into(),
            language: "auto".into(),
            ui_language: "auto".into(),
            onboarding_completed: false,
        }
    }
}

pub struct SettingsStore {
    path: PathBuf,
    current: AppSettings,
}

impl SettingsStore {
    pub fn load(app: &AppHandle) -> Self {
        let path = app
            .path()
            .app_data_dir()
            .map(|d| d.join("settings.json"))
            .unwrap_or_else(|_| PathBuf::from("settings.json"));

        let current = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();

        Self { path, current }
    }

    pub fn current(&self) -> &AppSettings {
        &self.current
    }

    pub fn update(&mut self, settings: AppSettings) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create settings dir: {e}"))?;
        }
        let raw = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, raw).map_err(|e| format!("write settings: {e}"))?;
        self.current = settings;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_onboarding_completed_is_false() {
        let s = AppSettings::default();
        assert!(!s.onboarding_completed);
    }

    #[test]
    fn missing_onboarding_field_deserializes_to_false() {
        let s: AppSettings = serde_json::from_str(
            r#"{
            "hotkey": "Alt+Space",
            "modelId": "base",
            "language": "auto",
            "uiLanguage": "auto"
        }"#,
        )
        .unwrap();
        assert!(!s.onboarding_completed);
    }
}
