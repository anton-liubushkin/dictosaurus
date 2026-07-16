//! Speech model catalog and downloader.
//!
//! Engines:
//! - `whisper`          — ggml models from `ggerganov/whisper.cpp` (multilingual)
//! - `nemo_ctc`         — NeMo CTC ONNX exports for sherpa-onnx (e.g. GigaAM v3)
//! - `nemo_transducer`  — NeMo transducer ONNX exports for sherpa-onnx (e.g. Parakeet TDT)
//!
//! Models come from the curated catalog below. All files are stored under
//! `<app data>/models`; a model may consist of several files.
//! Download progress is reported via the `model-download-progress` event.

use futures_util::StreamExt;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

static MODELS_DIR: OnceLock<PathBuf> = OnceLock::new();
static DOWNLOAD_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

const PROGRESS_EVENT: &str = "model-download-progress";

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
            id: "whisper-podlodka-turbo".into(),
            label: "Whisper Podlodka Turbo".into(),
            size_label: "~874 MB".into(),
            description: "Russian-tuned Whisper: strong accuracy with punctuation".into(),
            engine: Engine::Whisper,
            languages: "ru".into(),
            feature_dim: None,
            files: vec![ModelFile {
                rel_path: "ggml-podlodka-turbo-q8_0.bin".into(),
                url: "https://huggingface.co/evilfreelancer/whisper-podlodka-turbo-GGUF/resolve/main/ggml-podlodka-turbo-q8_0.bin".into(),
                bytes_hint: 874_188_075,
            }],
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

/// Resolves a model id from the curated catalog.
pub fn def_by_id(id: &str) -> Option<ModelDef> {
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

/// Whether at least one curated model has already been downloaded.
pub fn any_model_downloaded() -> bool {
    curated().iter().any(|def| resolve_paths(def).is_some())
}

/// Relative cost of re-decoding a growing clip for the live preview.
/// Lower is faster/lighter. `None` marks models that are too heavy to
/// re-transcribe every few hundred milliseconds (they still work fine as the
/// authoritative model on release, just not for the real-time preview).
fn streaming_speed_rank(def: &ModelDef) -> Option<u32> {
    match def.engine {
        // CTC is non-autoregressive and the fastest to re-run.
        Engine::NemoCtc => Some(0),
        Engine::NemoTransducer => Some(1),
        Engine::Whisper => match def.bytes_hint_total() {
            0..=200_000_000 => Some(2),           // tiny / base
            200_000_001..=600_000_000 => Some(3), // small / large-v3-turbo-q5
            _ => None,                            // medium / large / podlodka — too slow
        },
    }
}

fn language_supported(def: &ModelDef, language: &str) -> bool {
    if def.languages == "multilingual" {
        return true;
    }
    // Unknown target language ("auto") should not penalize any model.
    if language.is_empty() || language == "auto" {
        return true;
    }
    def.languages.split(',').any(|code| code.trim() == language)
}

/// Picks the lightest *downloaded* model to drive the live preview, independent
/// of the model selected for the final transcription. Preference order:
/// language-compatible only, then fastest engine, then smallest download.
/// Returns `None` when no downloaded model is both light enough and compatible
/// with `language`.
pub fn resolve_preview_model(language: &str) -> Option<String> {
    pick_preview_model(
        curated().iter().filter(|def| resolve_paths(def).is_some()),
        language,
    )
    .map(|def| def.id.clone())
}

/// Pure selection core (FS-free) so unit tests can exercise language gating
/// without touching the models directory.
fn pick_preview_model<'a>(
    candidates: impl Iterator<Item = &'a ModelDef>,
    language: &str,
) -> Option<&'a ModelDef> {
    let mut eligible: Vec<(&ModelDef, u32)> = candidates
        .filter(|def| language_supported(def, language))
        .filter_map(|def| streaming_speed_rank(def).map(|rank| (def, rank)))
        .collect();

    eligible.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then(a.0.bytes_hint_total().cmp(&b.0.bytes_hint_total()))
    });

    eligible.first().map(|(def, _)| *def)
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

    log::info!("[models] downloaded {}", def.id);
    emit_progress(app, &model_id, 0, None, 100, true, None);
    Ok(())
}

const VAD_MODEL_FILE: &str = "silero_vad.onnx";
const VAD_MODEL_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";

