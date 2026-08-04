//! Unix-socket protocol between short-lived CLI commands and the Sage daemon.

use crate::paths::{daemon_socket_path, ensure_dirs};
use crate::store::JobSummary;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum DaemonCommand {
    Ping,
    Toggle,
    Start,
    Stop,
    Cancel,
    Status,
    Pending { limit: usize },
    History { limit: usize },
    ClearHistory,
    Transcribe { path: String },
    PasteNext,
    CancelJob { id: String },
    Retry { id: String },
    SetHotkey { chord: String },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonResponse {
    pub ok: bool,
    pub message: String,
    #[serde(default)]
    pub jobs: Vec<JobSummary>,
}

impl DaemonResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            jobs: Vec::new(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            jobs: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("daemon communication failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid daemon response: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to start Sage daemon: {0}")]
    Start(String),
}

pub fn request(command: &DaemonCommand, auto_start: bool) -> Result<DaemonResponse, IpcError> {
    match send(command) {
        Ok(response) => Ok(response),
        Err(error) if auto_start => {
            start_background()?;
            send(command).map_err(|_| error)
        }
        Err(error) => Err(error),
    }
}

fn send(command: &DaemonCommand) -> Result<DaemonResponse, IpcError> {
    let mut stream = UnixStream::connect(daemon_socket_path())?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let payload = serde_json::to_vec(command)?;
    stream.write_all(&payload)?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    Ok(serde_json::from_slice(&response)?)
}

fn start_background() -> Result<(), IpcError> {
    ensure_dirs()?;
    let executable = std::env::current_exe()?;
    let child = Command::new(executable)
        .arg("__daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| IpcError::Start(error.to_string()))?;
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if send(&DaemonCommand::Ping).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(IpcError::Start(format!(
                "daemon pid {} did not become ready",
                child.id()
            )));
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}
