//! Curated, offline model catalog and verified model downloads.

use crate::paths::{ensure_dirs, models_dir};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

pub const DEFAULT_QUANT: &str = "Q8_0";

#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    pub quant: &'static str,
    pub filename: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub family: &'static str,
    pub architecture: &'static str,
    pub repository: &'static str,
    pub revision: &'static str,
    pub base_model: &'static str,
    pub license: &'static str,
    pub languages: &'static [&'static str],
    pub supports_language_detection: bool,
    pub supports_translation: bool,
    pub supports_streaming: bool,
    pub speed_score: u8,
    pub accuracy_score: u8,
    pub recommended: bool,
    pub files: &'static [ModelFile],
}

const PARAKEET_LANGUAGES: &[&str] = &["en"];
const CANARY_LANGUAGES: &[&str] = &["en", "de", "es", "fr"];
const MOONSHINE_LANGUAGES: &[&str] = &["en"];
const QWEN_LANGUAGES: &[&str] = &[
    "zh", "en", "yue", "ar", "de", "fr", "es", "pt", "id", "it", "ko", "ru", "th", "vi", "ja",
    "tr", "hi", "ms", "nl", "sv", "da", "fi", "pl", "cs", "fil", "fa", "el", "ro", "hu", "mk",
];
const WHISPER_LANGUAGES: &[&str] = &[
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it",
    "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur",
    "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn",
    "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si",
    "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo",
    "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln",
    "ha", "ba", "jw", "su",
];

const PARAKEET_FILES: &[ModelFile] = &[ModelFile {
    quant: DEFAULT_QUANT,
    filename: "parakeet-unified-en-0.6b-Q8_0.gguf",
    size_bytes: 731_357_568,
    sha256: "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795",
}];
const CANARY_FILES: &[ModelFile] = &[ModelFile {
    quant: DEFAULT_QUANT,
    filename: "canary-180m-flash-Q8_0.gguf",
    size_bytes: 218_447_552,
    sha256: "e13c7f5d0952b056a027cfffec13e3a3a134d1608babed24f983568f141e297c",
}];
const WHISPER_FILES: &[ModelFile] = &[ModelFile {
    quant: DEFAULT_QUANT,
    filename: "whisper-medium-Q8_0.gguf",
    size_bytes: 831_538_144,
    sha256: "09e6a65e7de377aa5b10bae24608bc6f8ca2ed04b3993ef10d4a02bcd9a82adf",
}];
const MOONSHINE_FILES: &[ModelFile] = &[ModelFile {
    quant: DEFAULT_QUANT,
    filename: "moonshine-streaming-tiny-Q8_0.gguf",
    size_bytes: 50_462_816,
    sha256: "930e4622ad3a24158b91406c30c977fa6a26b34cb32d6ac3e57cfb23383a869e",
}];
const QWEN_FILES: &[ModelFile] = &[ModelFile {
    quant: DEFAULT_QUANT,
    filename: "Qwen3-ASR-0.6B-Q8_0.gguf",
    size_bytes: 850_423_456,
    sha256: "f081b2d5e23bd669d92cc331d722a8a0681943b8e6f34b48996fd5c319b5acd8",
}];

