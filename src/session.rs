//! Short-lived CLI commands backed by the single-owner Sage daemon.

use crate::asr::{self, AsrError};
use crate::audio;
use crate::config::{self, ConfigError};
use crate::hotkey::HotKey;
use crate::ipc::{self, DaemonCommand, DaemonResponse};
use crate::output;
use chrono::{Local, TimeZone};
use std::path::Path;
use std::thread;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Audio(#[from] audio::AudioError),
    #[error(transparent)]
    Asr(#[from] AsrError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn cmd_toggle() -> Result<(), SessionError> {
    print_response(call(DaemonCommand::Toggle, true)?)
}

pub fn cmd_start() -> Result<(), SessionError> {
    print_response(call(DaemonCommand::Start, true)?)
}

pub fn cmd_stop() -> Result<(), SessionError> {
    print_response(call(DaemonCommand::Stop, true)?)
}

pub fn cmd_cancel() -> Result<(), SessionError> {
    print_response(call(DaemonCommand::Cancel, true)?)
}

pub fn cmd_status() -> Result<(), SessionError> {
    print_response(call(DaemonCommand::Status, true)?)
}

pub fn cmd_pending(limit: usize) -> Result<(), SessionError> {
    let response = call(DaemonCommand::Pending { limit }, true)?;
    ensure_ok(&response)?;
    if response.jobs.is_empty() {
        println!("(no pending transcripts)");
        return Ok(());
    }
    for job in response.jobs {
        let target = job.target.unwrap_or_else(|| "clipboard fallback".into());
        let preview = job
            .transcript
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(70)
            .collect::<String>();
        println!(
            "{} [{}] {} — {}{}",
            job.id,
            job.status,
            target,
            preview,
            job.error
                .map(|error| format!(" ({error})"))
                .unwrap_or_default()
        );
    }
    Ok(())
}

pub fn cmd_paste_next() -> Result<(), SessionError> {
    print_response(call(DaemonCommand::PasteNext, true)?)
}

pub fn cmd_cancel_job(id: &str) -> Result<(), SessionError> {
    print_response(call(DaemonCommand::CancelJob { id: id.into() }, true)?)
}

pub fn cmd_retry(id: &str) -> Result<(), SessionError> {
    print_response(call(DaemonCommand::Retry { id: id.into() }, true)?)
}

pub fn cmd_daemon_stop() -> Result<(), SessionError> {
    print_response(call(DaemonCommand::Shutdown, false)?)
}

pub fn cmd_last() -> Result<(), SessionError> {
    let response = call(DaemonCommand::History { limit: 1 }, true)?;
    ensure_ok(&response)?;
    let transcript = response
        .jobs
        .first()
        .and_then(|job| job.transcript.as_deref())
        .ok_or_else(|| SessionError::Message("no transcript yet".into()))?;
    println!("{transcript}");
    Ok(())
}

pub fn cmd_copy_last() -> Result<(), SessionError> {
    let response = call(DaemonCommand::History { limit: 1 }, true)?;
    ensure_ok(&response)?;
    let transcript = response
        .jobs
        .first()
        .and_then(|job| job.transcript.as_deref())
        .ok_or_else(|| SessionError::Message("no transcript to copy".into()))?;
    output::copy_to_clipboard(transcript)
        .map_err(|error| SessionError::Message(error.to_string()))?;
    println!("copied to clipboard ({} chars)", transcript.chars().count());
    Ok(())
}

pub fn cmd_transcribe(path: &Path) -> Result<(), SessionError> {
    let path = path
        .canonicalize()
        .map_err(|_| SessionError::Message(format!("file not found: {}", path.display())))?;
    print_response(call(
        DaemonCommand::Transcribe {
            path: path.display().to_string(),
        },
        true,
    )?)
}

pub fn cmd_model_pull() -> Result<(), SessionError> {
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
    let response = call(DaemonCommand::History { limit }, true)?;
    ensure_ok(&response)?;
    if response.jobs.is_empty() {
        println!("(no history)");
        return Ok(());
    }
    for (index, job) in response.jobs.iter().enumerate() {
        let timestamp = Local
            .timestamp_millis_opt(job.created_at_ms)
            .single()
            .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "unknown".into());
        let preview = job
            .transcript
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(80)
            .collect::<String>();
        println!("{}. [{}] {preview}", index + 1, timestamp);
    }
    Ok(())
}

