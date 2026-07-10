//! Tauri commands exposed to the frontend.

use crate::{
    dictionary::{DictionaryDocument, DictionaryError, DictionaryStore},
    hotkey, models,
    settings::AppSettings,
    transcribe, AppState,
};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, State};

const DICTIONARY_LOCK_ERROR: &str = "dictionary state is unavailable";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DictionaryErrorCode {
    CorruptJson,
    UnsupportedVersion,
    Storage,
    Validation,
    Unavailable,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryCommandError {
    pub code: DictionaryErrorCode,
    pub message: String,
}

impl DictionaryCommandError {
    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: DictionaryErrorCode::Internal,
            message: message.into(),
        }
    }
}

impl From<DictionaryError> for DictionaryCommandError {
    fn from(error: DictionaryError) -> Self {
        let code = match &error {
            DictionaryError::CorruptJson { .. } => DictionaryErrorCode::CorruptJson,
            DictionaryError::UnsupportedVersion { .. } => DictionaryErrorCode::UnsupportedVersion,
            DictionaryError::Storage { .. } => DictionaryErrorCode::Storage,
            DictionaryError::Unavailable { .. } => DictionaryErrorCode::Unavailable,
            DictionaryError::AliasConflict { .. }
            | DictionaryError::DocumentTooLarge { .. }
            | DictionaryError::DuplicateId { .. }
            | DictionaryError::EmptyAlias { .. }
            | DictionaryError::EmptyCanonicalTerm { .. }
            | DictionaryError::EmptyId { .. }
            | DictionaryError::NullByte { .. }
            | DictionaryError::TooManyAliases { .. }
            | DictionaryError::TooManyEntries { .. } => DictionaryErrorCode::Validation,
            DictionaryError::Regex { .. } | DictionaryError::Serialization { .. } => {
                DictionaryErrorCode::Internal
            }
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

fn get_dictionary_from(
    dictionary: &Mutex<DictionaryStore>,
) -> Result<DictionaryDocument, DictionaryCommandError> {
    let store = dictionary
        .lock()
        .map_err(|_| DictionaryCommandError::internal(DICTIONARY_LOCK_ERROR))?;
    if let Some(error) = store.load_error() {
        return Err(error.clone().into());
    }
    Ok(store.current().clone())
}

fn update_dictionary_in(
    dictionary: &Mutex<DictionaryStore>,
    document: DictionaryDocument,
) -> Result<DictionaryDocument, DictionaryCommandError> {
    dictionary
        .lock()
        .map_err(|_| DictionaryCommandError::internal(DICTIONARY_LOCK_ERROR))?
        .update(document)
        .map_err(Into::into)
}

fn reset_dictionary_in(dictionary: &Mutex<DictionaryStore>) -> Result<(), DictionaryCommandError> {
    dictionary
        .lock()
        .map_err(|_| DictionaryCommandError::internal(DICTIONARY_LOCK_ERROR))?
        .reset()
        .map_err(Into::into)
}

fn reload_dictionary_in(
    dictionary: &Mutex<DictionaryStore>,
) -> Result<DictionaryDocument, DictionaryCommandError> {
    dictionary
        .lock()
        .map_err(|_| DictionaryCommandError::internal(DICTIONARY_LOCK_ERROR))?
        .reload()
        .map_err(Into::into)
}

#[tauri::command]
pub fn get_dictionary(
    state: State<'_, AppState>,
) -> Result<DictionaryDocument, DictionaryCommandError> {
    get_dictionary_from(&state.dictionary)
}

#[tauri::command]
pub fn update_dictionary(
    state: State<'_, AppState>,
    document: DictionaryDocument,
) -> Result<DictionaryDocument, DictionaryCommandError> {
    update_dictionary_in(&state.dictionary, document)
}

#[tauri::command]
pub fn reset_dictionary(state: State<'_, AppState>) -> Result<(), DictionaryCommandError> {
    reset_dictionary_in(&state.dictionary)
}

#[tauri::command]
pub fn reload_dictionary(
    state: State<'_, AppState>,
) -> Result<DictionaryDocument, DictionaryCommandError> {
    reload_dictionary_in(&state.dictionary)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::{
        DictionaryDocument, DictionaryEntry, DictionaryError, DictionaryStore, DICTIONARY_VERSION,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "dictosaurus-commands-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn document() -> DictionaryDocument {
        DictionaryDocument {
            version: DICTIONARY_VERSION,
            enabled: true,
            entries: vec![DictionaryEntry {
                id: "rust".into(),
                term: "Rust".into(),
                aliases: vec!["rustlang".into()],
                enabled: true,
            }],
        }
    }

    #[test]
    fn get_dictionary_reports_load_error_without_overwriting_source() {
        let dir = TestDir::new();
        let path = dir.0.join("dictionary.json");
        fs::write(&path, "{not-json").unwrap();
        let dictionary = Mutex::new(DictionaryStore::from_path(path.clone()));

        let error = get_dictionary_from(&dictionary).unwrap_err();

        assert_eq!(error.code, DictionaryErrorCode::CorruptJson);
        assert!(error.message.contains("invalid dictionary JSON"));
        assert_eq!(fs::read_to_string(path).unwrap(), "{not-json");
    }

    #[test]
    fn update_dictionary_persists_document_and_refreshes_snapshot() {
        let dir = TestDir::new();
        let path = dir.0.join("dictionary.json");
        let dictionary = Mutex::new(DictionaryStore::from_path(path.clone()));

        update_dictionary_in(&dictionary, document()).unwrap();

        assert_eq!(get_dictionary_from(&dictionary).unwrap(), document());
        assert_eq!(
            dictionary.lock().unwrap().snapshot().apply("rustlang"),
            "Rust"
        );
        let persisted: DictionaryDocument =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(persisted, document());
    }

    #[test]
    fn update_dictionary_returns_the_canonical_saved_document() {
        let dir = TestDir::new();
        let dictionary = Mutex::new(DictionaryStore::from_path(dir.0.join("dictionary.json")));
        let input = DictionaryDocument {
            entries: vec![DictionaryEntry {
                id: "  rust  ".into(),
                term: "  Rust   Language  ".into(),
                aliases: vec![" rustlang ".into(), "RUSTLANG".into()],
                enabled: true,
            }],
            ..DictionaryDocument::default()
        };

        let saved = update_dictionary_in(&dictionary, input).unwrap();

        assert_eq!(
            saved,
            DictionaryDocument {
                entries: vec![DictionaryEntry {
                    id: "rust".into(),
                    term: "Rust Language".into(),
                    aliases: vec!["rustlang".into()],
                    enabled: true,
                }],
                ..DictionaryDocument::default()
            }
        );
    }

    #[test]
    fn reset_dictionary_recovers_from_load_error() {
        let dir = TestDir::new();
        let path = dir.0.join("dictionary.json");
        fs::write(&path, "{not-json").unwrap();
        let dictionary = Mutex::new(DictionaryStore::from_path(path));

        reset_dictionary_in(&dictionary).unwrap();

        assert_eq!(
            get_dictionary_from(&dictionary).unwrap(),
            DictionaryDocument::default()
        );
    }

    #[test]
    fn reload_dictionary_recovers_after_external_file_repair() {
        let dir = TestDir::new();
        let path = dir.0.join("dictionary.json");
        fs::write(&path, "{not-json").unwrap();
        let dictionary = Mutex::new(DictionaryStore::from_path(path.clone()));
        fs::write(&path, serde_json::to_vec(&document()).unwrap()).unwrap();

        let reloaded = reload_dictionary_in(&dictionary).unwrap();

        assert_eq!(reloaded, document());
        assert_eq!(get_dictionary_from(&dictionary).unwrap(), document());
    }

    #[test]
    fn dictionary_error_codes_are_stable_and_centralized() {
        let cases = [
            (
                DictionaryError::CorruptJson {
                    message: "bad json".into(),
                },
                DictionaryErrorCode::CorruptJson,
            ),
            (
                DictionaryError::UnsupportedVersion { version: 2 },
                DictionaryErrorCode::UnsupportedVersion,
            ),
            (
                DictionaryError::Storage {
                    operation: "read dictionary",
                    message: "denied".into(),
                },
                DictionaryErrorCode::Storage,
            ),
            (
                DictionaryError::Unavailable {
                    message: "missing".into(),
                },
                DictionaryErrorCode::Unavailable,
            ),
            (
                DictionaryError::DuplicateId { id: "same".into() },
                DictionaryErrorCode::Validation,
            ),
            (
                DictionaryError::Serialization {
                    message: "failed".into(),
                },
                DictionaryErrorCode::Internal,
            ),
        ];

        for (error, expected_code) in cases {
            assert_eq!(DictionaryCommandError::from(error).code, expected_code);
        }

        assert_eq!(
            serde_json::to_value(DictionaryCommandError {
                code: DictionaryErrorCode::CorruptJson,
                message: "invalid dictionary JSON".into(),
            })
            .unwrap(),
            serde_json::json!({
                "code": "corruptJson",
                "message": "invalid dictionary JSON",
            })
        );
    }

    #[test]
    fn dictionary_commands_return_error_for_poisoned_lock() {
        let dir = TestDir::new();
        let dictionary = Arc::new(Mutex::new(DictionaryStore::from_path(
            dir.0.join("dictionary.json"),
        )));
        let poisoned = Arc::clone(&dictionary);
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("poison dictionary lock");
        })
        .join();

        assert_eq!(
            get_dictionary_from(&dictionary).unwrap_err().code,
            DictionaryErrorCode::Internal
        );
        assert_eq!(
            update_dictionary_in(&dictionary, document())
                .unwrap_err()
                .code,
            DictionaryErrorCode::Internal
        );
        assert_eq!(
            reset_dictionary_in(&dictionary).unwrap_err().code,
            DictionaryErrorCode::Internal
        );
        assert_eq!(
            reload_dictionary_in(&dictionary).unwrap_err().code,
            DictionaryErrorCode::Internal
        );
    }
}