pub static CATALOG: &[ModelSpec] = &[
    ModelSpec {
        id: "parakeet-unified-en-0.6b",
        name: "Parakeet Unified EN 0.6B",
        description: "Fast and accurate English transcription",
        family: "Parakeet",
        architecture: "parakeet",
        repository: "handy-computer/parakeet-unified-en-0.6b-gguf",
        revision: "7e948f21b7bdbac698d3318db9d350f1096f3b6c",
        base_model: "nvidia/parakeet-unified-en-0.6b",
        license: "CC-BY-4.0",
        languages: PARAKEET_LANGUAGES,
        supports_language_detection: false,
        supports_translation: false,
        supports_streaming: true,
        speed_score: 79,
        accuracy_score: 90,
        recommended: true,
        files: PARAKEET_FILES,
    },
    ModelSpec {
        id: "canary-180m-flash",
        name: "Canary 180M Flash",
        description: "Small, fast model for English, German, Spanish, and French",
        family: "Canary",
        architecture: "canary",
        repository: "handy-computer/canary-180m-flash-gguf",
        revision: "b147f9dc52b59f0998e410540a84727bd86457fd",
        base_model: "nvidia/canary-180m-flash",
        license: "CC-BY-4.0",
        languages: CANARY_LANGUAGES,
        supports_language_detection: false,
        supports_translation: true,
        supports_streaming: false,
        speed_score: 98,
        accuracy_score: 88,
        recommended: true,
        files: CANARY_FILES,
    },
    ModelSpec {
        id: "whisper-medium",
        name: "Whisper Medium",
        description: "Reliable transcription and language detection across 99 languages",
        family: "Whisper",
        architecture: "whisper",
        repository: "handy-computer/whisper-medium-gguf",
        revision: "ec78f06fded51aa82cde751678b78f76f78c8b7f",
        base_model: "openai/whisper-medium",
        license: "Apache-2.0",
        languages: WHISPER_LANGUAGES,
        supports_language_detection: true,
        supports_translation: true,
        supports_streaming: false,
        speed_score: 42,
        accuracy_score: 84,
        recommended: true,
        files: WHISPER_FILES,
    },
    ModelSpec {
        id: "moonshine-streaming-tiny",
        name: "Moonshine Streaming Tiny",
        description: "Very fast and lightweight English transcription",
        family: "Moonshine",
        architecture: "moonshine_streaming",
        repository: "handy-computer/moonshine-streaming-tiny-gguf",
        revision: "85ddff612fa3a2cf40b2f745abcfa90ef82f293b",
        base_model: "UsefulSensors/moonshine-streaming-tiny",
        license: "MIT",
        languages: MOONSHINE_LANGUAGES,
        supports_language_detection: false,
        supports_translation: false,
        supports_streaming: true,
        speed_score: 100,
        accuracy_score: 74,
        recommended: false,
        files: MOONSHINE_FILES,
    },
    ModelSpec {
        id: "qwen3-asr-0.6b",
        name: "Qwen3-ASR 0.6B",
        description: "Accurate automatic transcription across 30 languages",
        family: "Qwen3-ASR",
        architecture: "qwen3_asr",
        repository: "handy-computer/Qwen3-ASR-0.6B-gguf",
        revision: "e4e16599b900eb0cb36e524514756bb92eb092b7",
        base_model: "Qwen/Qwen3-ASR-0.6B",
        license: "Apache-2.0",
        languages: QWEN_LANGUAGES,
        supports_language_detection: true,
        supports_translation: false,
        supports_streaming: false,
        speed_score: 63,
        accuracy_score: 87,
        recommended: true,
        files: QWEN_FILES,
    },
];

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("unknown model '{0}' — run `sage model list`")]
    UnknownModel(String),
    #[error("model '{model}' does not offer quantization '{quant}'")]
    UnknownQuant { model: String, quant: String },
    #[error("download failed: {0}")]
    Download(String),
    #[error("model integrity check failed for {0}")]
    Integrity(PathBuf),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn find(id: &str) -> Result<&'static ModelSpec, ModelError> {
    CATALOG
        .iter()
        .find(|model| model.id.eq_ignore_ascii_case(id))
        .ok_or_else(|| ModelError::UnknownModel(id.to_string()))
}

pub fn file_for(
    model: &'static ModelSpec,
    quant: Option<&str>,
) -> Result<&'static ModelFile, ModelError> {
    let quant = quant.unwrap_or(DEFAULT_QUANT);
    model
        .files
        .iter()
        .find(|file| file.quant.eq_ignore_ascii_case(quant))
        .ok_or_else(|| ModelError::UnknownQuant {
            model: model.id.to_string(),
            quant: quant.to_string(),
        })
}

pub fn path_for(file: &ModelFile) -> PathBuf {
    models_dir().join(file.filename)
}

