//! User configuration (phase 0 skeleton; expanded in later phases).
//!
//! File: `~/.config/sage/config.toml` (see [`crate::paths::config_file`]).
//!
//! Phase 1 does not require a config file — defaults are used if missing.

#![allow(dead_code)]

use crate::paths::{config_file, ensure_dirs};
use serde::{Deserialize, Serialize};
use std::fs;
use thiserror::Error;

/// Sage settings. Fields will grow with phases (hotkey, clipboard, model, …).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    /// Reserved for phase 2+: path or name of the STT model.
    pub model: Option<String>,
    /// Reserved for phase 4+: copy transcript to clipboard after stop.
    pub copy_to_clipboard: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: None,
            copy_to_clipboard: true,
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

/// Load settings from disk, or defaults if the file does not exist.
///
/// Phase 0/1: TOML parsing is minimal — if the file exists but is empty/invalid,
/// we fall back to defaults and do not crash the CLI.
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
    // Phase 0: no toml crate yet — accept only empty/missing file, or JSON for tests.
    // Full TOML lands when settings grow (phase 4+). For now, try JSON as a simple format.
    if let Ok(settings) = serde_json::from_str::<Settings>(&raw) {
        return Ok(settings);
    }
    // Unknown format: keep defaults rather than failing hard in phase 1.
    Ok(Settings::default())
}

/// Write default config as JSON (placeholder until TOML is wired).
pub fn save_defaults() -> Result<Settings, ConfigError> {
    ensure_dirs()?;
    let settings = Settings::default();
    let path = config_file();
    // Use .toml extension later; for now write JSON next to planned path if needed.
    // Prefer writing config.toml with a simple hand-written stub for readability.
    let stub = "\
# Sage configuration (phase 0/1 stub)
# model = \"path/or/name\"   # phase 2+
# copy_to_clipboard = true  # phase 4+
";
    fs::write(&path, stub)?;
    let _ = settings;
    Ok(Settings::default())
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
        assert_eq!(s, Settings::default());
        std::env::remove_var("SAGE_CONFIG_DIR");
        std::env::remove_var("SAGE_DATA_DIR");
        let _ = fs::remove_dir_all(&dir);
    }
}