pub fn cmd_history_clear() -> Result<(), SessionError> {
    print_response(call(DaemonCommand::ClearHistory, true)?)
}

pub fn cmd_config_show() -> Result<(), SessionError> {
    let settings = config::load()?;
    let raw = toml::to_string_pretty(&settings)
        .map_err(|error| SessionError::Message(error.to_string()))?;
    println!("# {}", crate::paths::config_file().display());
    print!("{raw}");
    Ok(())
}

pub fn cmd_config_set(key: &str, value: &str) -> Result<(), SessionError> {
    if key == "hotkey" {
        let normalized = HotKey::parse(value)
            .map_err(SessionError::Message)?
            .display();
        let settings = match ipc::request(
            &DaemonCommand::SetHotkey {
                chord: normalized.clone(),
            },
            false,
        ) {
            Ok(response) => {
                ensure_ok(&response)?;
                config::load()?
            }
            Err(_) => config::set_key(key, &normalized)?,
        };
        thread::sleep(Duration::from_millis(150));
        ipc::request(&DaemonCommand::Ping, true)
            .map_err(|error| SessionError::Message(error.to_string()))?;
        println!("updated {key}");
        println!("global shortcut registration restarted");
        println!(
            "hotkey={} pending={}h separator={:?}",
            settings.hotkey, settings.pending_delivery_hours, settings.pending_separator
        );
        return Ok(());
    }
    let settings = config::set_key(key, value)?;
    println!("updated {key}");
    println!(
        "hotkey={} pending={}h separator={:?}",
        settings.hotkey, settings.pending_delivery_hours, settings.pending_separator
    );
    Ok(())
}

pub fn cmd_config_get(key: &str) -> Result<(), SessionError> {
    let settings = config::load()?;
    let value = match key {
        "model" => settings.model.unwrap_or_default(),
        "copy_to_clipboard" => settings.copy_to_clipboard.to_string(),
        "auto_paste" => settings.auto_paste.to_string(),
        "hotkey" => settings.hotkey,
        "minimum_key_time" => settings.minimum_key_time.to_string(),
        "whisper_threads" => settings.whisper_threads.to_string(),
        "max_history_entries" => settings.max_history_entries.to_string(),
        "sound_effects" => settings.sound_effects.to_string(),
        "pending_delivery_hours" => settings.pending_delivery_hours.to_string(),
        "pending_separator" => settings.pending_separator,
        other => return Err(SessionError::Message(format!("unknown key: {other}"))),
    };
    println!("{value}");
    Ok(())
}

pub fn cmd_shortcut_show() -> Result<(), SessionError> {
    let settings = config::load()?;
    let hotkey = HotKey::parse(&settings.hotkey).map_err(SessionError::Message)?;
    println!("{}", hotkey.display());
    Ok(())
}

pub fn cmd_shortcut_set(chord: &str) -> Result<(), SessionError> {
    let hotkey = HotKey::parse(chord).map_err(SessionError::Message)?;
    cmd_config_set("hotkey", &hotkey.display())
}

fn call(command: DaemonCommand, auto_start: bool) -> Result<DaemonResponse, SessionError> {
    ipc::request(&command, auto_start).map_err(|error| SessionError::Message(error.to_string()))
}

fn ensure_ok(response: &DaemonResponse) -> Result<(), SessionError> {
    if response.ok {
        Ok(())
    } else {
        Err(SessionError::Message(response.message.clone()))
    }
}

fn print_response(response: DaemonResponse) -> Result<(), SessionError> {
    ensure_ok(&response)?;
    println!("{}", response.message);
    Ok(())
}
