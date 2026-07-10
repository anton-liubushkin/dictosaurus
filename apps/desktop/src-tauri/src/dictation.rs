//! Push-to-talk dictation state machine.
//!
//! Hotkey pressed  -> start microphone capture, show the overlay orb.
//! Hotkey released -> stop capture, transcribe, paste the text into the
//!                    focused app (it also stays in the clipboard), hide the
//!                    overlay.

use crate::{
    audio,
    dictionary::{DictionarySnapshot, DictionaryStore},
    models, overlay, paste,
    settings::AppSettings,
    transcribe, AppState,
};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Phase {
    #[default]
    Idle,
    Recording,
    Transcribing,
}

#[derive(Default)]
pub struct Dictation {
    inner: Mutex<Inner>,
    /// Bumped every time the overlay is shown (new session or error flash).
    /// Delayed hides capture the value and only fire if it is unchanged, so
    /// a hide scheduled by a previous session can never blank the overlay of
    /// a newer one.
    generation: AtomicU64,
}

#[derive(Default)]
struct Inner {
    phase: Phase,
    recorder: Option<audio::RecorderHandle>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatePayload {
    phase: &'static str,
    text: Option<String>,
    message: Option<String>,
}

const STATE_EVENT: &str = "dictation-state";
const LEVEL_EVENT: &str = "audio-level";
const MAX_NATIVE_VOCABULARY_HINT_BYTES: usize = 1024;

fn dictionary_snapshot(dictionary: &Mutex<DictionaryStore>) -> Option<DictionarySnapshot> {
    match dictionary.lock() {
        Ok(store) => Some(store.snapshot()),
        Err(_) => {
            log::error!(
                "[dictionary] state lock is poisoned; dictionary disabled for this request"
            );
            None
        }
    }
}

fn post_process_transcription(
    text: &str,
    dictionary: Option<&DictionarySnapshot>,
) -> Option<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let corrected = match dictionary {
        Some(snapshot) => snapshot.apply(&normalized),
        None => normalized,
    };
    (!corrected.is_empty()).then_some(corrected)
}

fn model_uses_native_vocabulary_hints(model_id: &str) -> bool {
    models::curated()
        .iter()
        .find(|model| model.id == model_id)
        .is_some_and(|model| model.engine == models::Engine::Whisper)
}

fn native_vocabulary_hints(model_id: &str, dictionary: Option<&DictionarySnapshot>) -> String {
    if !model_uses_native_vocabulary_hints(model_id) {
        return String::new();
    }

    dictionary
        .map(|snapshot| snapshot.build_hints(MAX_NATIVE_VOCABULARY_HINT_BYTES))
        .unwrap_or_default()
}

fn emit_state(app: &AppHandle, phase: &'static str, text: Option<String>, message: Option<String>) {
    let _ = app.emit(
        STATE_EVENT,
        StatePayload {
            phase,
            text,
            message,
        },
    );
}

pub fn hotkey_pressed(app: &AppHandle) {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().current().clone();

    if !models::is_downloaded(&settings.model_id) {
        crate::tray::show_settings(app);
        emit_state(
            app,
            "error",
            None,
            Some("No speech model downloaded yet. Pick one in Settings.".into()),
        );
        flash_overlay(app, 2000);
        return;
    }

    {
        let mut inner = state.dictation.inner.lock().unwrap();
        // Key auto-repeat fires extra Pressed events while held — ignore them.
        if inner.phase != Phase::Idle {
            return;
        }

        match audio::start_recording() {
            Ok(handle) => {
                inner.phase = Phase::Recording;
                inner.recorder = Some(handle);
            }
            Err(e) => {
                drop(inner);
                log::warn!("[dictation] failed to start recording: {e}");
                emit_state(app, "error", None, Some(format!("Microphone: {e}")));
                flash_overlay(app, 2000);
                return;
            }
        }
    }

    state.dictation.generation.fetch_add(1, Ordering::SeqCst);
    log::info!("[dictation] recording started");
    emit_state(app, "recording", None, None);
    overlay::show(app);
    spawn_level_task(app.clone());
}

