//! Whisper (ggml) model catalog and downloader.
//!
//! Models are fetched from the official `ggerganov/whisper.cpp` repository on
//! Hugging Face and stored in `<app data>/models`. Download progress is
//! reported to the frontend via the `model-download-progress` event.

use futures_util::StreamExt;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

static MODELS_DIR: OnceLock<PathBuf> = OnceLock::new();
static DOWNLOAD_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

const PROGRESS_EVENT: &str = "model-download-progress";
const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

#[derive(Clone, Copy, Debug)]
pub struct ModelDef {
    pub id: &'static str,
    pub label: &'static str,
    pub size_label: &'static str,
    pub description: &'static str,
    pub file_name: &'static str,
    pub bytes_hint: u64,
}

pub fn catalog() -> &'static [ModelDef] {
    &[
        ModelDef {
            id: "tiny",
            label: "Tiny",
            size_label: "~75 MB",
            description: "Fastest, lowest accuracy",
            file_name: "ggml-tiny.bin",
            bytes_hint: 77_700_000,
        },
        ModelDef {
            id: "base",
            label: "Base",
            size_label: "~142 MB",
            description: "Fast, decent for short phrases",
            file_name: "ggml-base.bin",
            bytes_hint: 147_950_000,
        },
        ModelDef {
            id: "small",
            label: "Small",
            size_label: "~466 MB",
            description: "Balanced speed and accuracy",
            file_name: "ggml-small.bin",
            bytes_hint: 487_600_000,
        },
        ModelDef {
            id: "medium",
            label: "Medium",
            size_label: "~1.5 GB",
            description: "Slower, high accuracy",
            file_name: "ggml-medium.bin",
            bytes_hint: 1_533_800_000,
        },
        ModelDef {
            id: "large-v3-turbo-q5_0",
            label: "Large v3 Turbo (Q5)",
            size_label: "~547 MB",
            description: "Great accuracy, compact and fast — recommended",
            file_name: "ggml-large-v3-turbo-q5_0.bin",
            bytes_hint: 574_000_000,
        },
        ModelDef {
            id: "large-v3-turbo",
            label: "Large v3 Turbo",
            size_label: "~1.6 GB",
            description: "Best accuracy",
            file_name: "ggml-large-v3-turbo.bin",
            bytes_hint: 1_624_600_000,
        },
    ]
}

pub fn def_by_id(id: &str) -> Option<&'static ModelDef> {
    catalog().iter().find(|d| d.id == id)
}

pub fn init_storage(app: &AppHandle) {
    let dir = match app.path().app_data_dir() {
        Ok(base) => base.join("models"),
        Err(e) => {
            log::warn!("[models] app_data_dir: {e}");
            return;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("[models] create_dir_all {}: {e}", dir.display());
        return;
    }
    let _ = MODELS_DIR.set(dir);
}

fn models_dir() -> Option<&'static Path> {
    MODELS_DIR.get().map(|p| p.as_path())
}

pub fn resolve_model_path(model_id: &str) -> Option<PathBuf> {
    let def = def_by_id(model_id)?;
    let path = models_dir()?.join(def.file_name);
    path.is_file().then_some(path)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub size_label: String,
    pub description: String,
    pub downloaded: bool,
}

pub fn catalog_status() -> Vec<ModelInfo> {
    catalog()
        .iter()
        .map(|d| ModelInfo {
            id: d.id.to_string(),
            label: d.label.to_string(),
            size_label: d.size_label.to_string(),
            description: d.description.to_string(),
            downloaded: resolve_model_path(d.id).is_some(),
        })
        .collect()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgressPayload {
    model_id: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    percent: u8,
    done: bool,
    error: Option<String>,
}

fn emit_progress(
    app: &AppHandle,
    model_id: &str,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    percent: u8,
    done: bool,
    error: Option<String>,
) {
    let _ = app.emit(
        PROGRESS_EVENT,
        DownloadProgressPayload {
            model_id: model_id.to_string(),
            downloaded_bytes,
            total_bytes,
            percent: percent.min(100),
            done,
            error,
        },
    );
}

pub async fn download_model(app: &AppHandle, model_id: String) -> Result<(), String> {
    let def = def_by_id(&model_id).ok_or_else(|| format!("unknown model id: {model_id}"))?;

    if resolve_model_path(def.id).is_some() {
        emit_progress(app, &model_id, 0, None, 100, true, None);
        return Ok(());
    }

    let dir = models_dir()
        .map(Path::to_path_buf)
        .ok_or_else(|| "models directory not initialized".to_string())?;

    let _guard = DOWNLOAD_LOCK.lock().await;
    if resolve_model_path(def.id).is_some() {
        return Ok(());
    }

    let dest = dir.join(def.file_name);
    let url = format!("{HF_BASE}/{}", def.file_name);
    let hint = def.bytes_hint;

    emit_progress(app, &model_id, 0, None, 0, false, None);

    let progress_app = app.clone();
    let progress_id = model_id.clone();
    let result = download_file(&url, &dest, move |downloaded, total| {
        let pct = match total {
            Some(t) if t > 0 => ((downloaded as f64 / t as f64) * 100.0).min(99.0) as u8,
            _ => ((downloaded as f64 / hint.max(1) as f64) * 100.0).min(99.0) as u8,
        };
        emit_progress(&progress_app, &progress_id, downloaded, total, pct, false, None);
    })
    .await;

    match result {
        Ok(()) => {
            log::info!("[models] downloaded {} -> {}", def.id, dest.display());
            emit_progress(app, &model_id, 0, None, 100, true, None);
            Ok(())
        }
        Err(error) => {
            emit_progress(app, &model_id, 0, None, 0, true, Some(error.clone()));
            Err(error)
        }
    }
}

pub fn delete_model(model_id: &str) -> Result<(), String> {
    let path = resolve_model_path(model_id).ok_or_else(|| "model is not downloaded".to_string())?;
    std::fs::remove_file(&path).map_err(|e| format!("delete {}: {e}", path.display()))
}

/// Downloads `url` into `dest` atomically (via a `.partial` file + rename).
async fn download_file(
    url: &str,
    dest: &Path,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| e.to_string())?;

    let partial = dest.with_extension("bin.partial");

    let run = async {
        let response = client
            .get(url)
            .header(reqwest::header::USER_AGENT, "Dictosaurus/0.1")
            .send()
            .await
            .map_err(|e| format!("request: {e}"))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }

        let total = response.content_length();
        let mut stream = response.bytes_stream();
        let mut file = tokio::fs::File::create(&partial)
            .await
            .map_err(|e| format!("create file: {e}"))?;

        let mut downloaded: u64 = 0;
        on_progress(0, total);

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("stream: {e}"))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("write: {e}"))?;
            downloaded += chunk.len() as u64;
            on_progress(downloaded, total);
        }

        file.flush().await.map_err(|e| format!("flush: {e}"))?;
        Ok(())
    };

    match run.await {
        Ok(()) => std::fs::rename(&partial, dest).map_err(|e| format!("finalize: {e}")),
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            Err(e)
        }
    }
}
