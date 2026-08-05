//! User configuration (`~/.config/sage/config.toml`).

use crate::paths::{config_file, ensure_dirs};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Selected catalog model id. `None` means setup is not complete.
    pub model: Option<String>,
    /// Selected quantization for the catalog model.
    pub model_quant: Option<String>,
    /// Language intent: `auto` or a model-supported language code.
    pub language: String,
    /// Copy transcript to clipboard after successful delivery. Default: true.
    pub copy_to_clipboard: bool,
    /// Deliver pending transcripts when their captured target regains focus.
    pub auto_paste: bool,
    /// Global toggle hotkey, e.g. "ctrl+shift+r".
    pub hotkey: String,
    /// Minimum recording duration (seconds) before transcribe; shorter = discard.
    pub minimum_key_time: f64,
    /// Speech-to-text CPU threads (0 = engine default).
    pub stt_threads: u32,
    /// Max history entries to keep (0 = unlimited).
    pub max_history_entries: usize,
    /// Play lightweight start/stop/cancel cues.
    pub sound_effects: bool,
    /// Hours a completed transcript waits for its captured target to regain focus.
    pub pending_delivery_hours: f64,
    /// Text placed between multiple transcripts delivered to the same target.
    pub pending_separator: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: None,
            model_quant: None,
            language: "auto".into(),
            copy_to_clipboard: true,
            auto_paste: true,
            hotkey: "ctrl+shift+space".into(),
            minimum_key_time: 0.2,
            stt_threads: 0,
            max_history_entries: 100,
            sound_effects: true,
            pending_delivery_hours: 2.0,
            pending_separator: " ".into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid config: {0}")]
    Parse(String),
}

pub fn load() -> Result<Settings, ConfigError> {
    ensure_dirs()?;
    let path = config_file();
    if !path.exists() {
        return Ok(Settings::default());
    }
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    let raw = fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(Settings::default());
    }
    let settings = if let Ok(settings) = toml::from_str::<Settings>(&raw) {
        settings
    } else {
        // Back-compat: JSON
        serde_json::from_str(&raw).map_err(|e| ConfigError::Parse(e.to_string()))?
    };
    validate(&settings)?;
    Ok(settings)
}

pub fn save(settings: &Settings) -> Result<(), ConfigError> {
    validate(settings)?;
    ensure_dirs()?;
    let raw = toml::to_string_pretty(settings).map_err(|e| ConfigError::Parse(e.to_string()))?;
    let path = config_file();
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, raw)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn set_key(key: &str, value: &str) -> Result<Settings, ConfigError> {
    let mut s = load()?;
    match key {
        "model" | "model_quant" => {
            return Err(ConfigError::Parse(
                "use `sage model use <id>` to change the transcription model".into(),
            ))
        }
        "language" => s.language = value.to_ascii_lowercase(),
        "copy_to_clipboard" => s.copy_to_clipboard = parse_bool(value)?,
        "auto_paste" => s.auto_paste = parse_bool(value)?,
        "hotkey" => s.hotkey = value.to_string(),
        "minimum_key_time" => {
            s.minimum_key_time = value
                .parse()
                .map_err(|_| ConfigError::Parse(format!("invalid number: {value}")))?
        }
        "stt_threads" => {
            s.stt_threads = value
                .parse()
                .map_err(|_| ConfigError::Parse(format!("invalid u32: {value}")))?
        }
        "max_history_entries" => {
            s.max_history_entries = value
                .parse()
                .map_err(|_| ConfigError::Parse(format!("invalid usize: {value}")))?
        }
        "sound_effects" => s.sound_effects = parse_bool(value)?,
        "pending_delivery_hours" => {
            let hours: f64 = value
                .parse()
                .map_err(|_| ConfigError::Parse(format!("invalid number: {value}")))?;
            if !hours.is_finite() || hours <= 0.0 || hours > 168.0 {
                return Err(ConfigError::Parse(
                    "pending_delivery_hours must be greater than 0 and at most 168".into(),
                ));
            }
            s.pending_delivery_hours = hours;
        }
        "pending_separator" => s.pending_separator = parse_separator(value)?,
        other => {
            return Err(ConfigError::Parse(format!(
                "unknown key '{other}'. Valid: language, copy_to_clipboard, auto_paste, hotkey, \
                 minimum_key_time, stt_threads, max_history_entries, sound_effects, \
                 pending_delivery_hours, pending_separator"
            )));
        }
    }
    save(&s)?;
    Ok(s)
}

