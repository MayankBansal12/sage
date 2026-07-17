//! CLI session commands: start / stop / status / last / transcribe / model.
//!
//! `start` spawns a background worker that records until `stop`.
//! `stop` finalizes the WAV, then runs transcription (phase 2).

use crate::asr::{self, AsrError};
use crate::audio;
use crate::paths::{ensure_dirs, recordings_dir, stop_flag_path};
use crate::state::{
    clear_stop_flag, load_state, process_is_alive, reconcile_stale_state, request_stop, save_state,
    SessionState, SessionStatus, StateError,
};
use chrono::Local;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Audio(#[from] audio::AudioError),
    #[error(transparent)]
    Asr(#[from] AsrError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn cmd_start() -> Result<(), SessionError> {
    ensure_dirs()?;
    let state = reconcile_stale_state(load_state()?)?;

    if state.status == SessionStatus::Recording {
        return Err(SessionError::Message(format!(
            "already recording (worker pid {})",
            state
                .worker_pid
                .map(|p| p.to_string())
                .unwrap_or_else(|| "?".into())
        )));
    }

    clear_stop_flag()?;

    let started_at = Local::now().to_rfc3339();
    let pending = SessionState {
        status: SessionStatus::Recording,
        worker_pid: None,
        started_at: Some(started_at.clone()),
        last_recording_path: state.last_recording_path.clone(),
        last_transcript: state.last_transcript.clone(),
        last_error: None,
    };
    save_state(&pending)?;

    let exe = std::env::current_exe()?;
    let child = Command::new(exe)
        .arg("__worker")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            let _ = save_state(&SessionState {
                status: SessionStatus::Idle,
                worker_pid: None,
                started_at: None,
                last_recording_path: state.last_recording_path.clone(),
                last_transcript: state.last_transcript.clone(),
                last_error: Some(format!("failed to spawn recorder worker: {e}")),
            });
            e
        })?;

    let pid = child.id();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let current = load_state()?;
        if current.status == SessionStatus::Recording && current.worker_pid == Some(pid) {
            println!("recording started");
            println!("  pid:        {pid}");
            println!("  started_at: {started_at}");
            println!("run `sage stop` when finished speaking");
            return Ok(());
        }
        if current.status == SessionStatus::Idle {
            let err = current
                .last_error
                .unwrap_or_else(|| "recorder worker failed to start".into());
            return Err(SessionError::Message(err));
        }
        if !process_is_alive(pid) {
            let after = load_state()?;
            let err = after
                .last_error
                .unwrap_or_else(|| "recorder worker exited before becoming ready".into());
            if after.status != SessionStatus::Idle {
                let _ = save_state(&SessionState {
                    status: SessionStatus::Idle,
                    worker_pid: None,
                    started_at: None,
                    last_recording_path: after.last_recording_path,
                    last_transcript: after.last_transcript,
                    last_error: Some(err.clone()),
                });
            }
            return Err(SessionError::Message(err));
        }
        if Instant::now() > deadline {
            let _ = request_stop();
            return Err(SessionError::Message(
                "timed out waiting for recorder worker to open the microphone".into(),
            ));
        }
        thread::sleep(Duration::from_millis(40));
    }
}

