//! In-process transcription. Dispatches by model engine:
//! - Whisper (ggml) via `whisper-rs` / whisper.cpp with Metal
//! - NeMo CTC (e.g. GigaAM v3) and NeMo transducer (e.g. Parakeet TDT) ONNX
//!   models via `sherpa-onnx`
//!
//! The loaded model is cached between dictations so only the first use of a
//! model pays the load cost (a single cache slot also caps peak RAM).
//! `preload_in_background` warms the cache at startup.

use crate::models::{self, Engine, ModelDef};
use once_cell::sync::Lazy;
use sherpa_onnx::{
    OfflineNemoEncDecCtcModelConfig, OfflineRecognizer, OfflineRecognizerConfig,
    OfflineTransducerModelConfig,
};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

enum Loaded {
    Whisper(WhisperContext),
    Sherpa(OfflineRecognizer),
}

static CACHE: Lazy<Mutex<Option<(PathBuf, Loaded)>>> = Lazy::new(|| Mutex::new(None));

const SAMPLE_RATE: usize = 16_000;
/// whisper.cpp rejects clips shorter than ~1 s; pad with silence.
const MIN_SAMPLES: usize = SAMPLE_RATE + SAMPLE_RATE / 10;
/// Trailing silence so the final word is not cut off mid-decode.
const TAIL_SILENCE: usize = SAMPLE_RATE * 3 / 10;

pub fn preload_in_background(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let model_id = {
            let state = app.state::<crate::AppState>();
            let guard = state.settings.lock().unwrap();
            guard.current().model_id.clone()
        };
        match preload(&model_id) {
            Ok(true) => log::info!("[transcribe] preloaded model {model_id}"),
            Ok(false) => {}
            Err(e) => log::warn!("[transcribe] preload failed: {e}"),
        }
    });
}

fn preload(model_id: &str) -> Result<bool, String> {
    let Some(def) = models::def_by_id(model_id) else {
        return Ok(false);
    };
    let Some(paths) = models::resolve_paths(&def) else {
        return Ok(false);
    };
    with_loaded(&def, &paths, |_| Ok(()))?;
    Ok(true)
}

/// Loads (or reuses) the model and runs `f` on it under the cache lock.
fn with_loaded<T>(
    def: &ModelDef,
    paths: &[PathBuf],
    f: impl FnOnce(&Loaded) -> Result<T, String>,
) -> Result<T, String> {
    let key = &paths[0];
    let mut guard = CACHE.lock().unwrap();
    let cached = matches!(&*guard, Some((p, _)) if p == key);
    if !cached {
        // Free the previous model before loading the next one to cap peak RAM.
        *guard = None;
        let loaded = match def.engine {
            Engine::Whisper => Loaded::Whisper(load_whisper(key)?),
            Engine::NemoCtc => Loaded::Sherpa(load_nemo_ctc(def, paths)?),
            Engine::NemoTransducer => Loaded::Sherpa(load_nemo_transducer(def, paths)?),
        };
        *guard = Some((key.clone(), loaded));
    }
    f(&guard.as_ref().expect("model cached above").1)
}

/// Transcribes 16 kHz mono f32 PCM with the model `model_id`. `language` is a
/// whisper language code or "auto" (ignored by the sherpa-onnx engines).
pub fn transcribe(model_id: &str, language: &str, pcm: &[f32]) -> Result<String, String> {
    let def =
        models::def_by_id(model_id).ok_or_else(|| format!("unknown model id: {model_id}"))?;
    let paths =
        models::resolve_paths(&def).ok_or_else(|| "model files are missing".to_string())?;

    with_loaded(&def, &paths, |loaded| match loaded {
        Loaded::Whisper(ctx) => run_whisper(ctx, language, pcm),
        Loaded::Sherpa(recognizer) => run_sherpa(recognizer, pcm),
    })
}

// --- Whisper (whisper.cpp) ---

fn load_whisper(path: &Path) -> Result<WhisperContext, String> {
    let params = WhisperContextParameters {
        use_gpu: true,
        ..Default::default()
    };
    WhisperContext::new_with_params(path, params).map_err(|e| format!("load whisper model: {e}"))
}

fn run_whisper(ctx: &WhisperContext, language: &str, pcm: &[f32]) -> Result<String, String> {
    let mut samples = pcm.to_vec();
    samples.extend(std::iter::repeat_n(0.0, TAIL_SILENCE));
    if samples.len() < MIN_SAMPLES {
        samples.resize(MIN_SAMPLES, 0.0);
    }

    let mut state = ctx
        .create_state()
        .map_err(|e| format!("whisper state: {e}"))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    match language {
        "" | "auto" => params.set_language(None),
        lang => params.set_language(Some(lang)),
    }
    params.set_translate(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    params.set_token_timestamps(false);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .min(8);
    params.set_n_threads(threads);

    state
        .full(params, &samples)
        .map_err(|e| format!("whisper inference: {e}"))?;

    let n_segments = state.full_n_segments();
    let mut text = String::new();
    for i in 0..n_segments {
        if let Some(segment) = state.get_segment(i) {
            if let Ok(seg_text) = segment.to_str_lossy() {
                text.push_str(&seg_text);
            }
        }
    }
    Ok(text)
}

// --- sherpa-onnx (NeMo CTC / NeMo transducer) ---

fn base_sherpa_config(def: &ModelDef, tokens: &Path) -> OfflineRecognizerConfig {
    let mut config = OfflineRecognizerConfig::default();
    if let Some(dim) = def.feature_dim {
        config.feat_config.feature_dim = dim;
    }
    config.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
    config.model_config.num_threads = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .min(4);
    config
}

fn path_str(path: &Path) -> Option<String> {
    Some(path.to_string_lossy().into_owned())
}

/// `paths` = [model, tokens].
fn load_nemo_ctc(def: &ModelDef, paths: &[PathBuf]) -> Result<OfflineRecognizer, String> {
    let mut config = base_sherpa_config(def, &paths[1]);
    config.model_config.nemo_ctc = OfflineNemoEncDecCtcModelConfig {
        model: path_str(&paths[0]),
    };
    OfflineRecognizer::create(&config)
        .ok_or_else(|| format!("failed to load NeMo CTC model {}", def.id))
}

/// `paths` = [encoder, decoder, joiner, tokens].
fn load_nemo_transducer(def: &ModelDef, paths: &[PathBuf]) -> Result<OfflineRecognizer, String> {
    if paths.len() != 4 {
        return Err(format!("transducer model {} needs 4 files", def.id));
    }
    let mut config = base_sherpa_config(def, &paths[3]);
    config.model_config.transducer = OfflineTransducerModelConfig {
        encoder: path_str(&paths[0]),
        decoder: path_str(&paths[1]),
        joiner: path_str(&paths[2]),
    };
    config.model_config.model_type = Some("nemo_transducer".into());
    OfflineRecognizer::create(&config)
        .ok_or_else(|| format!("failed to load NeMo transducer model {}", def.id))
}

fn run_sherpa(recognizer: &OfflineRecognizer, pcm: &[f32]) -> Result<String, String> {
    let stream = recognizer.create_stream();
    stream.accept_waveform(SAMPLE_RATE as i32, pcm);
    recognizer.decode(&stream);
    Ok(stream.get_result().map(|r| r.text).unwrap_or_default())
}