/// Local path of the Silero VAD model, if it has been downloaded.
pub fn vad_model_path() -> Option<PathBuf> {
    let path = models_dir()?.join(VAD_MODEL_FILE);
    path.is_file().then_some(path)
}

/// Ensures the small (~2 MB) Silero VAD model used for live-preview speech
/// segmentation is present, downloading it once if needed.
pub async fn ensure_vad_model() -> Result<PathBuf, String> {
    let dir = models_dir()
        .map(Path::to_path_buf)
        .ok_or_else(|| "models directory not initialized".to_string())?;
    let dest = dir.join(VAD_MODEL_FILE);
    if dest.is_file() {
        return Ok(dest);
    }

    let _guard = DOWNLOAD_LOCK.lock().await;
    if dest.is_file() {
        return Ok(dest);
    }
    download_file(VAD_MODEL_URL, &dest, |_, _| {}).await?;
    log::info!("[models] downloaded Silero VAD model");
    Ok(dest)
}

/// Fetches the VAD model in the background so the live preview can use speech
/// segmentation without blocking startup. Best-effort: failures just leave the
/// preview on its timer-window fallback.
pub fn prefetch_vad_model_in_background() {
    tauri::async_runtime::spawn(async {
        if let Err(e) = ensure_vad_model().await {
            log::warn!("[models] VAD model prefetch failed: {e}");
        }
    });
}

pub fn delete_model(model_id: &str) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_catalog_does_not_include_streaming_models() {
        assert!(
            curated()
                .iter()
                .all(|model| !model.id.contains("streaming")),
            "live/streaming ASR models should not be part of the curated catalog"
        );
    }

    fn rank(model_id: &str) -> Option<u32> {
        streaming_speed_rank(&def_by_id(model_id).unwrap())
    }

    #[test]
    fn sherpa_models_rank_faster_than_whisper_for_preview() {
        assert_eq!(rank("gigaam-v3-e2e-ctc"), Some(0));
        assert_eq!(rank("parakeet-tdt-0.6b-v3"), Some(1));
        assert!(rank("tiny") > rank("gigaam-v3-e2e-ctc"));
    }

    #[test]
    fn heavy_whisper_models_are_not_eligible_for_preview() {
        assert_eq!(rank("medium"), None);
        assert_eq!(rank("large-v3-turbo"), None);
        assert_eq!(rank("whisper-podlodka-turbo"), None);
    }

    #[test]
    fn light_whisper_models_are_eligible_for_preview() {
        assert_eq!(rank("tiny"), Some(2));
        assert_eq!(rank("base"), Some(2));
        assert_eq!(rank("small"), Some(3));
        assert_eq!(rank("large-v3-turbo-q5_0"), Some(3));
    }

    #[test]
    fn language_support_matches_multilingual_auto_and_exact_codes() {
        let gigaam = def_by_id("gigaam-v3-e2e-ctc").unwrap(); // "ru"
        let parakeet = def_by_id("parakeet-tdt-0.6b-v3").unwrap(); // "multilingual"

        assert!(language_supported(&gigaam, "ru"));
        assert!(!language_supported(&gigaam, "en"));
        assert!(language_supported(&gigaam, "auto"));
        assert!(language_supported(&parakeet, "en"));
        assert!(language_supported(&parakeet, "ru"));
    }

    #[test]
    fn pick_preview_model_rejects_language_incompatible_candidates() {
        let gigaam = def_by_id("gigaam-v3-e2e-ctc").unwrap();
        assert!(pick_preview_model(std::iter::once(&gigaam), "en").is_none());
        assert_eq!(
            pick_preview_model(std::iter::once(&gigaam), "ru").map(|d| d.id.as_str()),
            Some("gigaam-v3-e2e-ctc")
        );
    }

    #[test]
    fn pick_preview_model_prefers_faster_compatible_engine() {
        let gigaam = def_by_id("gigaam-v3-e2e-ctc").unwrap();
        let parakeet = def_by_id("parakeet-tdt-0.6b-v3").unwrap();
        let tiny = def_by_id("tiny").unwrap();
        let picked =
            pick_preview_model([&gigaam, &parakeet, &tiny].into_iter(), "ru").unwrap();
        assert_eq!(picked.id, "gigaam-v3-e2e-ctc");
    }
}
