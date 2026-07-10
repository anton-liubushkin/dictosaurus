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
use std::sync::{Arc, Mutex};
use tauri::Manager;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperTokenId,
};

enum Loaded {
    Whisper(WhisperContext),
    Sherpa(OfflineRecognizer),
}

type CacheSlot = Option<(PathBuf, Arc<Loaded>)>;

static CACHE: Lazy<Mutex<CacheSlot>> = Lazy::new(|| Mutex::new(None));

const SAMPLE_RATE: usize = 16_000;
/// whisper.cpp rejects clips shorter than ~1 s; pad with silence.
const MIN_SAMPLES: usize = SAMPLE_RATE + SAMPLE_RATE / 10;
/// Trailing silence so the final word is not cut off mid-decode.
const TAIL_SILENCE: usize = SAMPLE_RATE * 3 / 10;
const MAX_WHISPER_PROMPT_TOKENS: usize = 128;

pub(crate) struct TranscriptionRequest<'a> {
    pub(crate) model_id: &'a str,
    pub(crate) language: &'a str,
    pub(crate) pcm: &'a [f32],
    pub(crate) vocabulary_hints: &'a str,
}

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
    loaded(&def, &paths)?;
    Ok(true)
}

/// Loads (or reuses) the cached model.
fn loaded(def: &ModelDef, paths: &[PathBuf]) -> Result<Arc<Loaded>, String> {
    let key = &paths[0];
    let mut guard = CACHE.lock().unwrap();
    if let Some((p, loaded)) = &*guard {
        if p == key {
            return Ok(loaded.clone());
        }
    }
    // Free the previous model before loading the next one to cap peak RAM.
    *guard = None;
    let loaded = Arc::new(match def.engine {
        Engine::Whisper => Loaded::Whisper(load_whisper(key)?),
        Engine::NemoCtc => Loaded::Sherpa(load_nemo_ctc(def, paths)?),
        Engine::NemoTransducer => Loaded::Sherpa(load_nemo_transducer(def, paths)?),
    });
    *guard = Some((key.clone(), loaded.clone()));
    Ok(loaded)
}

fn loaded_by_id(model_id: &str) -> Result<Arc<Loaded>, String> {
    let def = models::def_by_id(model_id).ok_or_else(|| format!("unknown model id: {model_id}"))?;
    let paths = models::resolve_paths(&def).ok_or_else(|| "model files are missing".to_string())?;
    loaded(&def, &paths)
}

/// Transcribes 16 kHz mono f32 PCM. `language` is a whisper language code or
/// "auto" (ignored by the sherpa-onnx engines).
pub(crate) fn transcribe(request: TranscriptionRequest<'_>) -> Result<String, String> {
    let loaded = loaded_by_id(request.model_id)?;
    match &*loaded {
        Loaded::Whisper(ctx) => {
            run_whisper(ctx, request.language, request.pcm, request.vocabulary_hints)
        }
        Loaded::Sherpa(recognizer) => run_sherpa(recognizer, request.pcm),
    }
}

// --- Whisper (whisper.cpp) ---

fn load_whisper(path: &Path) -> Result<WhisperContext, String> {
    let params = WhisperContextParameters {
        use_gpu: true,
        ..Default::default()
    };
    WhisperContext::new_with_params(path, params).map_err(|e| format!("load whisper model: {e}"))
}

fn eligible_whisper_prompt(hints: &str) -> Option<&str> {
    (!hints.is_empty() && !hints.contains('\0')).then_some(hints)
}

fn whisper_tokenizer_capacity(prompt: &str) -> usize {
    prompt.len() + 1
}

fn limit_whisper_prompt_tokens(mut tokens: Vec<WhisperTokenId>) -> Vec<WhisperTokenId> {
    tokens.truncate(MAX_WHISPER_PROMPT_TOKENS);
    tokens
}

