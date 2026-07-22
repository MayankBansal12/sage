//! User configuration (`~/.config/sage/config.toml`).

use crate::paths::{config_file, ensure_dirs};
use serde::{Deserialize, Serialize};
use std::fs;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Optional absolute path to a whisper ggml model.
    pub model: Option<String>,
    /// Copy transcript to clipboard after success (phase 4). Default: true.
    pub copy_to_clipboard: bool,
    /// Simulate Ctrl+V after copy (phase 6). Default: false.
    pub auto_paste: bool,
    /// Hotkey chord for daemon mode, e.g. "ctrl+shift+r" (phase 5).
    pub hotkey: String,
    /// Double-tap lock enabled (phase 5 semantics).
    pub double_tap_lock: bool,
    /// Minimum recording duration (seconds) before transcribe; shorter = discard.
    pub minimum_key_time: f64,
    /// Whisper decode threads (0 = auto).
    pub whisper_threads: u32,
    /// Max history entries to keep (0 = unlimited).
    pub max_history_entries: usize,
    /// Play terminal bell on start/stop/cancel (phase 7 lightweight sounds).
    pub sound_effects: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: None,
            copy_to_clipboard: true,
            auto_paste: false,
            hotkey: "ctrl+shift+space".into(),
            double_tap_lock: true,
            minimum_key_time: 0.2,
            whisper_threads: 0,
            max_history_entries: 100,
            sound_effects: true,
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
    let raw = fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(Settings::default());
    }
    if let Ok(s) = toml::from_str::<Settings>(&raw) {
        return Ok(s);
    }
    // Back-compat: JSON
    serde_json::from_str(&raw).map_err(|e| ConfigError::Parse(e.to_string()))
}

pub fn save(settings: &Settings) -> Result<(), ConfigError> {
    ensure_dirs()?;
    let raw = toml::to_string_pretty(settings).map_err(|e| ConfigError::Parse(e.to_string()))?;
    let path = config_file();
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, raw)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn set_key(key: &str, value: &str) -> Result<Settings, ConfigError> {
    let mut s = load()?;
    match key {
        "model" => {
            s.model = if value.is_empty() || value == "none" {
                None
            } else {
                Some(value.to_string())
            };
        }
        "copy_to_clipboard" => s.copy_to_clipboard = parse_bool(value)?,
        "auto_paste" => s.auto_paste = parse_bool(value)?,
        "hotkey" => s.hotkey = value.to_string(),
        "double_tap_lock" => s.double_tap_lock = parse_bool(value)?,
        "minimum_key_time" => {
            s.minimum_key_time = value
                .parse()
                .map_err(|_| ConfigError::Parse(format!("invalid number: {value}")))?
        }
        "whisper_threads" => {
            s.whisper_threads = value
                .parse()
                .map_err(|_| ConfigError::Parse(format!("invalid u32: {value}")))?
        }
        "max_history_entries" => {
            s.max_history_entries = value
                .parse()
                .map_err(|_| ConfigError::Parse(format!("invalid usize: {value}")))?
        }
        "sound_effects" => s.sound_effects = parse_bool(value)?,
        other => {
            return Err(ConfigError::Parse(format!(
                "unknown key '{other}'. Valid: model, copy_to_clipboard, auto_paste, hotkey, \
                 double_tap_lock, minimum_key_time, whisper_threads, max_history_entries, sound_effects"
            )));
        }
    }
    save(&s)?;
    Ok(s)
}

fn parse_bool(v: &str) -> Result<bool, ConfigError> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::Parse(format!("expected bool, got '{v}'"))),
    }
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
        assert!(!s.auto_paste);
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
        let mut s = Settings::default();
        s.hotkey = "ctrl+r".into();
        save(&s).unwrap();
        let loaded = load().unwrap();
        assert_eq!(loaded.hotkey, "ctrl+r");
        set_key("auto_paste", "true").unwrap();
        assert!(load().unwrap().auto_paste);
        std::env::remove_var("SAGE_CONFIG_DIR");
        std::env::remove_var("SAGE_DATA_DIR");
        let _ = fs::remove_dir_all(&dir);
    }
}
