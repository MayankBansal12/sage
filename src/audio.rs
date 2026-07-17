//! Microphone capture via cpal, written to a WAV file on stop.

use crate::paths::stop_flag_path;
use crate::state::stop_requested;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use hound::{WavSpec, WavWriter};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("no default input device found")]
    NoInputDevice,
    #[error("failed to get default input config: {0}")]
    DefaultConfig(String),
    #[error("failed to build input stream: {0}")]
    BuildStream(String),
    #[error("failed to start input stream: {0}")]
    PlayStream(String),
    #[error("failed to write wav: {0}")]
    WavWrite(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no audio samples captured")]
    NoSamples,
}

/// Record from the default microphone until a stop is requested (stop flag file)
/// or `max_duration` elapses. Writes a mono/stereo WAV to `output_path`.
///
/// `on_stream_ready` is invoked once the input stream is playing so the caller
/// can publish "recording armed" state before samples accumulate.
pub fn record_until_stop<F>(
    output_path: &Path,
    max_duration: Duration,
    on_stream_ready: F,
) -> Result<PathBuf, AudioError>
where
    F: FnOnce() -> Result<(), AudioError>,
{
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or(AudioError::NoInputDevice)?;

    let config = device
        .default_input_config()
        .map_err(|e| AudioError::DefaultConfig(e.to_string()))?;

    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.clone().into();
    let channels = stream_config.channels;
    let sample_rate = stream_config.sample_rate.0;

    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let err_flag = Arc::new(AtomicBool::new(false));

    let stream = match sample_format {
        SampleFormat::I16 => build_stream::<i16>(
            &device,
            &stream_config,
            samples.clone(),
            err_flag.clone(),
        )?,
        SampleFormat::U16 => build_stream::<u16>(
            &device,
            &stream_config,
            samples.clone(),
            err_flag.clone(),
        )?,
        SampleFormat::F32 => build_stream::<f32>(
            &device,
            &stream_config,
            samples.clone(),
            err_flag.clone(),
        )?,
        sample_format => {
            return Err(AudioError::BuildStream(format!(
                "unsupported sample format: {sample_format:?}"
            )));
        }
    };

    stream
        .play()
        .map_err(|e| AudioError::PlayStream(e.to_string()))?;

    on_stream_ready()?;

    let started = Instant::now();
    // Poll stop flag; also exit if the stop flag path's parent disappears (cleanup).
    while !stop_requested() && started.elapsed() < max_duration {
        if err_flag.load(Ordering::SeqCst) {
            break;
        }
        // Cheap poll — stop flag is a tiny file create from `sage stop`.
        std::thread::sleep(Duration::from_millis(40));
        // If run dir was wiped, bail.
        if !stop_flag_path()
            .parent()
            .map(|p| p.exists())
            .unwrap_or(false)
        {
            break;
        }
    }

    // Drop stream to stop callbacks before reading samples.
    drop(stream);

    let captured = samples
        .lock()
        .map_err(|_| AudioError::WavWrite("sample buffer lock poisoned".into()))?
        .clone();

    if captured.is_empty() {
        return Err(AudioError::NoSamples);
    }

    write_wav(output_path, sample_rate, channels, &captured)?;
    Ok(output_path.to_path_buf())
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: Arc<Mutex<Vec<f32>>>,
    err_flag: Arc<AtomicBool>,
) -> Result<cpal::Stream, AudioError>
where
    T: Sample + SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let err_flag_cb = err_flag.clone();
    let stream = device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let mut buf = match samples.lock() {
                    Ok(b) => b,
                    Err(_) => {
                        err_flag.store(true, Ordering::SeqCst);
                        return;
                    }
                };
                for &sample in data {
                    buf.push(f32::from_sample(sample));
                }
            },
            move |err| {
                eprintln!("sage recorder stream error: {err}");
                err_flag_cb.store(true, Ordering::SeqCst);
            },
            None,
        )
        .map_err(|e| AudioError::BuildStream(e.to_string()))?;
    Ok(stream)
}

fn write_wav(
    path: &Path,
    sample_rate: u32,
    channels: u16,
    samples: &[f32],
) -> Result<(), AudioError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let spec = WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer =
        WavWriter::create(path, spec).map_err(|e| AudioError::WavWrite(e.to_string()))?;

    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let i = (clamped * i16::MAX as f32) as i16;
        writer
            .write_sample(i)
            .map_err(|e| AudioError::WavWrite(e.to_string()))?;
    }

    writer
        .finalize()
        .map_err(|e| AudioError::WavWrite(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn write_wav_creates_readable_file() {
        let dir = std::env::temp_dir().join(format!("sage-wav-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");

        // 0.1s of 440Hz mono at 16kHz
        let sample_rate = 16_000u32;
        let mut samples = Vec::new();
        for n in 0..(sample_rate / 10) {
            let t = n as f32 / sample_rate as f32;
            samples.push((2.0 * PI * 440.0 * t).sin() * 0.2);
        }

        write_wav(&path, sample_rate, 1, &samples).expect("write wav");
        assert!(path.exists());
        let meta = std::fs::metadata(&path).unwrap();
        assert!(meta.len() > 100, "wav too small: {}", meta.len());

        let reader = hound::WavReader::open(&path).expect("open wav");
        assert_eq!(reader.spec().sample_rate, sample_rate);
        assert_eq!(reader.spec().channels, 1);
        let count = reader.into_samples::<i16>().count();
        assert_eq!(count, samples.len());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
