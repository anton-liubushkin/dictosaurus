//! Microphone capture via cpal.
//!
//! Audio is captured at the device's native sample rate on a dedicated thread
//! (cpal streams are not `Send`), downmixed to mono, and buffered in memory.
//! A live RMS level is published through an atomic for the overlay
//! visualization. Resampling to whisper's 16 kHz happens once, after capture.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};

pub struct RecordedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub struct RecorderHandle {
    stop_tx: mpsc::Sender<()>,
    join: Option<std::thread::JoinHandle<Result<RecordedAudio, String>>>,
    level: Arc<AtomicU32>,
}

impl RecorderHandle {
    /// Latest RMS level of the incoming audio, roughly 0.0..1.0.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// Stops capture and returns the recorded audio. Blocks until the audio
    /// thread shuts down.
    pub fn stop(mut self) -> Result<RecordedAudio, String> {
        let _ = self.stop_tx.send(());
        self.join
            .take()
            .expect("stop called once")
            .join()
            .map_err(|_| "audio capture thread panicked".to_string())?
    }
}

pub fn start_recording() -> Result<RecorderHandle, String> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let level = Arc::new(AtomicU32::new(0));
    let thread_level = level.clone();

    let join = std::thread::Builder::new()
        .name("audio-capture".into())
        .spawn(move || capture_thread(stop_rx, ready_tx, thread_level))
        .map_err(|e| format!("spawn audio thread: {e}"))?;

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(RecorderHandle {
            stop_tx,
            join: Some(join),
            level,
        }),
        Ok(Err(e)) => {
            let _ = join.join();
            Err(e)
        }
        Err(_) => {
            let _ = join.join();
            Err("audio capture thread exited unexpectedly".into())
        }
    }
}

fn capture_thread(
    stop_rx: mpsc::Receiver<()>,
    ready_tx: mpsc::Sender<Result<(), String>>,
    level: Arc<AtomicU32>,
) -> Result<RecordedAudio, String> {
    let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));

    let init = (|| -> Result<(cpal::Stream, u32), String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("no default input device (microphone)")?;
        let supported = device
            .default_input_config()
            .map_err(|e| format!("query input config: {e}"))?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        let sample_rate = config.sample_rate;

        let err_fn = |e| log::warn!("[audio] stream error: {e}");

        macro_rules! stream_for {
            ($t:ty, $conv:expr) => {{
                let buf = buffer.clone();
                let level = level.clone();
                device.build_input_stream(
                    config.clone(),
                    move |data: &[$t], _| {
                        push_frames(data.iter().map($conv), channels, &buf, &level)
                    },
                    err_fn,
                    None,
                )
            }};
        }

        let stream = match sample_format {
            cpal::SampleFormat::F32 => stream_for!(f32, |s: &f32| *s),
            cpal::SampleFormat::I16 => stream_for!(i16, |s: &i16| *s as f32 / 32_768.0),
            cpal::SampleFormat::U16 => {
                stream_for!(u16, |s: &u16| (*s as f32 - 32_768.0) / 32_768.0)
            }
            cpal::SampleFormat::I32 => stream_for!(i32, |s: &i32| *s as f32 / 2_147_483_648.0),
            other => return Err(format!("unsupported sample format: {other:?}")),
        }
        .map_err(|e| format!("open input stream: {e}"))?;

        stream.play().map_err(|e| format!("start stream: {e}"))?;
        Ok((stream, sample_rate))
    })();

    match init {
        Err(e) => {
            let _ = ready_tx.send(Err(e.clone()));
            Err(e)
        }
        Ok((stream, sample_rate)) => {
            let _ = ready_tx.send(Ok(()));
            // Block until stop is requested (or the handle is dropped).
            let _ = stop_rx.recv();
            drop(stream);
            let samples = std::mem::take(&mut *buffer.lock().unwrap());
            Ok(RecordedAudio {
                samples,
                sample_rate,
            })
        }
    }
}

fn push_frames<I: Iterator<Item = f32>>(
    samples: I,
    channels: usize,
    buffer: &Mutex<Vec<f32>>,
    level: &AtomicU32,
) {
    let mut guard = buffer.lock().unwrap();
    let start = guard.len();

    if channels <= 1 {
        guard.extend(samples);
    } else {
        let mut acc = 0.0f32;
        let mut n = 0usize;
        for s in samples {
            acc += s;
            n += 1;
            if n == channels {
                guard.push(acc / channels as f32);
                acc = 0.0;
                n = 0;
            }
        }
    }

    let chunk = &guard[start..];
    if !chunk.is_empty() {
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
        level.store(rms.to_bits(), Ordering::Relaxed);
    }
}

/// Offline windowed-sinc resampler (Hann window, 48 taps). Good quality for
/// speech and dependency-free; we only resample short dictation clips once.
pub fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = (input.len() as f64 * ratio).floor() as usize;
    // Low-pass cutoff relative to the input Nyquist (only matters when downsampling).
    let cutoff = ratio.min(1.0);
    const HALF_TAPS: isize = 24;

    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let center = i as f64 / ratio;
        let left = center.floor() as isize - HALF_TAPS + 1;

        let mut acc = 0.0f64;
        let mut norm = 0.0f64;
        for j in left..(left + 2 * HALF_TAPS) {
            if j < 0 || j as usize >= input.len() {
                continue;
            }
            let x = center - j as f64;
            let sinc = if x.abs() < 1e-9 {
                1.0
            } else {
                let px = std::f64::consts::PI * x * cutoff;
                px.sin() / px
            };
            let window = 0.5 * (1.0 + (std::f64::consts::PI * x / HALF_TAPS as f64).cos());
            let coeff = sinc * window;
            acc += input[j as usize] as f64 * coeff;
            norm += coeff;
        }
        out.push(if norm.abs() > 1e-9 {
            (acc / norm) as f32
        } else {
            0.0
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_length_matches_ratio() {
        let input = vec![0.0f32; 48_000];
        let out = resample(&input, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn resample_preserves_dc() {
        let input = vec![0.5f32; 4_800];
        let out = resample(&input, 48_000, 16_000);
        let mid = out[out.len() / 2];
        assert!((mid - 0.5).abs() < 1e-3, "mid sample was {mid}");
    }
}
