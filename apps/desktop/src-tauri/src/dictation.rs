//! Push-to-talk dictation state machine.
//!
//! Hotkey pressed  -> start microphone capture, show the overlay orb. With a
//!                    streaming model and live preview enabled, audio chunks
//!                    are also decoded incrementally and partial text is
//!                    streamed to the overlay.
//! Hotkey released -> stop capture, transcribe, paste the text into the
//!                    focused app (it also stays in the clipboard), hide the
//!                    overlay.

use crate::{audio, models, overlay, paste, settings::AppSettings, transcribe, AppState};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};
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
    /// Live-preview decoder; returns the final text once capture stops.
    live: Option<std::thread::JoinHandle<Option<String>>>,
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
const PARTIAL_EVENT: &str = "dictation-partial";
/// Minimum interval between partial-text emits (~10 Hz).
const PARTIAL_THROTTLE: Duration = Duration::from_millis(100);

fn emit_state(app: &AppHandle, phase: &'static str, text: Option<String>, message: Option<String>) {
    let _ = app.emit(STATE_EVENT, StatePayload {
        phase,
        text,
        message,
    });
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

    let live_enabled = settings.live_preview
        && models::def_by_id(&settings.model_id).is_some_and(|def| def.engine.is_streaming());

    {
        let mut inner = state.dictation.inner.lock().unwrap();
        // Key auto-repeat fires extra Pressed events while held — ignore them.
        if inner.phase != Phase::Idle {
            return;
        }

        let (tap, live) = if live_enabled {
            let (sample_rate_tx, sample_rate_rx) = mpsc::channel();
            let (chunk_tx, chunk_rx) = mpsc::channel();
            let tap = audio::AudioTap {
                sample_rate_tx,
                chunk_tx,
            };
            (Some(tap), Some((sample_rate_rx, chunk_rx)))
        } else {
            (None, None)
        };

        match audio::start_recording(tap) {
            Ok(handle) => {
                inner.phase = Phase::Recording;
                inner.recorder = Some(handle);
                inner.live = live.map(|(sample_rate_rx, chunk_rx)| {
                    let app = app.clone();
                    let model_id = settings.model_id.clone();
                    std::thread::Builder::new()
                        .name("live-preview".into())
                        .spawn(move || live_worker(app, model_id, sample_rate_rx, chunk_rx))
                        .expect("spawn live-preview thread")
                });
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

    let (handle, live) = {
        let mut inner = state.dictation.inner.lock().unwrap();
        if inner.phase != Phase::Recording {
            return;
        }
        inner.phase = Phase::Transcribing;
        (inner.recorder.take(), inner.live.take())
    };
    let Some(handle) = handle else {
        set_idle(app);
        return;
    };

    emit_state(app, "transcribing", None, None);

    let settings = state.settings.lock().unwrap().current().clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome =
            tauri::async_runtime::spawn_blocking(move || run_pipeline(handle, live, &settings))
                .await
                .unwrap_or_else(|e| Err(format!("transcription task failed: {e}")));

        match outcome {
            Ok(Some(text)) => {
                log::info!("[dictation] transcribed {} chars", text.chars().count());
                if let Err(e) = paste::insert_text(&app, &text) {
                    log::warn!("[dictation] paste failed (text is in clipboard): {e}");
                }
                emit_state(&app, "inserted", Some(text), None);
                // The text is already on screen; just a brief confirmation
                // flash so the orb never feels like it blocks the UI.
                finish(&app, 200).await;
            }
            Ok(None) => {
                log::info!("[dictation] nothing to insert (too short or empty)");
                emit_state(&app, "canceled", None, None);
                finish(&app, 200).await;
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

/// Blocking part: join the audio thread, then either take the live-preview
/// final result or run a batch transcription of the recorded clip.
/// Returns `Ok(None)` when there is nothing worth inserting.
fn run_pipeline(
    handle: audio::RecorderHandle,
    live: Option<std::thread::JoinHandle<Option<String>>>,
    settings: &AppSettings,
) -> Result<Option<String>, String> {
    // Stopping the recorder drops the capture stream, which closes the tap
    // channel and lets the live worker finish.
    let recorded = handle.stop()?;
    log::info!(
        "[dictation] captured {:.2}s at {} Hz",
        recorded.samples.len() as f64 / recorded.sample_rate.max(1) as f64,
        recorded.sample_rate
    );

    let min_samples = recorded.sample_rate as usize * 3 / 10;
    if recorded.samples.len() < min_samples {
        // Nothing worth inserting; still reap the worker thread.
        if let Some(live) = live {
            let _ = live.join();
        }
        return Ok(None);
    }

    let live_text = live.and_then(|worker| match worker.join() {
        Ok(text) => text,
        Err(_) => {
            log::warn!("[dictation] live preview thread panicked");
            None
        }
    });

    let text = match live_text {
        Some(text) => text,
        // Live preview was off or its decoder failed — batch-transcribe.
        None => {
            let pcm = audio::resample(&recorded.samples, recorded.sample_rate, 16_000);
            transcribe::transcribe(&settings.model_id, &settings.language, &pcm)?
        }
    };
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    Ok((!text.is_empty()).then_some(text))
}

/// Live-preview decoder thread: consumes tapped audio chunks, decodes them
/// incrementally and emits throttled partial-text events to the overlay.
/// Returns the final text once the capture stream closes the channel, or
/// `None` if the streaming decoder could not be started.
fn live_worker(
    app: AppHandle,
    model_id: String,
    sample_rate_rx: mpsc::Receiver<u32>,
    chunk_rx: mpsc::Receiver<Vec<f32>>,
) -> Option<String> {
    let session = match transcribe::LiveSession::start(&model_id) {
        Ok(session) => session,
        Err(e) => {
            // Dropping `chunk_rx` closes the tap; recording continues and the
            // release path falls back to batch transcription.
            log::warn!("[dictation] live preview unavailable: {e}");
            return None;
        }
    };
    let sample_rate = sample_rate_rx.recv().ok()?;
    log::debug!("[dictation] live preview started ({model_id}, {sample_rate} Hz)");

    let mut last_emit = Instant::now();
    let mut last_text = String::new();
    loop {
        match chunk_rx.recv_timeout(PARTIAL_THROTTLE) {
            Ok(chunk) => {
                session.feed(sample_rate, &chunk);
                // Catch up on anything queued while we were decoding.
                while let Ok(chunk) = chunk_rx.try_recv() {
                    session.feed(sample_rate, &chunk);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if last_emit.elapsed() >= PARTIAL_THROTTLE {
            let text = session.partial();
            if text != last_text {
                log::debug!("[dictation] partial: {} chars", text.chars().count());
                let _ = app.emit_to(overlay::LABEL, PARTIAL_EVENT, text.clone());
                last_text = text;
            }
            last_emit = Instant::now();
        }
    }

    let text = session.finish();
    log::debug!("[dictation] live final: {} chars", text.chars().count());
    if text != last_text {
        let _ = app.emit_to(overlay::LABEL, PARTIAL_EVENT, text.clone());
    }
    Some(text)
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