fn parse_separator(value: &str) -> Result<String, ConfigError> {
    let separator = match value {
        "space" => " ".into(),
        "newline" | "\\n" => "\n".into(),
        "tab" | "\\t" => "\t".into(),
        other => other.to_string(),
    };
    if separator.chars().count() > 16 {
        return Err(ConfigError::Parse(
            "pending_separator must be at most 16 characters".into(),
        ));
    }
    Ok(separator)
}

fn parse_bool(v: &str) -> Result<bool, ConfigError> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::Parse(format!("expected bool, got '{v}'"))),
    }
}

fn validate(settings: &Settings) -> Result<(), ConfigError> {
    match (&settings.model, &settings.model_quant) {
        (None, Some(_)) => {
            return Err(ConfigError::Parse(
                "model_quant cannot be set without a model".into(),
            ))
        }
        (Some(model_id), quant) => {
            let model = crate::models::find(model_id)
                .map_err(|error| ConfigError::Parse(error.to_string()))?;
            crate::models::file_for(model, quant.as_deref())
                .map_err(|error| ConfigError::Parse(error.to_string()))?;
            crate::models::effective_language(model, &settings.language)
                .map_err(ConfigError::Parse)?;
        }
        (None, None) => {}
    }
    if settings.language != "auto"
        && (settings.language.is_empty()
            || !settings
                .language
                .chars()
                .all(|character| character.is_ascii_alphabetic() || character == '-'))
    {
        return Err(ConfigError::Parse(
            "language must be `auto` or a language code such as en or pt".into(),
        ));
    }
    if !settings.pending_delivery_hours.is_finite()
        || settings.pending_delivery_hours <= 0.0
        || settings.pending_delivery_hours > 168.0
    {
        return Err(ConfigError::Parse(
            "pending_delivery_hours must be greater than 0 and at most 168".into(),
        ));
    }
    if !settings.minimum_key_time.is_finite() || settings.minimum_key_time < 0.0 {
        return Err(ConfigError::Parse(
            "minimum_key_time must be a finite non-negative number".into(),
        ));
    }
    if settings.pending_separator.chars().count() > 16 {
        return Err(ConfigError::Parse(
            "pending_separator must be at most 16 characters".into(),
        ));
    }
    let hotkey = crate::hotkey::HotKey::parse(&settings.hotkey).map_err(ConfigError::Parse)?;
    if hotkey.key.is_none() {
        return Err(ConfigError::Parse(
            "hotkey must include a non-modifier key".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env;

    #[test]
    fn load_defaults_when_missing() {
        let _g = test_env::lock();
        let dir = std::env::temp_dir().join(format!("sage-cfg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        std::env::set_var("SAGE_CONFIG_DIR", &dir);
        std::env::set_var("SAGE_DATA_DIR", dir.join("data"));
        let s = load().unwrap();
        assert!(s.copy_to_clipboard);
        assert!(s.auto_paste);
        assert_eq!(s.language, "auto");
        assert!(s.model.is_none());
        std::env::remove_var("SAGE_CONFIG_DIR");
        std::env::remove_var("SAGE_DATA_DIR");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_and_set_key_roundtrip() {
        let _g = test_env::lock();
        let dir = std::env::temp_dir().join(format!("sage-cfg2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        std::env::set_var("SAGE_CONFIG_DIR", &dir);
        std::env::set_var("SAGE_DATA_DIR", dir.join("data"));
        let s = Settings {
            hotkey: "ctrl+r".into(),
            ..Settings::default()
        };
        save(&s).unwrap();
        let loaded = load().unwrap();
        assert_eq!(loaded.hotkey, "ctrl+r");
        set_key("auto_paste", "true").unwrap();
        assert!(load().unwrap().auto_paste);
        std::env::remove_var("SAGE_CONFIG_DIR");
        std::env::remove_var("SAGE_DATA_DIR");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_invalid_values_loaded_from_file() {
        let _g = test_env::lock();
        let dir = std::env::temp_dir().join(format!("sage-cfg-invalid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        std::env::set_var("SAGE_CONFIG_DIR", &dir);
        std::env::set_var("SAGE_DATA_DIR", dir.join("data"));
        ensure_dirs().unwrap();
        fs::write(config_file(), "pending_delivery_hours = -1\n").unwrap();
        assert!(load().is_err());
        std::env::remove_var("SAGE_CONFIG_DIR");
        std::env::remove_var("SAGE_DATA_DIR");
        let _ = fs::remove_dir_all(&dir);
    }
}