fn run_whisper(
    ctx: &WhisperContext,
    language: &str,
    pcm: &[f32],
    vocabulary_hints: &str,
) -> Result<String, String> {
    let mut samples = pcm.to_vec();
    samples.extend(std::iter::repeat_n(0.0, TAIL_SILENCE));
    if samples.len() < MIN_SAMPLES {
        samples.resize(MIN_SAMPLES, 0.0);
    }

    let mut state = ctx
        .create_state()
        .map_err(|e| format!("whisper state: {e}"))?;

    // whisper-rs 0.16 mishandles whisper.cpp's negative required-capacity return.
    // Byte length + 1 guarantees room because byte fallback cannot produce more
    // than one token per input byte. FullParams then borrows the truncated Vec
    // through state.full.
    let prompt_tokens = eligible_whisper_prompt(vocabulary_hints).and_then(|prompt| {
        match ctx.tokenize(prompt, whisper_tokenizer_capacity(prompt)) {
            Ok(tokens) => Some(limit_whisper_prompt_tokens(tokens)),
            Err(error) => {
                log::warn!(
                    "[transcribe] ignoring whisper vocabulary hints; tokenization failed: {error}"
                );
                None
            }
        }
    });

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
    if let Some(tokens) = prompt_tokens.as_deref() {
        params.set_tokens(tokens);
    }
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
//
// Current curated sherpa models deliberately ignore native vocabulary hints.
// GigaAM CTC does not support them, while Parakeet lacks the required BPE
// tokenizer assets and stable modified-beam decoding. Exact dictionary
// post-processing remains available for both models.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcription_request_preserves_all_inputs() {
        let pcm = [0.25, -0.5];
        let request = TranscriptionRequest {
            model_id: "tiny",
            language: "en",
            pcm: &pcm,
            vocabulary_hints: "Rust, New York",
        };

        assert_eq!(request.model_id, "tiny");
        assert_eq!(request.language, "en");
        assert_eq!(request.pcm, &pcm);
        assert_eq!(request.vocabulary_hints, "Rust, New York");
    }

    #[test]
    fn whisper_prompt_uses_non_empty_hints_exactly() {
        assert_eq!(
            eligible_whisper_prompt("Rust, New York"),
            Some("Rust, New York")
        );
    }

    #[test]
    fn whisper_prompt_ignores_empty_or_nul_hints() {
        assert_eq!(eligible_whisper_prompt(""), None);
        assert_eq!(eligible_whisper_prompt("Rust\0lang"), None);
    }

    #[test]
    fn whisper_prompt_tokenization_is_bounded() {
        assert_eq!(MAX_WHISPER_PROMPT_TOKENS, 128);
    }

    #[test]
    fn whisper_tokenizer_capacity_exceeds_input_byte_length() {
        for prompt in ["Rust", "Ёж", &"a".repeat(1024)] {
            assert_eq!(whisper_tokenizer_capacity(prompt), prompt.len() + 1);
            assert!(whisper_tokenizer_capacity(prompt) > prompt.len());
        }
    }

    #[test]
    fn whisper_prompt_tokens_are_truncated_after_tokenization() {
        let tokens = (0..256).collect();

        let tokens = limit_whisper_prompt_tokens(tokens);

        assert_eq!(tokens.len(), MAX_WHISPER_PROMPT_TOKENS);
        assert_eq!(tokens[0], 0);
        assert_eq!(tokens[127], 127);
    }

    #[test]
    fn parakeet_config_keeps_default_decoding_without_safe_hotword_assets() {
        let def = models::def_by_id("parakeet-tdt-0.6b-v3").unwrap();
        let config = base_sherpa_config(&def, Path::new("tokens.txt"));

        let default = OfflineRecognizerConfig::default();
        assert_eq!(config.decoding_method, default.decoding_method);
        assert_eq!(config.max_active_paths, default.max_active_paths);
        assert_eq!(config.hotwords_score, default.hotwords_score);
    }

    #[test]
    fn non_hotword_sherpa_config_keeps_default_decoding() {
        let def = models::def_by_id("gigaam-v3-e2e-ctc").unwrap();
        let config = base_sherpa_config(&def, Path::new("tokens.txt"));

        assert_eq!(config.decoding_method, None);
        assert_eq!(
            config.max_active_paths,
            OfflineRecognizerConfig::default().max_active_paths
        );
    }
}
