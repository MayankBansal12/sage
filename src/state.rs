//! On-disk session state shared between `sage start`, `sage stop`, and the recorder worker.

use crate::paths::{ensure_dirs, state_path, stop_flag_path};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Idle,
    Recording,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Idle => write!(f, "idle"),
            SessionStatus::Recording => write!(f, "recording"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub status: SessionStatus,
    /// PID of the background recorder worker (when status == Recording).
    pub worker_pid: Option<u32>,
    pub started_at: Option<String>,
    pub last_recording_path: Option<String>,
    /// Last successful transcript (phase 2+).
    #[serde(default)]
    pub last_transcript: Option<String>,
    pub last_error: Option<String>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            status: SessionStatus::Idle,
            worker_pid: None,
            started_at: None,
            last_recording_path: None,
            last_transcript: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse state file: {0}")]
    Parse(#[from] serde_json::Error),
}

pub fn load_state() -> Result<SessionState, StateError> {
    ensure_dirs()?;
    let path = state_path();
    if !path.exists() {
        return Ok(SessionState::default());
    }
    let raw = fs::read_to_string(&path)?;
    if raw.trim().is_empty() {
        return Ok(SessionState::default());
    }
    Ok(serde_json::from_str(&raw)?)
}

pub fn save_state(state: &SessionState) -> Result<(), StateError> {
    ensure_dirs()?;
    let path = state_path();
    let tmp = path.with_extension("json.tmp");
    let raw = serde_json::to_string_pretty(state)?;
    fs::write(&tmp, raw)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn clear_stop_flag() -> Result<(), StateError> {
    let path = stop_flag_path();
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn request_stop() -> Result<(), StateError> {
    ensure_dirs()?;
    fs::write(stop_flag_path(), b"stop\n")?;
    Ok(())
}

pub fn stop_requested() -> bool {
    stop_flag_path().exists()
}

/// Returns true if a process with the given PID appears to be alive.
pub fn process_is_alive(pid: u32) -> bool {
    let path = PathBuf::from(format!("/proc/{pid}"));
    path.exists()
}

/// If state says recording but the worker is dead, reset to idle.
pub fn reconcile_stale_state(mut state: SessionState) -> Result<SessionState, StateError> {
    if state.status == SessionStatus::Recording {
        match state.worker_pid {
            Some(pid) if process_is_alive(pid) => {}
            Some(pid) => {
                state.status = SessionStatus::Idle;
                state.worker_pid = None;
                state.started_at = None;
                state.last_error = Some(format!(
                    "recorder worker (pid {pid}) exited unexpectedly; state reset to idle"
                ));
                save_state(&state)?;
                let _ = clear_stop_flag();
            }
            None => {
                state.status = SessionStatus::Idle;
                state.started_at = None;
                state.last_error =
                    Some("recording state had no worker pid; state reset to idle".into());
                save_state(&state)?;
                let _ = clear_stop_flag();
            }
        }
    }
    Ok(state)
}
