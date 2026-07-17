//! Speech-to-text via whisper-rs (whisper.cpp).
//!
//! Default model: `ggml-tiny.bin` under [`crate::paths::models_dir`].
//! Download with: `sage model pull`

use crate::paths::{ensure_dirs, models_dir};
use std::path::{Path, PathBuf};
use thiserror::Error;
use whisper_rs::{
    convert_integer_to_float_audio, convert_stereo_to_mono_audio, FullParams, SamplingStrategy,
    WhisperContext, WhisperContextParameters,
};

/// Default whisper.cpp GGUF/GGML model filename (small, CPU-friendly).
pub const DEFAULT_MODEL_NAME: &str = "ggml-tiny.bin";

/// Hugging Face resolve URL for the default model (ggerganov/whisper.cpp).
pub const DEFAULT_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin";

#[derive(Debug, Error)]
pub enum AsrError {
    #[error("model not found at {0} — run `sage model pull`")]
    ModelMissing(PathBuf),
    #[error("failed to load model: {0}")]
    LoadModel(String),
    #[error("failed to read audio: {0}")]
    AudioRead(String),
    #[error("transcription failed: {0}")]
    Transcribe(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolve the model file path.
///
/// Order: `SAGE_MODEL_PATH` env → `models_dir()/ggml-tiny.bin`.
pub fn default_model_path() -> PathBuf {
    if let Ok(p) = std::env::var("SAGE_MODEL_PATH") {
        return PathBuf::from(p);
    }
    models_dir().join(DEFAULT_MODEL_NAME)
}

/// Download the default tiny model if missing.
pub fn pull_default_model() -> Result<PathBuf, AsrError> {
    ensure_dirs()?;
    let dest = default_model_path();
    if dest.exists() {
        let meta = std::fs::metadata(&dest)?;
        if meta.len() > 1_000_000 {
            return Ok(dest);
        }
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    eprintln!("downloading {DEFAULT_MODEL_NAME} …");
    eprintln!("  from: {DEFAULT_MODEL_URL}");
    eprintln!("  to:   {}", dest.display());

    let tmp = dest.with_extension("bin.partial");
    let response = ureq::get(DEFAULT_MODEL_URL)
        .call()
        .map_err(|e| AsrError::Download(e.to_string()))?;

    if !(200..300).contains(&response.status()) {
        return Err(AsrError::Download(format!(
            "HTTP {} fetching model",
            response.status()
        )));
    }

    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(&tmp)?;
    std::io::copy(&mut reader, &mut file)?;
    drop(file);

    let size = std::fs::metadata(&tmp)?.len();
    if size < 1_000_000 {
        let _ = std::fs::remove_file(&tmp);
        return Err(AsrError::Download(format!(
            "downloaded file too small ({size} bytes) — check network or URL"
        )));
    }

    std::fs::rename(&tmp, &dest)?;
    eprintln!("model ready ({} MB)", size / 1_000_000);
    Ok(dest)
}

/// Transcribe a WAV file to text using the default (or env-overridden) model.
pub fn transcribe_file(audio_path: &Path) -> Result<String, AsrError> {
    let model = default_model_path();
    if !model.exists() {
        return Err(AsrError::ModelMissing(model));
    }
    transcribe_file_with_model(audio_path, &model)
}

pub fn transcribe_file_with_model(audio_path: &Path, model_path: &Path) -> Result<String, AsrError> {
    if !model_path.exists() {
        return Err(AsrError::ModelMissing(model_path.to_path_buf()));
    }
    if !audio_path.exists() {
        return Err(AsrError::AudioRead(format!(
            "file not found: {}",
            audio_path.display()
        )));
    }

    // Quiet whisper.cpp spam on stderr for normal use.
    let _ = whisper_rs::install_logging_hooks();

    let ctx = WhisperContext::new_with_params(
        model_path.to_string_lossy().as_ref(),
        WhisperContextParameters::default(),
    )
    .map_err(|e| AsrError::LoadModel(e.to_string()))?;

    let mut state = ctx
        .create_state()
        .map_err(|e| AsrError::LoadModel(e.to_string()))?;

    let samples = load_wav_as_whisper_samples(audio_path)?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(num_cpus_soft());
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    // Auto language detect; English-only tiny still works well for en.
    params.set_language(Some("auto"));

    state
        .full(params, &samples)
        .map_err(|e| AsrError::Transcribe(e.to_string()))?;

    let n = state
        .full_n_segments()
        .map_err(|e| AsrError::Transcribe(e.to_string()))?;

    let mut text = String::new();
    for i in 0..n {
        let seg = state
            .full_get_segment_text(i)
            .map_err(|e| AsrError::Transcribe(e.to_string()))?;
        text.push_str(&seg);
    }

    Ok(text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Load WAV → mono f32 @ 16 kHz (whisper.cpp requirement).
fn load_wav_as_whisper_samples(path: &Path) -> Result<Vec<f32>, AsrError> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| AsrError::AudioRead(e.to_string()))?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels as usize;

    let raw_i16: Vec<i16> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AsrError::AudioRead(e.to_string()))?,
        hound::SampleFormat::Float => {
            let floats: Vec<f32> = reader
                .samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| AsrError::AudioRead(e.to_string()))?;
            floats
                .into_iter()
                .map(|f| (f.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .collect()
        }
    };

    if raw_i16.is_empty() {
        return Err(AsrError::AudioRead("audio file contains no samples".into()));
    }

    // Interleaved i16 → f32
    let mut float = vec![0.0f32; raw_i16.len()];
    convert_integer_to_float_audio(&raw_i16, &mut float)
        .map_err(|e| AsrError::AudioRead(e.to_string()))?;

    // Stereo → mono
    let mono = if channels >= 2 {
        convert_stereo_to_mono_audio(&float).map_err(|e| AsrError::AudioRead(e.to_string()))?
    } else {
        float
    };

    // Resample to 16 kHz if needed
    let samples = if sample_rate == 16_000 {
        mono
    } else {
        resample_linear(&mono, sample_rate, 16_000)
    };

    Ok(samples)
}

fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == 0 || input.is_empty() {
        return Vec::new();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = ((input.len() as f64) * ratio).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(input.len() - 1);
        let t = (src - i0 as f64) as f32;
        let s = input[i0] * (1.0 - t) + input[i1] * t;
        out.push(s);
    }
    out
}

fn num_cpus_soft() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get().min(8) as i32)
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_path_uses_models_dir() {
        let p = default_model_path();
        assert!(p.ends_with(DEFAULT_MODEL_NAME) || std::env::var("SAGE_MODEL_PATH").is_ok());
    }

    #[test]
    fn resample_changes_length() {
        let input = vec![0.0f32; 16_000]; // 1s at 16k
        let out = resample_linear(&input, 16_000, 8_000);
        assert!((out.len() as i32 - 8_000).abs() <= 1);
    }

    #[test]
    fn transcribe_missing_model_errors_clearly() {
        let dir = std::env::temp_dir().join(format!("sage-asr-miss-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("empty.wav");
        // minimal valid-ish path that doesn't need model if missing first
        let model = dir.join("no-such-model.bin");
        let err = transcribe_file_with_model(&wav, &model).unwrap_err();
        assert!(matches!(err, AsrError::ModelMissing(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