pub fn hotkey_released(app: &AppHandle) {
    let state = app.state::<AppState>();

    let handle = {
        let mut inner = state.dictation.inner.lock().unwrap();
        if inner.phase != Phase::Recording {
            return;
        }
        inner.phase = Phase::Transcribing;
        inner.recorder.take()
    };
    let Some(handle) = handle else {
        set_idle(app);
        return;
    };

    emit_state(app, "transcribing", None, None);

    let settings = state.settings.lock().unwrap().current().clone();
    let dictionary = dictionary_snapshot(&state.dictionary);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = tauri::async_runtime::spawn_blocking(move || {
            run_pipeline(handle, &settings, dictionary)
        })
        .await
        .unwrap_or_else(|e| Err(format!("transcription task failed: {e}")));

        match outcome {
            Ok(Some(text)) => {
                log::info!("[dictation] transcribed {} chars", text.chars().count());
                if let Err(e) = paste::insert_text(&app, &text) {
                    log::warn!("[dictation] paste failed (text is in clipboard): {e}");
                }
                emit_state(&app, "inserted", Some(text), None);
                // Long enough for the mascot's slide-out animation to play
                // before the window is hidden.
                finish(&app, 450).await;
            }
            Ok(None) => {
                log::info!("[dictation] nothing to insert (too short or empty)");
                emit_state(&app, "canceled", None, None);
                finish(&app, 450).await;
            }
            Err(e) => {
                log::error!("[dictation] {e}");
                emit_state(&app, "error", None, Some(e));
                // Long enough to read that something failed.
                finish(&app, 2000).await;
            }
        }
    });
}

/// Blocking part: join the audio thread, then run a batch transcription of the
/// recorded clip.
/// Returns `Ok(None)` when there is nothing worth inserting.
fn run_pipeline(
    handle: audio::RecorderHandle,
    settings: &AppSettings,
    dictionary: Option<DictionarySnapshot>,
) -> Result<Option<String>, String> {
    let recorded = handle.stop()?;
    log::info!(
        "[dictation] captured {:.2}s at {} Hz",
        recorded.samples.len() as f64 / recorded.sample_rate.max(1) as f64,
        recorded.sample_rate
    );

    let min_samples = recorded.sample_rate as usize * 3 / 10;
    if recorded.samples.len() < min_samples {
        return Ok(None);
    }

    let pcm = audio::resample(&recorded.samples, recorded.sample_rate, 16_000);
    let vocabulary_hints = native_vocabulary_hints(&settings.model_id, dictionary.as_ref());
    let text = transcribe::transcribe(transcribe::TranscriptionRequest {
        model_id: &settings.model_id,
        language: &settings.language,
        pcm: &pcm,
        vocabulary_hints: &vocabulary_hints,
    })?;
    Ok(post_process_transcription(&text, dictionary.as_ref()))
}

fn spawn_level_task(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let level = {
                let state = app.state::<AppState>();
                let inner = state.dictation.inner.lock().unwrap();
                if inner.phase != Phase::Recording {
                    break;
                }
                inner.recorder.as_ref().map(|r| r.level()).unwrap_or(0.0)
            };
            let _ = app.emit_to(overlay::LABEL, LEVEL_EVENT, level);
            tokio::time::sleep(Duration::from_millis(33)).await;
        }
    });
}

/// Ends a session: goes idle immediately (so the hotkey can start a new
/// session right away), then hides the overlay after `delay_ms` — unless a
/// new session has shown the overlay again in the meantime.
async fn finish(app: &AppHandle, delay_ms: u64) {
    // Capture the generation BEFORE going idle: a new session can only start
    // after set_idle, and its bump must invalidate this pending hide.
    let generation = app
        .state::<AppState>()
        .dictation
        .generation
        .load(Ordering::SeqCst);
    set_idle(app);
    hide_overlay_after(app.clone(), delay_ms, generation).await;
}

fn set_idle(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.dictation.inner.lock().unwrap().phase = Phase::Idle;
}

/// Shows the overlay briefly (used for error feedback outside a session).
fn flash_overlay(app: &AppHandle, ms: u64) {
    let state = app.state::<AppState>();
    let generation = state.dictation.generation.fetch_add(1, Ordering::SeqCst) + 1;
    overlay::show(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        hide_overlay_after(app, ms, generation).await;
    });
}

