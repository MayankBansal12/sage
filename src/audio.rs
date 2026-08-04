//! Microphone capture via cpal, written to a WAV file on stop.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use hound::{WavSpec, WavWriter};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
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
    #[error("audio stream failed: {0}")]
    Stream(String),
}

#[derive(Debug, Clone)]
pub struct RecordingInfo {
    pub path: PathBuf,
    pub duration: Duration,
}

/// Stream microphone samples directly to disk until `stop` becomes true.
pub fn record_until_signal<F>(
    output_path: &Path,
    max_duration: Duration,
    stop: Arc<AtomicBool>,
    on_stream_ready: F,
) -> Result<RecordingInfo, AudioError>
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

    let err_flag = Arc::new(AtomicBool::new(false));

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let spec = WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let writer = WavWriter::create(output_path, spec)
        .map_err(|error| AudioError::WavWrite(error.to_string()))?;
    std::fs::set_permissions(output_path, std::fs::Permissions::from_mode(0o600))?;
    let (sample_tx, sample_rx) = mpsc::sync_channel::<Vec<i16>>(32);
    let writer_thread = std::thread::spawn(move || write_chunks(writer, sample_rx));

    let stream = match sample_format {
        SampleFormat::I16 => {
            build_stream::<i16>(&device, &stream_config, sample_tx.clone(), err_flag.clone())?
        }
        SampleFormat::U16 => {
            build_stream::<u16>(&device, &stream_config, sample_tx.clone(), err_flag.clone())?
        }
        SampleFormat::F32 => {
            build_stream::<f32>(&device, &stream_config, sample_tx.clone(), err_flag.clone())?
        }
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
    while !stop.load(Ordering::SeqCst) && started.elapsed() < max_duration {
        if err_flag.load(Ordering::SeqCst) {
            break;
        }
        // Cheap poll — stop flag is a tiny file create from `sage stop`.
        std::thread::sleep(Duration::from_millis(40));
    }

    // Drop stream to stop callbacks before reading samples.
    drop(stream);
    drop(sample_tx);
    let written = writer_thread
        .join()
        .map_err(|_| AudioError::WavWrite("WAV writer thread panicked".into()))??;

    if err_flag.load(Ordering::SeqCst) {
        return Err(AudioError::Stream(
            "capture callback failed or the disk writer could not keep up".into(),
        ));
    }
    if written == 0 {
        return Err(AudioError::NoSamples);
    }
    let frames = written as f64 / f64::from(channels);
    let duration = Duration::from_secs_f64(frames / f64::from(sample_rate));
    Ok(RecordingInfo {
        path: output_path.to_path_buf(),
        duration,
    })
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sender: mpsc::SyncSender<Vec<i16>>,
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
                let converted = data
                    .iter()
                    .map(|&sample| {
                        let value = f32::from_sample(sample).clamp(-1.0, 1.0);
                        (value * i16::MAX as f32) as i16
                    })
                    .collect();
                if sender.try_send(converted).is_err() {
                    err_flag.store(true, Ordering::SeqCst);
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

fn write_chunks(
    mut writer: WavWriter<std::io::BufWriter<std::fs::File>>,
    receiver: mpsc::Receiver<Vec<i16>>,
) -> Result<u64, AudioError> {
    let mut written = 0u64;
    for chunk in receiver {
        for sample in chunk {
            writer
                .write_sample(sample)
                .map_err(|error| AudioError::WavWrite(error.to_string()))?;
            written += 1;
        }
    }
    writer
        .finalize()
        .map_err(|error| AudioError::WavWrite(error.to_string()))?;
    Ok(written)
}

#[cfg(test)]
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
