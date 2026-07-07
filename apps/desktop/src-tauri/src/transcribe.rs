//! In-process whisper.cpp transcription via `whisper-rs`.
//!
//! The loaded model context is cached between dictations so only the first
//! use of a model pays the load cost. `preload_in_background` warms the cache
//! at startup.

use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

static CTX_CACHE: Lazy<Mutex<Option<(PathBuf, WhisperContext)>>> = Lazy::new(|| Mutex::new(None));

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
        if let Some(path) = crate::models::resolve_model_path(&model_id) {
            if let Err(e) = with_context(&path, |_| Ok(())) {
                log::warn!("[transcribe] preload failed: {e}");
            } else {
                log::info!("[transcribe] preloaded model {model_id}");
            }
        }
    });
}

fn with_context<T>(
    path: &Path,
    f: impl FnOnce(&WhisperContext) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = CTX_CACHE.lock().unwrap();
    let cached = matches!(&*guard, Some((p, _)) if p == path);
    if !cached {
        // Free the previous model before loading the next one to cap peak RAM.
        *guard = None;
        let mut params = WhisperContextParameters::default();
        params.use_gpu = true;
        let ctx = WhisperContext::new_with_params(path, params)
            .map_err(|e| format!("load whisper model: {e}"))?;
        *guard = Some((path.to_path_buf(), ctx));
    }
    f(&guard.as_ref().expect("context cached above").1)
}

/// Transcribes 16 kHz mono f32 PCM. `language` is a whisper language code or
/// "auto" for detection.
pub fn transcribe(model_path: &Path, language: &str, pcm: &[f32]) -> Result<String, String> {
    let mut samples = pcm.to_vec();
    samples.extend(std::iter::repeat(0.0).take(TAIL_SILENCE));
    if samples.len() < MIN_SAMPLES {
        samples.resize(MIN_SAMPLES, 0.0);
    }

    with_context(model_path, |ctx| {
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
    })
}