pub fn cmd_stop() -> Result<(), SessionError> {
    ensure_dirs()?;
    let state = reconcile_stale_state(load_state()?)?;

    if state.status != SessionStatus::Recording {
        return Err(SessionError::Message(
            "not recording — nothing to stop".into(),
        ));
    }

    let pid = state.worker_pid;
    request_stop()?;

    // Wait for worker to finish and flip state to idle with a WAV path.
    let deadline = Instant::now() + Duration::from_secs(30);
    let wav_path = loop {
        let current = load_state()?;
        if current.status == SessionStatus::Idle {
            if let Some(err) = current.last_error {
                return Err(SessionError::Message(err));
            }
            if let Some(path) = current.last_recording_path {
                break path;
            }
            return Err(SessionError::Message(
                "recording stopped but no file path was saved".into(),
            ));
        }

        if let Some(pid) = pid {
            if !process_is_alive(pid) && current.status == SessionStatus::Recording {
                let fixed = reconcile_stale_state(current)?;
                if fixed.status == SessionStatus::Idle {
                    if let Some(err) = fixed.last_error {
                        return Err(SessionError::Message(err));
                    }
                }
            }
        }

        if Instant::now() > deadline {
            return Err(SessionError::Message(
                "timed out waiting for recorder worker to stop".into(),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    };

    println!("recording stopped");
    println!("  file: {wav_path}");

    // Phase 2: transcribe after capture.
    println!("transcribing…");
    match asr::transcribe_file(Path::new(&wav_path)) {
        Ok(text) => {
            let mut st = load_state()?;
            st.last_recording_path = Some(wav_path.clone());
            st.last_transcript = Some(text.clone());
            st.last_error = None;
            save_state(&st)?;
            println!("transcript: {text}");
            Ok(())
        }
        Err(e) => {
            let mut st = load_state()?;
            st.last_recording_path = Some(wav_path);
            st.last_error = Some(e.to_string());
            // Keep previous transcript if any.
            save_state(&st)?;
            Err(SessionError::Asr(e))
        }
    }
}

pub fn cmd_status() -> Result<(), SessionError> {
    ensure_dirs()?;
    let state = reconcile_stale_state(load_state()?)?;

    println!("status: {}", state.status);
    match state.status {
        SessionStatus::Recording => {
            if let Some(pid) = state.worker_pid {
                println!("worker_pid: {pid}");
            }
            if let Some(t) = &state.started_at {
                println!("started_at: {t}");
            }
        }
        SessionStatus::Idle => {}
    }
    match &state.last_recording_path {
        Some(p) => println!("last_recording: {p}"),
        None => println!("last_recording: (none)"),
    }
    match &state.last_transcript {
        Some(t) => println!("last_transcript: {t}"),
        None => println!("last_transcript: (none)"),
    }
    println!("model: {}", asr::default_model_path().display());
    if let Some(err) = &state.last_error {
        println!("last_error: {err}");
    }
    println!("state_file: {}", crate::paths::state_path().display());
    Ok(())
}

/// Print last transcript.
pub fn cmd_last() -> Result<(), SessionError> {
    ensure_dirs()?;
    let state = reconcile_stale_state(load_state()?)?;

    match &state.last_transcript {
        Some(text) => {
            println!("{text}");
            Ok(())
        }
        None => {
            if let Some(path) = &state.last_recording_path {
                Err(SessionError::Message(format!(
                    "no transcript yet (last recording: {path}). \
                     Run `sage model pull` then `sage transcribe {path}`"
                )))
            } else {
                Err(SessionError::Message(
                    "no transcript yet — run `sage model pull`, then `sage start` / `sage stop`"
                        .into(),
                ))
            }
        }
    }
}

/// Transcribe an existing WAV without recording.
pub fn cmd_transcribe(path: &Path) -> Result<(), SessionError> {
    ensure_dirs()?;
    if !path.exists() {
        return Err(SessionError::Message(format!(
            "file not found: {}",
            path.display()
        )));
    }
    println!("transcribing {} …", path.display());
    let text = asr::transcribe_file(path)?;
    let mut st = load_state()?;
    st.last_recording_path = Some(path.display().to_string());
    st.last_transcript = Some(text.clone());
    st.last_error = None;
    save_state(&st)?;
    println!("{text}");
    Ok(())
}

pub fn cmd_model_pull() -> Result<(), SessionError> {
    ensure_dirs()?;
    let path = asr::pull_default_model()?;
    println!("model: {}", path.display());
    Ok(())
}

pub fn cmd_model_path() -> Result<(), SessionError> {
    let path = asr::default_model_path();
    println!("{}", path.display());
    if path.exists() {
        let len = std::fs::metadata(&path)?.len();
        println!("status: present ({:.1} MB)", len as f64 / 1_000_000.0);
    } else {
        println!("status: missing — run `sage model pull`");
    }
    Ok(())
}

/// Internal entrypoint for the background recorder process.
pub fn run_worker() -> Result<(), SessionError> {
    ensure_dirs()?;
    let _ = clear_stop_flag();

    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    let out: PathBuf = recordings_dir().join(format!("recording-{stamp}.wav"));
    let pid = std::process::id();
    let prev = load_state().unwrap_or_default();
    let started_at = prev
        .started_at
        .clone()
        .unwrap_or_else(|| Local::now().to_rfc3339());
    let previous_path = prev.last_recording_path.clone();
    let previous_transcript = prev.last_transcript.clone();

    let max = Duration::from_secs(30 * 60);

    let result = audio::record_until_stop(&out, max, || {
        let armed = SessionState {
            status: SessionStatus::Recording,
            worker_pid: Some(pid),
            started_at: Some(started_at.clone()),
            last_recording_path: previous_path.clone(),
            last_transcript: previous_transcript.clone(),
            last_error: None,
        };
        save_state(&armed).map_err(|e| audio::AudioError::Io(std::io::Error::other(e.to_string())))
    });

    let _ = std::fs::remove_file(stop_flag_path());

    match result {
        Ok(path) => {
            let state = SessionState {
                status: SessionStatus::Idle,
                worker_pid: None,
                started_at: None,
                last_recording_path: Some(path.display().to_string()),
                last_transcript: previous_transcript,
                last_error: None,
            };
            save_state(&state)?;
            Ok(())
        }
        Err(e) => {
            let state = SessionState {
                status: SessionStatus::Idle,
                worker_pid: None,
                started_at: None,
                last_recording_path: previous_path,
                last_transcript: previous_transcript,
                last_error: Some(e.to_string()),
            };
            save_state(&state)?;
            Err(SessionError::Audio(e))
        }
    }
}
