//! Speech model catalog and downloader.
//!
//! Engines:
//! - `whisper`          — ggml models from `ggerganov/whisper.cpp` (multilingual)
//! - `nemo_ctc`         — NeMo CTC ONNX exports for sherpa-onnx (e.g. GigaAM v3)
//! - `nemo_transducer`  — NeMo transducer ONNX exports for sherpa-onnx (e.g. Parakeet TDT)
//!
//! Models come from two sources: the curated catalog below and the dynamic
//! Hugging Face catalog (`hf_catalog`, ids prefixed with `hf:`). All files are
//! stored under `<app data>/models`; a model may consist of several files.
//! Download progress is reported via the `model-download-progress` event.

use futures_util::StreamExt;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

use crate::hf_catalog;

static MODELS_DIR: OnceLock<PathBuf> = OnceLock::new();
static DOWNLOAD_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

const PROGRESS_EVENT: &str = "model-download-progress";
pub const HF_ID_PREFIX: &str = "hf:";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    Whisper,
    NemoCtc,
    NemoTransducer,
}

#[derive(Clone, Debug)]
pub struct ModelFile {
    /// Path relative to the models directory.
    pub rel_path: String,
    pub url: String,
    pub bytes_hint: u64,
}

#[derive(Clone, Debug)]
pub struct ModelDef {
    pub id: String,
    pub label: String,
    pub size_label: String,
    /// English fallback; the UI translates curated models by id.
    pub description: String,
    pub engine: Engine,
    /// "multilingual" or comma-separated language codes like "ru" or "de,en".
    pub languages: String,
    /// Mel filterbank size override; sherpa-onnx defaults to 80 and newer
    /// exports carry the value in ONNX metadata, but old GigaAM exports need
    /// an explicit 64.
    pub feature_dim: Option<i32>,
    /// File order is significant: CTC = [model, tokens];
    /// transducer = [encoder, decoder, joiner, tokens].
    pub files: Vec<ModelFile>,
}

impl ModelDef {
    pub fn bytes_hint_total(&self) -> u64 {
        self.files.iter().map(|f| f.bytes_hint).sum()
    }
}

fn whisper_model(
    id: &str,
    label: &str,
    size_label: &str,
    description: &str,
    file: &str,
    bytes: u64,
) -> ModelDef {
    ModelDef {
        id: id.into(),
        label: label.into(),
        size_label: size_label.into(),
        description: description.into(),
        engine: Engine::Whisper,
        languages: "multilingual".into(),
        feature_dim: None,
        files: vec![ModelFile {
            rel_path: file.into(),
            url: format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{file}"),
            bytes_hint: bytes,
        }],
    }
}

static CURATED: Lazy<Vec<ModelDef>> = Lazy::new(|| {
    vec![
        ModelDef {
            id: "gigaam-v3-e2e-ctc".into(),
            label: "GigaAM v3 (Sber)".into(),
            size_label: "~225 MB".into(),
            description: "Best for Russian: SOTA accuracy with punctuation".into(),
            engine: Engine::NemoCtc,
            languages: "ru".into(),
            feature_dim: Some(64),
            files: vec![
                ModelFile {
                    rel_path: "gigaam-v3-e2e-ctc.int8.onnx".into(),
                    url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-ctc-punct-giga-am-v3-russian-2025-12-16/resolve/main/model.int8.onnx".into(),
                    bytes_hint: 224_893_661,
                },
                ModelFile {
                    rel_path: "gigaam-v3-e2e-ctc.tokens.txt".into(),
                    url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-ctc-punct-giga-am-v3-russian-2025-12-16/resolve/main/tokens.txt".into(),
                    bytes_hint: 2_007,
                },
            ],
        },
        ModelDef {
            id: "parakeet-tdt-0.6b-v3".into(),
            label: "Parakeet TDT v3 (NVIDIA)".into(),
            size_label: "~670 MB".into(),
            description: "Best for European languages: 25 languages with punctuation".into(),
            engine: Engine::NemoTransducer,
            languages: "multilingual".into(),
            feature_dim: None,
            files: vec![
                ModelFile {
                    rel_path: "parakeet-tdt-v3.encoder.int8.onnx".into(),
                    url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/encoder.int8.onnx".into(),
                    bytes_hint: 652_184_281,
                },
                ModelFile {
                    rel_path: "parakeet-tdt-v3.decoder.int8.onnx".into(),
                    url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/decoder.int8.onnx".into(),
                    bytes_hint: 11_845_275,
                },
                ModelFile {
                    rel_path: "parakeet-tdt-v3.joiner.int8.onnx".into(),
                    url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/joiner.int8.onnx".into(),
                    bytes_hint: 6_355_277,
                },
                ModelFile {
                    rel_path: "parakeet-tdt-v3.tokens.txt".into(),
                    url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/tokens.txt".into(),
                    bytes_hint: 93_939,
                },
            ],
        },
        whisper_model(
            "tiny",
            "Tiny",
            "~75 MB",
            "Fastest, lowest accuracy",
            "ggml-tiny.bin",
            77_700_000,
        ),
        whisper_model(
            "base",
            "Base",
            "~142 MB",
            "Fast, decent for short phrases",
            "ggml-base.bin",
            147_950_000,
        ),
        whisper_model(
            "small",
            "Small",
            "~466 MB",
            "Balanced speed and accuracy",
            "ggml-small.bin",
            487_600_000,
        ),
        whisper_model(
            "medium",
            "Medium",
            "~1.5 GB",
            "Slower, high accuracy",
            "ggml-medium.bin",
            1_533_800_000,
        ),
        whisper_model(
            "large-v3-turbo-q5_0",
            "Large v3 Turbo (Q5)",
            "~547 MB",
            "Great accuracy, compact and fast",
            "ggml-large-v3-turbo-q5_0.bin",
            574_000_000,
        ),
        whisper_model(
            "large-v3-turbo",
            "Large v3 Turbo",
            "~1.6 GB",
            "Best multilingual accuracy",
            "ggml-large-v3-turbo.bin",
            1_624_600_000,
        ),
    ]
});

