//! Multi-family speech-to-text through a daemon-retained transcribe.cpp session.

use crate::models::{self, ModelError, ModelFile, ModelSpec};
use std::path::{Path, PathBuf};
use thiserror::Error;
use transcribe_cpp::{
    Backend, Model, ModelOptions, RunOptions, Session, SessionOptions, TimestampKind,
};

#[derive(Debug, Error)]
pub enum AsrError {
    #[error("no transcription model selected — run `sage model list`, then `sage model use <id>`")]
    NoModelSelected,
    #[error("model '{id}' is not installed at {path} — run `sage model use {id}`")]
    ModelMissing { id: String, path: PathBuf },
    #[error("failed to load model: {0}")]
    LoadModel(String),
    #[error("failed to read audio: {0}")]
    AudioRead(String),
    #[error("transcription failed: {0}")]
    Transcribe(String),
    #[error(transparent)]
    Model(#[from] ModelError),
}

pub struct Selection {
    pub model: &'static ModelSpec,
    pub file: &'static ModelFile,
    pub path: PathBuf,
    pub language: Option<String>,
}

pub fn selected() -> Result<Selection, AsrError> {
    let settings = crate::config::load().map_err(|error| AsrError::LoadModel(error.to_string()))?;
    selection_from_settings(&settings)
}

fn selection_from_settings(settings: &crate::config::Settings) -> Result<Selection, AsrError> {
    let id = settings.model.as_deref().ok_or(AsrError::NoModelSelected)?;
    let model = models::find(id)?;
    let file = models::file_for(model, settings.model_quant.as_deref())?;
    let path = models::path_for(file);
    let language =
        models::effective_language(model, &settings.language).map_err(AsrError::LoadModel)?;
    Ok(Selection {
        model,
        file,
        path,
        language,
    })
}

pub fn validate_model(path: &Path, spec: &ModelSpec, threads: u32) -> Result<(), AsrError> {
    let model = load_model(path)?;
    let architecture = model.arch();
    if architecture != spec.architecture {
        return Err(AsrError::LoadModel(format!(
            "expected architecture '{}', found '{}'",
            spec.architecture, architecture
        )));
    }
    model
        .session_with(&session_options(threads))
        .map_err(|error| AsrError::LoadModel(error.to_string()))?;
    Ok(())
}

fn load_model(path: &Path) -> Result<Model, AsrError> {
    Model::load_with(
        path,
        &ModelOptions {
            backend: Backend::Cpu,
            gpu_device: 0,
        },
    )
    .map_err(|error| AsrError::LoadModel(error.to_string()))
}

fn session_options(threads: u32) -> SessionOptions {
    SessionOptions {
        n_threads: i32::try_from(threads).unwrap_or(i32::MAX),
        ..SessionOptions::default()
    }
}

fn transcribe_with_session(
    audio_path: &Path,
    session: &mut Session,
    language: Option<String>,
) -> Result<String, AsrError> {
    let samples = load_wav_samples(audio_path)?;
    let options = RunOptions {
        timestamps: TimestampKind::None,
        language,
        ..RunOptions::default()
    };
    let transcript = session
        .run(&samples, &options)
        .map_err(|error| AsrError::Transcribe(error.to_string()))?;
    Ok(transcript
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" "))
}