/// Cheap catalog display check. Integrity is verified before activation and loading.
pub fn is_downloaded(file: &ModelFile) -> bool {
    fs::metadata(path_for(file))
        .map(|metadata| metadata.is_file() && metadata.len() == file.size_bytes)
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallationStatus {
    Missing,
    Corrupt,
    Verified,
}

pub fn installation_status(file: &ModelFile) -> InstallationStatus {
    let path = path_for(file);
    installation_status_at(&path, file)
}

fn installation_status_at(path: &Path, file: &ModelFile) -> InstallationStatus {
    let Ok(metadata) = fs::metadata(path) else {
        return InstallationStatus::Missing;
    };
    if !metadata.is_file() || metadata.len() != file.size_bytes {
        return InstallationStatus::Corrupt;
    }
    match verify_file(path, file) {
        Ok(()) => InstallationStatus::Verified,
        Err(_) => InstallationStatus::Corrupt,
    }
}

pub fn is_verified(file: &ModelFile) -> bool {
    installation_status(file) == InstallationStatus::Verified
}

pub fn effective_language(model: &ModelSpec, intent: &str) -> Result<Option<String>, String> {
    if intent == "auto" {
        if model.supports_language_detection {
            return Ok(None);
        }
        if model.languages.len() == 1 {
            return Ok(Some(model.languages[0].to_string()));
        }
        if model.languages.contains(&"en") {
            return Ok(Some("en".into()));
        }
    }
    if model.languages.contains(&intent) {
        return Ok(Some(intent.to_string()));
    }
    Err(format!(
        "model '{}' does not support language '{}'; supported: {}",
        model.id,
        intent,
        model.languages.join(", ")
    ))
}

pub fn verify_file(path: &Path, expected: &ModelFile) -> Result<(), ModelError> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() != expected.size_bytes {
        return Err(ModelError::Integrity(path.to_path_buf()));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    if format!("{:x}", hash.finalize()) != expected.sha256 {
        return Err(ModelError::Integrity(path.to_path_buf()));
    }
    Ok(())
}

pub fn pull(model: &'static ModelSpec, file: &'static ModelFile) -> Result<PathBuf, ModelError> {
    ensure_dirs()?;
    let destination = path_for(file);
    if verify_file(&destination, file).is_ok() {
        return Ok(destination);
    }

    let url = format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        model.repository, model.revision, file.filename
    );
    eprintln!(
        "downloading {} ({:.1} MiB)",
        model.name,
        mib(file.size_bytes)
    );
    eprintln!("  from: {url}");
    eprintln!("  to:   {}", destination.display());

    let partial =
        destination.with_file_name(format!(".{}.{}.partial", file.filename, std::process::id()));
    let result = download_to(&url, &partial, file);
    if let Err(error) = result {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    fs::rename(&partial, &destination)?;
    eprintln!("model ready: {}", destination.display());
    Ok(destination)
}

fn download_to(url: &str, partial: &Path, expected: &ModelFile) -> Result<(), ModelError> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(300))
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|error| ModelError::Download(error.to_string()))?;
    let mut reader = response.into_reader();
    let mut output = File::create(partial)?;
    fs::set_permissions(partial, fs::Permissions::from_mode(0o600))?;
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    let mut next_progress = 8 * 1024 * 1024;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| ModelError::Download(error.to_string()))?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        hash.update(&buffer[..read]);
        total += read as u64;
        if total >= next_progress {
            eprint!(
                "\r  {:.1}% ({:.1} / {:.1} MiB)",
                percent(total, expected.size_bytes),
                mib(total),
                mib(expected.size_bytes)
            );
            next_progress += 8 * 1024 * 1024;
        }
    }
    output.sync_all()?;
    eprintln!();
    if total != expected.size_bytes || format!("{:x}", hash.finalize()) != expected.sha256 {
        return Err(ModelError::Integrity(partial.to_path_buf()));
    }
    Ok(())
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn percent(done: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        done as f64 * 100.0 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_is_small_complete_and_unique() {
        assert_eq!(CATALOG.len(), 5);
        let mut ids = HashSet::new();
        for model in CATALOG {
            assert!(ids.insert(model.id));
            assert!(!model.languages.is_empty());
            assert!(!model.files.is_empty());
            assert!(model.files.iter().all(|file| file.sha256.len() == 64));
        }
    }

    #[test]
    fn language_resolution_respects_model_capabilities() {
        let parakeet = find("parakeet-unified-en-0.6b").unwrap();
        assert_eq!(
            effective_language(parakeet, "auto").unwrap(),
            Some("en".into())
        );
        let whisper = find("whisper-medium").unwrap();
        assert_eq!(effective_language(whisper, "auto").unwrap(), None);
        let canary = find("canary-180m-flash").unwrap();
        assert_eq!(
            effective_language(canary, "auto").unwrap(),
            Some("en".into())
        );
        assert!(effective_language(canary, "ja").is_err());
    }

    #[test]
    fn verifies_expected_size_and_sha256() {
        let path = std::env::temp_dir().join(format!("sage-model-hash-{}", std::process::id()));
        std::fs::write(&path, b"sage").unwrap();
        let expected = ModelFile {
            quant: "test",
            filename: "test.gguf",
            size_bytes: 4,
            sha256: "7e5d4325a44714fc86a9b6989e41e966a7297bfe7e913c15fb9ab19588e33d61",
        };
        verify_file(&path, &expected).unwrap();
        assert_eq!(
            installation_status_at(&path, &expected),
            InstallationStatus::Verified
        );
        let wrong = ModelFile {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            ..expected
        };
        assert!(matches!(
            verify_file(&path, &wrong),
            Err(ModelError::Integrity(_))
        ));
        assert_eq!(
            installation_status_at(&path, &wrong),
            InstallationStatus::Corrupt
        );
        assert_eq!(
            installation_status_at(Path::new("/no/such/catalog-model.gguf"), &expected),
            InstallationStatus::Missing
        );
        let _ = std::fs::remove_file(path);
    }
}