pub fn curated() -> &'static [ModelDef] {
    &CURATED
}

/// Resolves a model id from the curated catalog or the Hugging Face catalog.
pub fn def_by_id(id: &str) -> Option<ModelDef> {
    if id.starts_with(HF_ID_PREFIX) {
        return hf_catalog::def_by_id(id);
    }
    curated().iter().find(|d| d.id == id).cloned()
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

pub(crate) fn models_dir() -> Option<&'static Path> {
    MODELS_DIR.get().map(|p| p.as_path())
}

/// Absolute paths of all model files, in definition order, if every file exists.
pub fn resolve_paths(def: &ModelDef) -> Option<Vec<PathBuf>> {
    let dir = models_dir()?;
    let paths: Vec<PathBuf> = def.files.iter().map(|f| dir.join(&f.rel_path)).collect();
    paths.iter().all(|p| p.is_file()).then_some(paths)
}

pub fn is_downloaded(model_id: &str) -> bool {
    def_by_id(model_id)
        .and_then(|def| resolve_paths(&def))
        .is_some()
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
    curated()
        .iter()
        .map(|d| ModelInfo {
            id: d.id.clone(),
            label: d.label.clone(),
            size_label: d.size_label.clone(),
            description: d.description.clone(),
            engine: d.engine,
            languages: d.languages.clone(),
            downloaded: resolve_paths(d).is_some(),
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

    if resolve_paths(&def).is_some() {
        emit_progress(app, &model_id, 0, None, 100, true, None);
        return Ok(());
    }

    let dir = models_dir()
        .map(Path::to_path_buf)
        .ok_or_else(|| "models directory not initialized".to_string())?;

    let _guard = DOWNLOAD_LOCK.lock().await;
    if resolve_paths(&def).is_some() {
        return Ok(());
    }

    let total_hint = def.bytes_hint_total().max(1);
    emit_progress(app, &model_id, 0, None, 0, false, None);

    let mut completed_bytes: u64 = 0;
    for file in &def.files {
        let dest = dir.join(&file.rel_path);
        if dest.is_file() {
            completed_bytes += file.bytes_hint;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create model dir: {e}"))?;
        }

        let progress_app = app.clone();
        let progress_id = model_id.clone();
        let base = completed_bytes;
        let result = download_file(&file.url, &dest, move |downloaded, _total| {
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

    // Keep a descriptor next to the files so a downloaded HF model keeps
    // working even if its catalog entry disappears later.
    if model_id.starts_with(HF_ID_PREFIX) {
        if let Err(e) = hf_catalog::persist_descriptor(&model_id) {
            log::warn!("[models] persist descriptor for {model_id}: {e}");
        }
    }

    log::info!("[models] downloaded {}", def.id);
    emit_progress(app, &model_id, 0, None, 100, true, None);
    Ok(())
}

pub fn delete_model(model_id: &str) -> Result<(), String> {
    if let Some(repo_dir) = hf_catalog::download_dir(model_id) {
        if !repo_dir.is_dir() {
            return Err("model is not downloaded".to_string());
        }
        return std::fs::remove_dir_all(&repo_dir)
            .map_err(|e| format!("delete {}: {e}", repo_dir.display()));
    }

    let def = def_by_id(model_id).ok_or_else(|| format!("unknown model id: {model_id}"))?;
    let paths = resolve_paths(&def).ok_or_else(|| "model is not downloaded".to_string())?;
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
