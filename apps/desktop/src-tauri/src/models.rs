//! Speech model catalog and downloader.
//!
//! Two engines are supported:
//! - `whisper` — ggml models from `ggerganov/whisper.cpp` (multilingual)
//! - `gigaam`  — Sber GigaAM v3 (Russian SOTA) as ONNX for sherpa-onnx
//!
//! Models are stored in `<app data>/models`; a model may consist of several
//! files. Download progress is reported via the `model-download-progress`
//! event.

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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Whisper,
    Gigaam,
}

#[derive(Clone, Copy, Debug)]
pub struct ModelFile {
    pub file_name: &'static str,
    pub url: &'static str,
    pub bytes_hint: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct ModelDef {
    pub id: &'static str,
    pub label: &'static str,
    pub size_label: &'static str,
    /// English fallback; the UI translates by model id.
    pub description: &'static str,
    pub engine: Engine,
    /// "multilingual" or a single language code like "ru".
    pub languages: &'static str,
    pub files: &'static [ModelFile],
}

impl ModelDef {
    pub fn bytes_hint_total(&self) -> u64 {
        self.files.iter().map(|f| f.bytes_hint).sum()
    }
}

macro_rules! whisper_model {
    ($id:literal, $label:literal, $size:literal, $desc:literal, $file:literal, $bytes:literal) => {
        ModelDef {
            id: $id,
            label: $label,
            size_label: $size,
            description: $desc,
            engine: Engine::Whisper,
            languages: "multilingual",
            files: &[ModelFile {
                file_name: $file,
                url: concat!(
                    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/",
                    $file
                ),
                bytes_hint: $bytes,
            }],
        }
    };
}

pub fn catalog() -> &'static [ModelDef] {
    &[
        ModelDef {
            id: "gigaam-v3-e2e-ctc",
            label: "GigaAM v3 (Sber)",
            size_label: "~225 MB",
            description: "Best for Russian: SOTA accuracy with punctuation",
            engine: Engine::Gigaam,
            languages: "ru",
            files: &[
                ModelFile {
                    file_name: "gigaam-v3-e2e-ctc.int8.onnx",
                    url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-ctc-punct-giga-am-v3-russian-2025-12-16/resolve/main/model.int8.onnx",
                    bytes_hint: 224_893_661,
                },
                ModelFile {
                    file_name: "gigaam-v3-e2e-ctc.tokens.txt",
                    url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-ctc-punct-giga-am-v3-russian-2025-12-16/resolve/main/tokens.txt",
                    bytes_hint: 2_007,
                },
            ],
        },
        whisper_model!(
            "tiny",
            "Tiny",
            "~75 MB",
            "Fastest, lowest accuracy",
            "ggml-tiny.bin",
            77_700_000
        ),
        whisper_model!(
            "base",
            "Base",
            "~142 MB",
            "Fast, decent for short phrases",
            "ggml-base.bin",
            147_950_000
        ),
        whisper_model!(
            "small",
            "Small",
            "~466 MB",
            "Balanced speed and accuracy",
            "ggml-small.bin",
            487_600_000
        ),
        whisper_model!(
            "medium",
            "Medium",
            "~1.5 GB",
            "Slower, high accuracy",
            "ggml-medium.bin",
            1_533_800_000
        ),
        whisper_model!(
            "large-v3-turbo-q5_0",
            "Large v3 Turbo (Q5)",
            "~547 MB",
            "Great accuracy, compact and fast",
            "ggml-large-v3-turbo-q5_0.bin",
            574_000_000
        ),
        whisper_model!(
            "large-v3-turbo",
            "Large v3 Turbo",
            "~1.6 GB",
            "Best multilingual accuracy",
            "ggml-large-v3-turbo.bin",
            1_624_600_000
        ),
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

/// Absolute paths of all model files, in catalog order, if every file exists.
pub fn resolve_model_paths(model_id: &str) -> Option<Vec<PathBuf>> {
    let def = def_by_id(model_id)?;
    let dir = models_dir()?;
    let paths: Vec<PathBuf> = def.files.iter().map(|f| dir.join(f.file_name)).collect();
    paths.iter().all(|p| p.is_file()).then_some(paths)
}

pub fn is_downloaded(model_id: &str) -> bool {
    resolve_model_paths(model_id).is_some()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub size_label: String,
    pub description: String,
    pub engine: Engine,
    pub languages: String,
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
            engine: d.engine,
            languages: d.languages.to_string(),
            downloaded: is_downloaded(d.id),
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

    if is_downloaded(def.id) {
        emit_progress(app, &model_id, 0, None, 100, true, None);
        return Ok(());
    }

    let dir = models_dir()
        .map(Path::to_path_buf)
        .ok_or_else(|| "models directory not initialized".to_string())?;

    let _guard = DOWNLOAD_LOCK.lock().await;
    if is_downloaded(def.id) {
        return Ok(());
    }

    let total_hint = def.bytes_hint_total().max(1);
    emit_progress(app, &model_id, 0, None, 0, false, None);

    let mut completed_bytes: u64 = 0;
    for file in def.files {
        let dest = dir.join(file.file_name);
        if dest.is_file() {
            completed_bytes += file.bytes_hint;
            continue;
        }

        let progress_app = app.clone();
        let progress_id = model_id.clone();
        let base = completed_bytes;
        let result = download_file(file.url, &dest, move |downloaded, _total| {
            let overall = base + downloaded;
            let pct = ((overall as f64 / total_hint as f64) * 100.0).min(99.0) as u8;
            emit_progress(
                &progress_app,
                &progress_id,
                overall,
                Some(total_hint),
                pct,
                false,
                None,
            );
        })
        .await;

        if let Err(error) = result {
            emit_progress(app, &model_id, 0, None, 0, true, Some(error.clone()));
            return Err(error);
        }
        completed_bytes += file.bytes_hint;
    }

    log::info!("[models] downloaded {}", def.id);
    emit_progress(app, &model_id, 0, None, 100, true, None);
    Ok(())
}

pub fn delete_model(model_id: &str) -> Result<(), String> {
    let paths =
        resolve_model_paths(model_id).ok_or_else(|| "model is not downloaded".to_string())?;
    for path in paths {
        std::fs::remove_file(&path).map_err(|e| format!("delete {}: {e}", path.display()))?;
    }
    Ok(())
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

    let partial = dest.with_extension("partial");

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