/// Hides the overlay after a delay, but only if no newer session/flash has
/// shown it again while we slept (each show bumps the generation counter).
async fn hide_overlay_after(app: AppHandle, delay_ms: u64, generation: u64) {
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    let current = app
        .state::<AppState>()
        .dictation
        .generation
        .load(Ordering::SeqCst);
    if current == generation {
        overlay::hide(&app);
    } else {
        log::debug!("[dictation] skipping stale overlay hide (a new session took over)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::{
        DictionaryDocument, DictionaryEntry, DictionarySnapshot, DictionaryStore,
        DICTIONARY_VERSION,
    };
    use std::path::PathBuf;
    use std::sync::Arc;

    fn snapshot(enabled: bool, entries: Vec<DictionaryEntry>) -> DictionarySnapshot {
        DictionarySnapshot::compile(&DictionaryDocument {
            version: DICTIONARY_VERSION,
            enabled,
            entries,
        })
        .unwrap()
    }

    fn alias_entry() -> DictionaryEntry {
        DictionaryEntry {
            id: "rust".into(),
            term: "Rust".into(),
            aliases: vec!["rust lang".into()],
            enabled: true,
        }
    }

    #[test]
    fn empty_and_disabled_dictionary_leave_normalized_text_unchanged() {
        let empty = snapshot(true, Vec::new());
        let disabled = snapshot(false, vec![alias_entry()]);

        assert_eq!(
            post_process_transcription("  rust   lang  ", Some(&empty)),
            Some("rust lang".into())
        );
        assert_eq!(
            post_process_transcription("  rust   lang  ", Some(&disabled)),
            Some("rust lang".into())
        );
    }

    #[test]
    fn enabled_alias_corrects_returned_text_after_whitespace_normalization() {
        let dictionary = snapshot(true, vec![alias_entry()]);

        assert_eq!(
            post_process_transcription("  hello \n rust   lang  ", Some(&dictionary)),
            Some("hello Rust".into())
        );
    }

    #[test]
    fn native_hints_are_bounded_and_come_from_the_dictation_snapshot() {
        let dictionary = snapshot(true, vec![alias_entry()]);

        let hints = native_vocabulary_hints("tiny", Some(&dictionary));

        assert_eq!(hints, "Rust, rust lang");
        assert!(hints.len() <= MAX_NATIVE_VOCABULARY_HINT_BYTES);
        assert_eq!(dictionary.apply("rust lang"), "Rust");
    }

    #[test]
    fn whisper_request_hints_respect_the_1024_byte_bound() {
        let entries = (0..200)
            .map(|index| DictionaryEntry {
                id: index.to_string(),
                term: format!("Term{index:03}"),
                aliases: Vec::new(),
                enabled: true,
            })
            .collect();
        let dictionary = snapshot(true, entries);

        let hints = native_vocabulary_hints("tiny", Some(&dictionary));

        assert!(hints.len() > 900);
        assert!(hints.len() <= 1024);
    }

    #[test]
    fn disabled_or_missing_dictionary_has_no_native_hints() {
        let disabled = snapshot(false, vec![alias_entry()]);

        assert_eq!(native_vocabulary_hints("tiny", Some(&disabled)), "");
        assert_eq!(native_vocabulary_hints("tiny", None), "");
    }

    #[test]
    fn sherpa_models_do_not_receive_native_hints() {
        let dictionary = snapshot(true, vec![alias_entry()]);

        assert_eq!(
            native_vocabulary_hints("parakeet-tdt-0.6b-v3", Some(&dictionary)),
            ""
        );
        assert_eq!(
            native_vocabulary_hints("gigaam-v3-e2e-ctc", Some(&dictionary)),
            ""
        );
    }

    #[test]
    fn empty_transcription_returns_nothing_after_post_processing() {
        let dictionary = snapshot(true, vec![alias_entry()]);

        assert_eq!(
            post_process_transcription(" \n\t ", Some(&dictionary)),
            None
        );
    }

    #[test]
    fn poisoned_dictionary_lock_fails_closed() {
        let path = PathBuf::from(format!(
            "unused-dictionary-{}-poison.json",
            std::process::id()
        ));
        let dictionary = Arc::new(Mutex::new(DictionaryStore::from_path(path)));
        let poisoned = Arc::clone(&dictionary);
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("poison dictionary lock");
        })
        .join();

        let snapshot = dictionary_snapshot(&dictionary);

        assert!(snapshot.is_none());
        assert_eq!(
            post_process_transcription("  rust   lang  ", snapshot.as_ref()),
            Some("rust lang".into())
        );
    }
}
