//! In-process transcription. Dispatches by model engine:
//! - Whisper (ggml) via `whisper-rs` / whisper.cpp with Metal
//! - NeMo CTC (e.g. GigaAM v3) and NeMo transducer (e.g. Parakeet TDT) ONNX
//!   models via `sherpa-onnx`
//! - Streaming models (T-one CTC, NeMo/Nemotron streaming transducer) via the
//!   sherpa-onnx online recognizer; they also serve the plain batch path and
//!   additionally power the live-preview [`LiveSession`]
//!
//! The loaded model is cached between dictations so only the first use of a
//! model pays the load cost (a single cache slot also caps peak RAM).
//! `preload_in_background` warms the cache at startup.

use crate::models::{self, Engine, ModelDef};
use once_cell::sync::Lazy;
use sherpa_onnx::{
    OfflineNemoEncDecCtcModelConfig, OfflineRecognizer, OfflineRecognizerConfig,
    OfflineTransducerModelConfig, OnlineRecognizer, OnlineRecognizerConfig, OnlineStream,
    OnlineToneCtcModelConfig, OnlineTransducerModelConfig,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::Manager;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

enum Loaded {
    Whisper(WhisperContext),
    Sherpa(OfflineRecognizer),
    SherpaOnline(OnlineRecognizer),
}

type CacheSlot = Option<(PathBuf, Arc<Loaded>)>;

/// The cache hands out `Arc`s so a live streaming session can keep using the
/// model without holding the cache lock for the whole recording.
static CACHE: Lazy<Mutex<CacheSlot>> = Lazy::new(|| Mutex::new(None));

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
        Engine::ToneCtc | Engine::OnlineTransducer => {
            Loaded::SherpaOnline(load_online(def, paths)?)
        }
    });
    *guard = Some((key.clone(), loaded.clone()));
    Ok(loaded)
}

fn loaded_by_id(model_id: &str) -> Result<(ModelDef, Arc<Loaded>), String> {
    let def =
        models::def_by_id(model_id).ok_or_else(|| format!("unknown model id: {model_id}"))?;
    let paths =
        models::resolve_paths(&def).ok_or_else(|| "model files are missing".to_string())?;
    let loaded = loaded(&def, &paths)?;
    Ok((def, loaded))
}

/// Transcribes 16 kHz mono f32 PCM with the model `model_id`. `language` is a
/// whisper language code or "auto" (ignored by the sherpa-onnx engines).
pub fn transcribe(model_id: &str, language: &str, pcm: &[f32]) -> Result<String, String> {
    let (_, loaded) = loaded_by_id(model_id)?;
    match &*loaded {
        Loaded::Whisper(ctx) => run_whisper(ctx, language, pcm),
        Loaded::Sherpa(recognizer) => run_sherpa(recognizer, pcm),
        Loaded::SherpaOnline(recognizer) => run_sherpa_online(recognizer, pcm),
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

// --- sherpa-onnx online (streaming) models ---

fn num_threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .min(4)
}

/// T-one models take raw 8 kHz samples; `accept_waveform` resamples internally.
const T_ONE_SAMPLE_RATE: i32 = 8_000;

fn load_online(def: &ModelDef, paths: &[PathBuf]) -> Result<OnlineRecognizer, String> {
    let mut config = OnlineRecognizerConfig {
        decoding_method: Some("greedy_search".into()),
        ..Default::default()
    };
    config.model_config.num_threads = num_threads();

    match def.engine {
        Engine::ToneCtc => {
            config.feat_config.sample_rate = T_ONE_SAMPLE_RATE;
            config.model_config.t_one_ctc = OnlineToneCtcModelConfig {
                model: path_str(&paths[0]),
            };
            config.model_config.tokens = path_str(&paths[1]);
        }
        Engine::OnlineTransducer => {
            if paths.len() != 4 {
                return Err(format!("streaming transducer {} needs 4 files", def.id));
            }
            config.model_config.transducer = OnlineTransducerModelConfig {
                encoder: path_str(&paths[0]),
                decoder: path_str(&paths[1]),
                joiner: path_str(&paths[2]),
            };
            config.model_config.tokens = path_str(&paths[3]);
        }
        _ => return Err(format!("{} is not a streaming model", def.id)),
    }

    OnlineRecognizer::create(&config)
        .ok_or_else(|| format!("failed to load streaming model {}", def.id))
}

/// Batch path for streaming models (live preview off, or its final result):
/// feed the whole clip at once and drain the decoder.
fn run_sherpa_online(recognizer: &OnlineRecognizer, pcm: &[f32]) -> Result<String, String> {
    let stream = recognizer.create_stream();
    stream.accept_waveform(SAMPLE_RATE as i32, pcm);
    stream.input_finished();
    while recognizer.is_ready(&stream) {
        recognizer.decode(&stream);
    }
    Ok(recognizer
        .get_result(&stream)
        .map(|r| r.text)
        .unwrap_or_default())
}

// --- Live (incremental) decoding for the overlay preview ---

/// An incremental decode session over the cached streaming model. Feed raw
/// mono chunks at the capture sample rate (sherpa-onnx resamples internally),
/// poll `partial` for the current hypothesis, and call `finish` once the
/// recording stops to get the final text.
pub struct LiveSession {
    loaded: Arc<Loaded>,
    stream: OnlineStream,
}

impl LiveSession {
    pub fn start(model_id: &str) -> Result<LiveSession, String> {
        let (def, loaded) = loaded_by_id(model_id)?;
        let Loaded::SherpaOnline(recognizer) = &*loaded else {
            return Err(format!("{} is not a streaming model", def.id));
        };
        let stream = recognizer.create_stream();
        Ok(LiveSession { loaded, stream })
    }

    fn recognizer(&self) -> &OnlineRecognizer {
        match &*self.loaded {
            Loaded::SherpaOnline(recognizer) => recognizer,
            _ => unreachable!("checked in start"),
        }
    }

    pub fn feed(&self, sample_rate: u32, chunk: &[f32]) {
        self.stream.accept_waveform(sample_rate as i32, chunk);
    }

    /// Decodes everything that is ready and returns the current hypothesis.
    pub fn partial(&self) -> String {
        let recognizer = self.recognizer();
        while recognizer.is_ready(&self.stream) {
            recognizer.decode(&self.stream);
        }
        recognizer
            .get_result(&self.stream)
            .map(|r| r.text)
            .unwrap_or_default()
    }

    /// Flushes trailing context and returns the final text.
    pub fn finish(self) -> String {
        self.stream.input_finished();
        self.partial()
    }
}