fn load_wav_samples(path: &Path) -> Result<Vec<f32>, AsrError> {
    let mut reader =
        hound::WavReader::open(path).map_err(|error| AsrError::AudioRead(error.to_string()))?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels as usize;

    if channels == 0 {
        return Err(AsrError::AudioRead(
            "audio file reports zero channels".into(),
        ));
    }

    let interleaved: Vec<f32> =
        match (spec.sample_format, spec.bits_per_sample) {
            (hound::SampleFormat::Int, 1..=8) => reader
                .samples::<i8>()
                .map(|sample| sample.map(|value| f32::from(value) / i8::MAX as f32))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| AsrError::AudioRead(error.to_string()))?,
            (hound::SampleFormat::Int, 9..=16) => reader
                .samples::<i16>()
                .map(|sample| sample.map(|value| f32::from(value) / i16::MAX as f32))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| AsrError::AudioRead(error.to_string()))?,
            (hound::SampleFormat::Int, 17..=32) => {
                let maximum = ((1_i64 << (spec.bits_per_sample - 1)) - 1) as f32;
                reader
                    .samples::<i32>()
                    .map(|sample| sample.map(|value| value as f32 / maximum))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| AsrError::AudioRead(error.to_string()))?
            }
            (hound::SampleFormat::Float, _) => reader
                .samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| AsrError::AudioRead(error.to_string()))?,
            (_, bits) => {
                return Err(AsrError::AudioRead(format!(
                    "unsupported WAV bit depth: {bits}"
                )))
            }
        };

    if interleaved.is_empty() {
        return Err(AsrError::AudioRead("audio file contains no samples".into()));
    }

    let mono = if channels > 1 {
        interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        interleaved
    };

    Ok(if sample_rate == 16_000 {
        mono
    } else {
        resample_linear(&mono, sample_rate, 16_000)
    })
}

/// One serial engine is retained by the daemon and reloaded only on selection changes.
pub struct PersistentStt {
    selection_key: Option<(String, String, u32)>,
    session: Option<Session>,
}

impl PersistentStt {
    pub fn new() -> Self {
        Self {
            selection_key: None,
            session: None,
        }
    }

    pub fn transcribe(&mut self, audio_path: &Path) -> Result<String, AsrError> {
        let settings =
            crate::config::load().map_err(|error| AsrError::LoadModel(error.to_string()))?;
        let selection = selection_from_settings(&settings)?;
        if !models::is_downloaded(selection.file) {
            return Err(AsrError::ModelMissing {
                id: selection.model.id.into(),
                path: selection.path,
            });
        }

        let key = session_key(selection.model, selection.file, settings.stt_threads);
        if self.selection_key.as_ref() != Some(&key) || self.session.is_none() {
            models::verify_file(&selection.path, selection.file)?;
            let model = load_model(&selection.path)?;
            let architecture = model.arch();
            if architecture != selection.model.architecture {
                return Err(AsrError::LoadModel(format!(
                    "catalog expected architecture '{}', model reports '{}'",
                    selection.model.architecture, architecture
                )));
            }
            self.session = Some(
                model
                    .session_with(&session_options(settings.stt_threads))
                    .map_err(|error| AsrError::LoadModel(error.to_string()))?,
            );
            self.selection_key = Some(key);
        }

        transcribe_with_session(
            audio_path,
            self.session.as_mut().expect("session loaded"),
            selection.language,
        )
    }

    pub fn unload(&mut self) {
        self.session = None;
        self.selection_key = None;
    }
}

fn session_key(model: &ModelSpec, file: &ModelFile, threads: u32) -> (String, String, u32) {
    (model.id.to_string(), file.quant.to_string(), threads)
}

fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == 0 || input.is_empty() {
        return Vec::new();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = ((input.len() as f64) * ratio).round().max(1.0) as usize;
    let mut output = Vec::with_capacity(out_len);
    for index in 0..out_len {
        let source = index as f64 / ratio;
        let lower = source.floor() as usize;
        let upper = (lower + 1).min(input.len() - 1);
        let weight = (source - lower as f64) as f32;
        output.push(input[lower] * (1.0 - weight) + input[upper] * weight);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_preserves_expected_duration() {
        let samples = vec![0.25; 8_000];
        assert_eq!(resample_linear(&samples, 8_000, 16_000).len(), 16_000);
    }

    #[test]
    fn missing_model_is_reported_before_loading() {
        let model = models::find("moonshine-streaming-tiny").unwrap();
        let error = validate_model(Path::new("/no/such/model.gguf"), model, 0).unwrap_err();
        assert!(matches!(error, AsrError::LoadModel(_)));
    }

    #[test]
    fn thread_changes_invalidate_the_retained_session_key() {
        let model = models::find("moonshine-streaming-tiny").unwrap();
        let file = models::file_for(model, None).unwrap();
        assert_ne!(session_key(model, file, 0), session_key(model, file, 4));
    }
}
