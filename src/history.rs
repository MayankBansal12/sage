//! Transcription history (phase 3).

use crate::config::Settings;
use crate::paths::{ensure_dirs, history_path};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub text: String,
    pub audio_path: Option<String>,
    pub duration_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
}

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

pub fn load() -> Result<History, HistoryError> {
    ensure_dirs()?;
    let path = history_path();
    if !path.exists() {
        return Ok(History::default());
    }
    let raw = fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(History::default());
    }
    Ok(serde_json::from_str(&raw)?)
}

pub fn save(history: &History) -> Result<(), HistoryError> {
    ensure_dirs()?;
    let path = history_path();
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(history)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn append(
    text: impl Into<String>,
    audio_path: Option<String>,
    duration_secs: Option<u64>,
    settings: &Settings,
) -> Result<HistoryEntry, HistoryError> {
    let text = text.into();
    let entry = HistoryEntry {
        id: format!("{}", Utc::now().timestamp_millis()),
        timestamp: Utc::now(),
        text,
        audio_path,
        duration_secs,
    };
    let mut hist = load()?;
    hist.entries.insert(0, entry.clone());
    if settings.max_history_entries > 0 && hist.entries.len() > settings.max_history_entries {
        hist.entries.truncate(settings.max_history_entries);
    }
    save(&hist)?;
    Ok(entry)
}

pub fn clear() -> Result<(), HistoryError> {
    save(&History::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env;

    #[test]
    fn append_and_load() {
        let _g = test_env::lock();
        let dir = std::env::temp_dir().join(format!("sage-hist-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        std::env::set_var("SAGE_DATA_DIR", &dir);
        std::env::set_var("SAGE_CONFIG_DIR", dir.join("cfg"));
        let settings = Settings {
            max_history_entries: 2,
            ..Default::default()
        };
        append("one", None, None, &settings).unwrap();
        append("two", None, None, &settings).unwrap();
        append("three", None, None, &settings).unwrap();
        let h = load().unwrap();
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.entries[0].text, "three");
        assert_eq!(h.entries[1].text, "two");
        clear().unwrap();
        assert!(load().unwrap().entries.is_empty());
        std::env::remove_var("SAGE_DATA_DIR");
        std::env::remove_var("SAGE_CONFIG_DIR");
        let _ = fs::remove_dir_all(&dir);
    }
}
