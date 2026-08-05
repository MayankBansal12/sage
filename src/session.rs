//! Short-lived CLI commands backed by the single-owner Sage daemon.

use crate::asr::{self, AsrError};
use crate::audio;
use crate::config::{self, ConfigError};
use crate::hotkey::HotKey;
use crate::ipc::{self, DaemonCommand, DaemonResponse};
use crate::models::{self, ModelError};
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
    Model(#[from] ModelError),
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

pub fn cmd_model_list() -> Result<(), SessionError> {
    let settings = config::load()?;
    println!("    MODEL                          FAMILY       SIZE       LANGUAGES       SPEED  ACCURACY");
    for model in models::CATALOG {
        let file = models::file_for(model, None)?;
        let active = settings.model.as_deref() == Some(model.id)
            && settings
                .model_quant
                .as_deref()
                .unwrap_or(models::DEFAULT_QUANT)
                == file.quant;
        let marker = match (active, models::is_downloaded(file)) {
            (true, true) => "*+",
            (true, false) => "*!",
            (false, true) => " +",
            (false, false) => "  ",
        };
        let languages = if model.languages.len() == 1 {
            model.languages[0].to_uppercase()
        } else {
            format!("{} languages", model.languages.len())
        };
        println!(
            "{marker}  {:<30} {:<12} {:>7.1} MiB  {:<14} {:>3}/100 {:>3}/100{}",
            model.id,
            model.family,
            file.size_bytes as f64 / (1024.0 * 1024.0),
            languages,
            model.speed_score,
            model.accuracy_score,
            if model.recommended {
                "  recommended"
            } else {
                ""
            }
        );
    }
    println!();
    println!("* active  + downloaded  ! missing");
    println!("Downloads are checksum-verified before activation.");
    println!("Select a model with: sage model use <id>");
    Ok(())
}

pub fn cmd_model_info(id: &str) -> Result<(), SessionError> {
    let model = models::find(id)?;
    let settings = config::load()?;
    println!("{}", model.name);
    println!("id:           {}", model.id);
    println!("description:  {}", model.description);
    println!("family:       {}", model.family);
    println!("languages:    {}", model.languages.join(", "));
    println!(
        "language ID:  {}",
        yes_no(model.supports_language_detection)
    );
    println!("translation:  {}", yes_no(model.supports_translation));
    println!(
        "streaming:    {} (batch mode in Sage)",
        yes_no(model.supports_streaming)
    );
    println!("speed:        {}/100", model.speed_score);
    println!("accuracy:     {}/100", model.accuracy_score);
    println!("license:      {}", model.license);
    println!("base model:   https://huggingface.co/{}", model.base_model);
    println!("source:       https://huggingface.co/{}", model.repository);
    println!("quantizations:");
    for file in model.files {
        let active = settings.model.as_deref() == Some(model.id)
            && settings
                .model_quant
                .as_deref()
                .unwrap_or(models::DEFAULT_QUANT)
                == file.quant;
        println!(
            "  {:<8} {:>7.1} MiB  {:<9}{}  {}",
            file.quant,
            file.size_bytes as f64 / (1024.0 * 1024.0),
            if models::is_downloaded(file) {
                "downloaded"
            } else {
                "missing"
            },
            if active { " active" } else { "" },
            models::path_for(file).display()
        );
    }
    Ok(())
}

pub fn cmd_model_pull(id: &str, quant: Option<&str>) -> Result<(), SessionError> {
    let model = models::find(id)?;
    let file = models::file_for(model, quant)?;
    let path = models::pull(model, file)?;
    println!("model: {}", path.display());
    Ok(())
}

pub fn cmd_model_use(
    id: &str,
    quant: Option<&str>,
    language: Option<&str>,
) -> Result<(), SessionError> {
    let model = models::find(id)?;
    let file = models::file_for(model, quant)?;
    let mut settings = config::load()?;
    let language_intent = language
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| settings.language.clone());
    let effective =
        models::effective_language(model, &language_intent).map_err(SessionError::Message)?;
    let already_selected = settings.model.as_deref() == Some(model.id)
        && settings
            .model_quant
            .as_deref()
            .unwrap_or(models::DEFAULT_QUANT)
            == file.quant
        && models::is_verified(file);
    if already_selected {
        if settings.language != language_intent {
            settings.language = language_intent.clone();
            config::save(&settings)?;
        }
        println!("already selected {} ({})", model.name, file.quant);
        println!(
            "language: {}",
            effective.as_deref().unwrap_or("automatic detection")
        );
        return Ok(());
    }
    let path = models::pull(model, file)?;
    asr::validate_model(&path, model, settings.stt_threads)?;

    settings.model = Some(model.id.to_string());
    settings.model_quant = Some(file.quant.to_string());
    settings.language = language_intent.clone();
    config::save(&settings)?;

    println!("selected {} ({})", model.name, file.quant);
    println!("model: {}", path.display());
    println!(
        "language: {}",
        effective.as_deref().unwrap_or("automatic detection")
    );
    if language_intent == "auto" && effective.is_some() {
        println!("note: this model cannot detect language; using the value above");
    }
    Ok(())
}

pub fn cmd_model_current() -> Result<(), SessionError> {
    let selection = asr::selected()?;
    println!("{}", selection.model.name);
    println!("id:        {}", selection.model.id);
    println!("quant:     {}", selection.file.quant);
    println!("path:      {}", selection.path.display());
    println!(
        "status:    {}",
        match models::installation_status(selection.file) {
            models::InstallationStatus::Verified => "verified",
            models::InstallationStatus::Missing => "missing",
            models::InstallationStatus::Corrupt => "corrupt",
        }
    );
    println!(
        "language:  {}",
        selection
            .language
            .as_deref()
            .unwrap_or("automatic detection")
    );
    Ok(())
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
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
        "model_quant" => settings.model_quant.unwrap_or_default(),
        "language" => settings.language,
        "copy_to_clipboard" => settings.copy_to_clipboard.to_string(),
        "auto_paste" => settings.auto_paste.to_string(),
        "hotkey" => settings.hotkey,
        "minimum_key_time" => settings.minimum_key_time.to_string(),
        "stt_threads" => settings.stt_threads.to_string(),
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
