//! CLI session commands: record, stop, cancel, transcribe, deliver.

use crate::asr::{self, AsrError};
use crate::audio;
use crate::config::{self, ConfigError};
use crate::decision::{self, RecordingDecision};
use crate::feedback::{self, Cue};
use crate::history::{self, HistoryError};
use crate::hotkey::HotKey;
use crate::output;
use crate::paths::{
    cancel_flag_path, ensure_dirs, recordings_dir, stop_flag_path,
};
use crate::state::{
    clear_stop_flag, load_state, process_is_alive, reconcile_stale_state, request_stop, save_state,
    SessionState, SessionStatus, StateError,
};
use chrono::Local;
use std::fs;
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
    Config(#[from] ConfigError),
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn clear_cancel_flag() {
    let _ = fs::remove_file(cancel_flag_path());
}

fn request_cancel() -> Result<(), SessionError> {
    ensure_dirs()?;
    fs::write(cancel_flag_path(), b"cancel\n")?;
    // Also stop the worker so it exits the capture loop.
    request_stop()?;
    Ok(())
}

pub fn cancel_requested() -> bool {
    cancel_flag_path().exists()
}

pub fn cmd_start() -> Result<(), SessionError> {
    ensure_dirs()?;
    let state = reconcile_stale_state(load_state()?)?;
    let settings = config::load().unwrap_or_default();

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
    clear_cancel_flag();

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
            feedback::play(Cue::Start, &settings);
            println!("recording started");
            println!("  pid:        {pid}");
            println!("  started_at: {started_at}");
            println!("run `sage stop` when finished (or `sage cancel`)");
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
    let settings = config::load().unwrap_or_default();
    let state = reconcile_stale_state(load_state()?)?;

    if state.status != SessionStatus::Recording {
        return Err(SessionError::Message(
            "not recording — nothing to stop".into(),
        ));
    }

    let started_at = state.started_at.clone();
    let pid = state.worker_pid;
    clear_cancel_flag();
    request_stop()?;

    let deadline = Instant::now() + Duration::from_secs(30);
    let wav_path = loop {
        let current = load_state()?;
        if current.status == SessionStatus::Idle {
            if let Some(err) = &current.last_error {
                // cancel path sets a specific error
                if err == "cancelled" {
                    feedback::play(Cue::Cancel, &settings);
                    println!("recording cancelled");
                    return Ok(());
                }
                return Err(SessionError::Message(err.clone()));
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

    let duration_secs = started_at
        .as_ref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| {
            let now = chrono::Local::now();
            (now.timestamp() - t.timestamp()).max(0) as f64
        })
        .unwrap_or(0.0);

    let hotkey = HotKey::parse(&settings.hotkey).unwrap_or_else(|_| {
        HotKey::parse("ctrl+shift+space").expect("default")
    });
    if decision::decide(&hotkey, settings.minimum_key_time, duration_secs)
        == RecordingDecision::DiscardShortRecording
    {
        println!(
            "discarded short recording ({duration_secs:.2}s < min {:.2}s)",
            settings.minimum_key_time
        );
        let _ = fs::remove_file(&wav_path);
        feedback::play(Cue::Cancel, &settings);
        return Ok(());
    }

    finalize_transcript(Path::new(&wav_path), Some(duration_secs as u64), &settings)
}

pub fn cmd_cancel() -> Result<(), SessionError> {
    ensure_dirs()?;
    let settings = config::load().unwrap_or_default();
    let state = reconcile_stale_state(load_state()?)?;
    if state.status != SessionStatus::Recording {
        return Err(SessionError::Message(
            "not recording — nothing to cancel".into(),
        ));
    }
    request_cancel()?;
    let pid = state.worker_pid;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let current = load_state()?;
        if current.status == SessionStatus::Idle {
            // Remove cancelled audio if present and marked cancelled
            if let Some(path) = &current.last_recording_path {
                if current.last_error.as_deref() == Some("cancelled") {
                    let _ = fs::remove_file(path);
                }
            }
            feedback::play(Cue::Cancel, &settings);
            println!("recording cancelled");
            return Ok(());
        }
        if let Some(pid) = pid {
            if !process_is_alive(pid) {
                let _ = reconcile_stale_state(current)?;
            }
        }
        if Instant::now() > deadline {
            return Err(SessionError::Message("timed out waiting for cancel".into()));
        }
        thread::sleep(Duration::from_millis(40));
    }
}

fn finalize_transcript(
    wav_path: &Path,
    duration_secs: Option<u64>,
    settings: &config::Settings,
) -> Result<(), SessionError> {
    println!("transcribing…");
    match asr::transcribe_file(wav_path) {
        Ok(text) => {
            let mut st = load_state()?;
            st.last_recording_path = Some(wav_path.display().to_string());
            st.last_transcript = Some(text.clone());
            st.last_error = None;
            save_state(&st)?;

            history::append(
                text.clone(),
                Some(wav_path.display().to_string()),
                duration_secs,
                settings,
            )?;

            for note in output::deliver(&text, settings.copy_to_clipboard, settings.auto_paste) {
                println!("  {note}");
            }

            feedback::play(Cue::Stop, settings);
            println!("transcript: {text}");
            Ok(())
        }
        Err(e) => {
            let mut st = load_state()?;
            st.last_recording_path = Some(wav_path.display().to_string());
            st.last_error = Some(e.to_string());
            save_state(&st)?;
            feedback::play(Cue::Error, settings);
            Err(SessionError::Asr(e))
        }
    }
}

pub fn cmd_status() -> Result<(), SessionError> {
    ensure_dirs()?;
    let state = reconcile_stale_state(load_state()?)?;
    let settings = config::load().unwrap_or_default();

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
    println!(
        "settings: copy={} paste={} hotkey={}",
        settings.copy_to_clipboard, settings.auto_paste, settings.hotkey
    );
    if let Some(err) = &state.last_error {
        println!("last_error: {err}");
    }
    println!("state_file: {}", crate::paths::state_path().display());
    Ok(())
}

pub fn cmd_last() -> Result<(), SessionError> {
    ensure_dirs()?;
    let state = reconcile_stale_state(load_state()?)?;
    match &state.last_transcript {
        Some(text) => {
            println!("{text}");
            Ok(())
        }
        None => Err(SessionError::Message(
            "no transcript yet — run `sage model pull`, then record or `sage transcribe`".into(),
        )),
    }
}

pub fn cmd_copy_last() -> Result<(), SessionError> {
    ensure_dirs()?;
    let state = reconcile_stale_state(load_state()?)?;
    let text = state
        .last_transcript
        .ok_or_else(|| SessionError::Message("no transcript to copy".into()))?;
    output::copy_to_clipboard(&text).map_err(|e| SessionError::Message(e.to_string()))?;
    println!("copied to clipboard ({} chars)", text.len());
    Ok(())
}

pub fn cmd_transcribe(path: &Path) -> Result<(), SessionError> {
    ensure_dirs()?;
    let settings = config::load().unwrap_or_default();
    if !path.exists() {
        return Err(SessionError::Message(format!(
            "file not found: {}",
            path.display()
        )));
    }
    println!("transcribing {} …", path.display());
    finalize_transcript(path, None, &settings)
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

pub fn cmd_history_list(limit: usize) -> Result<(), SessionError> {
    let hist = history::load()?;
    if hist.entries.is_empty() {
        println!("(no history)");
        return Ok(());
    }
    for (i, e) in hist.entries.iter().take(limit).enumerate() {
        let preview: String = e.text.chars().take(80).collect();
        println!(
            "{}. [{}] {}",
            i + 1,
            e.timestamp.format("%Y-%m-%d %H:%M"),
            preview
        );
    }
    Ok(())
}

pub fn cmd_history_clear() -> Result<(), SessionError> {
    history::clear()?;
    println!("history cleared");
    Ok(())
}

pub fn cmd_config_show() -> Result<(), SessionError> {
    let s = config::load()?;
    let raw = toml::to_string_pretty(&s).map_err(|e| SessionError::Message(e.to_string()))?;
    println!("# {}", crate::paths::config_file().display());
    print!("{raw}");
    Ok(())
}

pub fn cmd_config_set(key: &str, value: &str) -> Result<(), SessionError> {
    let s = config::set_key(key, value)?;
    println!("updated {key}");
    println!(
        "hotkey={} copy={} paste={}",
        s.hotkey, s.copy_to_clipboard, s.auto_paste
    );
    Ok(())
}

pub fn cmd_config_get(key: &str) -> Result<(), SessionError> {
    let s = config::load()?;
    let v = match key {
        "model" => s.model.unwrap_or_default(),
        "copy_to_clipboard" => s.copy_to_clipboard.to_string(),
        "auto_paste" => s.auto_paste.to_string(),
        "hotkey" => s.hotkey,
        "double_tap_lock" => s.double_tap_lock.to_string(),
        "minimum_key_time" => s.minimum_key_time.to_string(),
        "whisper_threads" => s.whisper_threads.to_string(),
        "max_history_entries" => s.max_history_entries.to_string(),
        "sound_effects" => s.sound_effects.to_string(),
        other => {
            return Err(SessionError::Message(format!("unknown key: {other}")));
        }
    };
    println!("{v}");
    Ok(())
}

pub fn cmd_shortcut_show() -> Result<(), SessionError> {
    let s = config::load()?;
    let hk = HotKey::parse(&s.hotkey).map_err(SessionError::Message)?;
    println!("{}", hk.display());
    Ok(())
}

pub fn cmd_shortcut_set(chord: &str) -> Result<(), SessionError> {
    let hk = HotKey::parse(chord).map_err(SessionError::Message)?;
    config::set_key("hotkey", &hk.display())?;
    println!("hotkey set to {}", hk.display());
    Ok(())
}

/// Background recorder worker.
pub fn run_worker() -> Result<(), SessionError> {
    ensure_dirs()?;
    let _ = clear_stop_flag();
    clear_cancel_flag();

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

    let cancelled = cancel_requested();
    let _ = fs::remove_file(stop_flag_path());
    clear_cancel_flag();

    match result {
        Ok(path) => {
            if cancelled {
                let _ = fs::remove_file(&path);
                let state = SessionState {
                    status: SessionStatus::Idle,
                    worker_pid: None,
                    started_at: None,
                    last_recording_path: previous_path,
                    last_transcript: previous_transcript,
                    last_error: Some("cancelled".into()),
                };
                save_state(&state)?;
                return Ok(());
            }
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
                last_error: Some(if cancelled {
                    "cancelled".into()
                } else {
                    e.to_string()
                }),
            };
            save_state(&state)?;
            if cancelled {
                Ok(())
            } else {
                Err(SessionError::Audio(e))
            }
        }
    }
}
