//! Single-owner Sage daemon: recording, queueing, transcription, and delivery.

use crate::asr;
use crate::audio::{self, RecordingInfo};
use crate::config;
use crate::feedback::{self, Cue};
use crate::global_hotkey::{self, GlobalHotkeyEvent};
use crate::ipc::{DaemonCommand, DaemonResponse};
use crate::output;
use crate::paths::{daemon_pid_path, daemon_socket_path, ensure_dirs, recordings_dir};
use crate::store::{Job, Store};
use crate::target::{self, TargetEvent, TargetHandle, TargetToken};
use fs2::FileExt;
use std::fs;
use std::io::{Read, Seek, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

enum Event {
    Request(DaemonCommand, mpsc::Sender<DaemonResponse>),
    RecordingReady(String),
    RecordingFinished {
        id: String,
        result: Result<RecordingInfo, String>,
        cancelled: bool,
    },
    TranscriptionStarted {
        id: String,
        response: mpsc::Sender<bool>,
    },
    TranscriptionFinished {
        id: String,
        result: Result<String, String>,
    },
    Target(TargetEvent),
    DeliverIfFocused(TargetToken),
    Hotkey(GlobalHotkeyEvent),
    SignalShutdown,
}

struct ActiveRecording {
    id: String,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone)]
struct TranscriptionTask {
    id: String,
    audio_path: PathBuf,
}

pub fn cmd_daemon() -> Result<(), crate::session::SessionError> {
    run(false)
}

pub fn run_background() -> Result<(), crate::session::SessionError> {
    run(true)
}

fn run(quiet: bool) -> Result<(), crate::session::SessionError> {
    ensure_dirs()?;
    let pid_path = daemon_pid_path();
    let mut pid_lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&pid_path)?;
    pid_lock.try_lock_exclusive().map_err(|_| {
        crate::session::SessionError::Message("Sage daemon is already running or starting".into())
    })?;
    pid_lock.set_len(0)?;
    pid_lock.rewind()?;
    writeln!(pid_lock, "{}", std::process::id())?;
    fs::set_permissions(&pid_path, fs::Permissions::from_mode(0o600))?;

    let socket_path = daemon_socket_path();
    if socket_path.exists() {
        if UnixStream::connect(&socket_path).is_ok() {
            return Err(crate::session::SessionError::Message(
                "Sage daemon is already running".into(),
            ));
        }
        fs::remove_file(&socket_path)?;
    }
    let listener = UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;

    let (event_tx, event_rx) = mpsc::channel::<Event>();
    spawn_ipc_listener(listener, event_tx.clone());

    let target_handle = spawn_target_bridge(event_tx.clone());
    let settings = config::load()?;
    spawn_hotkey_bridge(settings.hotkey.clone(), event_tx.clone());

    let (stt_tx, stt_rx) = mpsc::channel::<TranscriptionTask>();
    spawn_transcriber(stt_rx, event_tx.clone());

    let mut store = Store::open().map_err(message)?;
    store.recover().map_err(message)?;
    for job in store.queued().map_err(message)? {
        let _ = stt_tx.send(TranscriptionTask {
            id: job.id,
            audio_path: PathBuf::from(job.audio_path),
        });
    }

    let signal_tx = event_tx.clone();
    let _ = ctrlc::set_handler(move || {
        let _ = signal_tx.send(Event::SignalShutdown);
    });

    if !quiet {
        println!("sage daemon running (pid {})", std::process::id());
        println!("socket: {}", socket_path.display());
    }

    let mut active: Option<ActiveRecording> = None;
    let mut hotkey_status = "initializing".to_string();
    let mut target_status = "initializing".to_string();
    let mut last_expiry_check = Instant::now();
    let mut running = true;

    while running {
        match event_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Request(command, response)) => {
                let (reply, should_stop) = handle_command(
                    command,
                    &mut active,
                    &mut store,
                    &stt_tx,
                    &event_tx,
                    &target_handle,
                    &hotkey_status,
                    &target_status,
                );
                let _ = response.send(reply);
                running = !should_stop;
            }
            Ok(Event::RecordingReady(id)) => {
                if active.as_ref().is_some_and(|recording| recording.id == id) {
                    if let Ok(settings) = config::load() {
                        feedback::play(Cue::Start, &settings);
                    }
                }
            }
            Ok(Event::RecordingFinished {
                id,
                result,
                cancelled,
            }) => {
                if active.as_ref().is_some_and(|recording| recording.id == id) {
                    active = None;
                }
                finish_recording(&id, result, cancelled, &mut store, &stt_tx);
            }
            Ok(Event::TranscriptionStarted { id, response }) => {
                let accepted = store.mark_transcribing(&id).unwrap_or(false);
                let _ = response.send(accepted);
            }
            Ok(Event::TranscriptionFinished { id, result }) => {
                finish_transcription(&id, result, &mut store, &target_handle);
            }
            Ok(Event::Target(TargetEvent::FocusChanged(target))) => {
                let tx = event_tx.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(150));
                    let _ = tx.send(Event::DeliverIfFocused(target));
                });
            }
            Ok(Event::Target(TargetEvent::Available)) => {
                target_status = "available".into();
            }
            Ok(Event::Target(TargetEvent::Unavailable(error))) => {
                target_status = format!("unavailable ({error})");
            }
            Ok(Event::DeliverIfFocused(target)) => {
                if target_handle
                    .current()
                    .is_some_and(|current| current.same_control(&target))
                {
                    let _ = deliver_target(&target, &mut store, &target_handle);
                }
            }
            Ok(Event::Hotkey(GlobalHotkeyEvent::Activated)) => {
                let _ = toggle_recording(&mut active, &mut store, &target_handle, &event_tx);
            }
            Ok(Event::Hotkey(GlobalHotkeyEvent::Available(description))) => {
                hotkey_status = format!("portal ({description})");
            }
            Ok(Event::Hotkey(GlobalHotkeyEvent::Unavailable(error))) => {
                hotkey_status = format!("fallback required ({error})");
            }
            Ok(Event::SignalShutdown) => running = false,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if last_expiry_check.elapsed() >= Duration::from_secs(30) {
            let _ = store.expire_ready();
            if let Ok(settings) = config::load() {
                prune_recordings(&mut store, settings.max_history_entries);
            }
            last_expiry_check = Instant::now();
        }
    }

    if let Some(recording) = active.take() {
        recording.cancel.store(true, Ordering::SeqCst);
        recording.stop.store(true, Ordering::SeqCst);
        let _ = store.mark_cancelled(&recording.id);
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            match event_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Event::RecordingFinished { id, result, .. }) if id == recording.id => {
                    if let Ok(info) = result {
                        let _ = fs::remove_file(info.path);
                    } else if let Ok(Some(job)) = store.get(&id) {
                        let _ = fs::remove_file(job.audio_path);
                    }
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                _ => {}
            }
        }
    }
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(&pid_path);
    if !quiet {
        println!("sage daemon stopped");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_command(
    command: DaemonCommand,
    active: &mut Option<ActiveRecording>,
    store: &mut Store,
    stt_tx: &mpsc::Sender<TranscriptionTask>,
    event_tx: &mpsc::Sender<Event>,
    target_handle: &TargetHandle,
    hotkey_status: &str,
    target_status: &str,
) -> (DaemonResponse, bool) {
    let response = match command {
        DaemonCommand::Ping => DaemonResponse::ok("pong"),
        DaemonCommand::Toggle => match toggle_recording(active, store, target_handle, event_tx) {
            Ok(message) => DaemonResponse::ok(message),
            Err(error) => DaemonResponse::error(error),
        },
        DaemonCommand::Start => match start_recording(active, store, target_handle, event_tx) {
            Ok(message) => DaemonResponse::ok(message),
            Err(error) => DaemonResponse::error(error),
        },
        DaemonCommand::Stop => match stop_recording(active, false) {
            Ok(message) => DaemonResponse::ok(message),
            Err(error) => DaemonResponse::error(error),
        },
        DaemonCommand::Cancel => match stop_recording(active, true) {
            Ok(message) => DaemonResponse::ok(message),
            Err(error) => DaemonResponse::error(error),
        },
        DaemonCommand::Status => match store.counts() {
            Ok(counts) => {
                let recording = active
                    .as_ref()
                    .map(|value| value.id.as_str())
                    .unwrap_or("none");
                let status = if active.is_some() {
                    "recording"
                } else {
                    "idle"
                };
                let counts = counts
                    .into_iter()
                    .map(|(status, count)| format!("{status}={count}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                DaemonResponse::ok(format!(
                    "status: {status}\nrecording: {recording}\nqueue: {counts}\nhotkey: {hotkey_status}\ntargeting: {target_status}"
                ))
            }
            Err(error) => DaemonResponse::error(error.to_string()),
        },
        DaemonCommand::Pending { limit } => match store.summaries(true, limit) {
            Ok(jobs) => DaemonResponse {
                ok: true,
                message: format!("{} pending job(s)", jobs.len()),
                jobs,
            },
            Err(error) => DaemonResponse::error(error.to_string()),
        },
        DaemonCommand::History { limit } => match store.summaries(false, limit) {
            Ok(jobs) => DaemonResponse {
                ok: true,
                message: format!("{} transcript(s)", jobs.len()),
                jobs,
            },
            Err(error) => DaemonResponse::error(error.to_string()),
        },
        DaemonCommand::ClearHistory => match store.clear_history() {
            Ok(paths) => {
                let count = paths.len();
                for path in paths {
                    let _ = fs::remove_file(path);
                }
                DaemonResponse::ok(format!("cleared history and {count} recording(s)"))
            }
            Err(error) => DaemonResponse::error(error.to_string()),
        },
        DaemonCommand::Transcribe { path } => {
            let path = PathBuf::from(path);
            if !path.exists() {
                DaemonResponse::error(format!("file not found: {}", path.display()))
            } else {
                let id = uuid::Uuid::new_v4().to_string();
                let path_text = path.display().to_string();
                match store
                    .create_recording(&id, &path_text, None, false)
                    .and_then(|_| store.mark_queued(&id, 0))
                {
                    Ok(()) => {
                        let _ = stt_tx.send(TranscriptionTask {
                            id: id.clone(),
                            audio_path: path,
                        });
                        DaemonResponse::ok(format!("queued transcription job {id}"))
                    }
                    Err(error) => DaemonResponse::error(error.to_string()),
                }
            }
        }
        DaemonCommand::PasteNext => match paste_next(store, target_handle) {
            Ok(message) => DaemonResponse::ok(message),
            Err(error) => DaemonResponse::error(error),
        },
        DaemonCommand::CancelJob { id } => match store.cancel_pending(&id) {
            Ok(true) => DaemonResponse::ok(format!("cancelled job {id}")),
            Ok(false) => DaemonResponse::error(format!("job {id} cannot be cancelled")),
            Err(error) => DaemonResponse::error(error.to_string()),
        },
        DaemonCommand::Retry { id } => match store.retry(&id) {
            Ok(true) => {
                if let Ok(Some(job)) = store.get(&id) {
                    let _ = stt_tx.send(TranscriptionTask {
                        id: job.id,
                        audio_path: PathBuf::from(job.audio_path),
                    });
                }
                DaemonResponse::ok(format!("queued job {id} for retry"))
            }
            Ok(false) => DaemonResponse::error(format!("job {id} cannot be retried")),
            Err(error) => DaemonResponse::error(error.to_string()),
        },
        DaemonCommand::SetHotkey { chord } => {
            if active.is_some() {
                DaemonResponse::error(
                    "cannot change the hotkey while recording; stop or cancel first",
                )
            } else {
                match config::set_key("hotkey", &chord) {
                    Ok(_) => {
                        return (
                            DaemonResponse::ok("hotkey updated; daemon restarting"),
                            true,
                        );
                    }
                    Err(error) => DaemonResponse::error(error.to_string()),
                }
            }
        }
        DaemonCommand::Shutdown => return (DaemonResponse::ok("daemon stopping"), true),
    };
    (response, false)
}

fn toggle_recording(
    active: &mut Option<ActiveRecording>,
    store: &mut Store,
    target_handle: &TargetHandle,
    event_tx: &mpsc::Sender<Event>,
) -> Result<String, String> {
    if active.is_some() {
        stop_recording(active, false)
    } else {
        start_recording(active, store, target_handle, event_tx)
    }
}

fn start_recording(
    active: &mut Option<ActiveRecording>,
    store: &mut Store,
    target_handle: &TargetHandle,
    event_tx: &mpsc::Sender<Event>,
) -> Result<String, String> {
    if active.is_some() {
        return Err("already recording".into());
    }
    let settings = config::load().map_err(|error| error.to_string())?;
    let target = target_handle
        .capture()
        .or_else(|| target_handle.current())
        .filter(|target| target.editable);
    let id = uuid::Uuid::new_v4().to_string();
    let path = recordings_dir().join(format!("recording-{id}.wav"));
    store
        .create_recording(&id, &path.display().to_string(), target.as_ref(), true)
        .map_err(|error| error.to_string())?;

    let stop = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker_cancel = cancel.clone();
    let worker_id = id.clone();
    let sender = event_tx.clone();
    std::thread::spawn(move || {
        let ready_id = worker_id.clone();
        let ready_sender = sender.clone();
        let result = audio::record_until_signal(
            &path,
            Duration::from_secs(30 * 60),
            worker_stop,
            move || {
                let _ = ready_sender.send(Event::RecordingReady(ready_id));
                Ok(())
            },
        )
        .map_err(|error| error.to_string());
        let _ = sender.send(Event::RecordingFinished {
            id: worker_id,
            result,
            cancelled: worker_cancel.load(Ordering::SeqCst),
        });
    });

    *active = Some(ActiveRecording {
        id: id.clone(),
        stop,
        cancel,
    });
    let target_message = target
        .map(|target| format!("target: {}", target.label()))
        .unwrap_or_else(|| "target: clipboard fallback".into());
    if settings.sound_effects {
        // The actual start cue is emitted only after CPAL opens the microphone.
    }
    Ok(format!("recording started\njob: {id}\n{target_message}"))
}

fn stop_recording(active: &mut Option<ActiveRecording>, cancel: bool) -> Result<String, String> {
    let recording = active
        .as_ref()
        .ok_or_else(|| "not recording — nothing to stop".to_string())?;
    if cancel {
        recording.cancel.store(true, Ordering::SeqCst);
    }
    recording.stop.store(true, Ordering::SeqCst);
    Ok(if cancel {
        "recording cancellation requested".into()
    } else {
        "recording stopped; finalizing audio".into()
    })
}

fn finish_recording(
    id: &str,
    result: Result<RecordingInfo, String>,
    cancelled: bool,
    store: &mut Store,
    stt_tx: &mpsc::Sender<TranscriptionTask>,
) {
    let settings = match config::load() {
        Ok(settings) => settings,
        Err(error) => {
            let _ = store.mark_failed(id, &format!("invalid configuration: {error}"));
            return;
        }
    };
    match result {
        Ok(info) if cancelled => {
            let _ = fs::remove_file(&info.path);
            let _ = store.mark_cancelled(id);
            feedback::play(Cue::Cancel, &settings);
        }
        Ok(info) if info.duration.as_secs_f64() < settings.minimum_key_time.max(0.0) => {
            let _ = fs::remove_file(&info.path);
            let _ = store.mark_cancelled(id);
            feedback::play(Cue::Cancel, &settings);
        }
        Ok(info) => {
            let duration_ms = i64::try_from(info.duration.as_millis()).unwrap_or(i64::MAX);
            if store.mark_queued(id, duration_ms).is_ok() {
                let _ = stt_tx.send(TranscriptionTask {
                    id: id.to_string(),
                    audio_path: info.path,
                });
                feedback::play(Cue::Stop, &settings);
            }
        }
        Err(error) => {
            if cancelled {
                if let Ok(Some(job)) = store.get(id) {
                    let _ = fs::remove_file(job.audio_path);
                }
                let _ = store.mark_cancelled(id);
                feedback::play(Cue::Cancel, &settings);
            } else {
                let _ = store.mark_failed(id, &error);
                feedback::play(Cue::Error, &settings);
            }
        }
    }
}

fn finish_transcription(
    id: &str,
    result: Result<String, String>,
    store: &mut Store,
    target_handle: &TargetHandle,
) {
    let settings = match config::load() {
        Ok(settings) => settings,
        Err(error) => {
            let _ = store.mark_failed(id, &format!("invalid configuration: {error}"));
            return;
        }
    };
    match result {
        Ok(text) if !text.trim().is_empty() => {
            if !matches!(
                store.mark_ready(id, text.trim(), settings.pending_delivery_hours),
                Ok(true)
            ) {
                return;
            }
            if let Ok(Some(job)) = store.get(id) {
                if let Some(target) = job.target {
                    if target_handle
                        .current()
                        .is_some_and(|current| current.same_control(&target))
                    {
                        let _ = deliver_target(&target, store, target_handle);
                    }
                } else if settings.copy_to_clipboard
                    && output::copy_to_clipboard(text.trim()).is_ok()
                    && store.mark_delivered(&[id.to_string()]).is_ok()
                {
                    prune_recordings(store, settings.max_history_entries);
                }
            }
        }
        Ok(_) => {
            let _ = store.mark_failed(id, "transcription produced no text");
        }
        Err(error) => {
            let _ = store.mark_failed(id, &error);
            feedback::play(Cue::Error, &settings);
        }
    }
}

fn deliver_target(
    target: &TargetToken,
    store: &mut Store,
    target_handle: &TargetHandle,
) -> Result<String, String> {
    let jobs = store
        .ready_for_target(target)
        .map_err(|error| error.to_string())?;
    if jobs.is_empty() {
        return Ok("nothing pending for target".into());
    }
    let settings = config::load().map_err(|error| error.to_string())?;
    if !settings.auto_paste {
        return Ok("auto-paste is disabled; transcript remains pending".into());
    }
    let text = join_transcripts(&jobs, &settings.pending_separator);
    target_handle.insert(target.clone(), text.clone())?;
    if settings.copy_to_clipboard {
        let _ = output::copy_to_clipboard(&text);
    }
    let ids = jobs.into_iter().map(|job| job.id).collect::<Vec<_>>();
    store
        .mark_delivered(&ids)
        .map_err(|error| error.to_string())?;
    prune_recordings(store, settings.max_history_entries);
    Ok(format!("delivered {} transcript(s)", ids.len()))
}

fn prune_recordings(store: &mut Store, maximum: usize) {
    if let Ok(paths) = store.prune_history(maximum) {
        for path in paths {
            let _ = fs::remove_file(path);
        }
    }
}

fn paste_next(store: &mut Store, target_handle: &TargetHandle) -> Result<String, String> {
    store.expire_ready().map_err(|error| error.to_string())?;
    let job = store
        .oldest_ready()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "no transcript is ready for delivery".to_string())?;
    let text = job.transcript.clone().unwrap_or_default();
    if let Some(target) = target_handle.current().filter(|target| target.editable) {
        target_handle.insert(target, text.clone())?;
    } else {
        output::copy_to_clipboard(&text).map_err(|error| error.to_string())?;
    }
    store
        .mark_delivered(&[job.id])
        .map_err(|error| error.to_string())?;
    Ok("delivered next pending transcript".into())
}

fn join_transcripts(jobs: &[Job], separator: &str) -> String {
    jobs.iter()
        .filter_map(|job| job.transcript.as_deref())
        .collect::<Vec<_>>()
        .join(separator)
}

fn spawn_transcriber(receiver: mpsc::Receiver<TranscriptionTask>, sender: mpsc::Sender<Event>) {
    std::thread::spawn(move || {
        let mut engine = asr::PersistentWhisper::new();
        loop {
            match receiver.recv_timeout(Duration::from_secs(10 * 60)) {
                Ok(task) => {
                    let (response_tx, response_rx) = mpsc::channel();
                    if sender
                        .send(Event::TranscriptionStarted {
                            id: task.id.clone(),
                            response: response_tx,
                        })
                        .is_err()
                        || !response_rx
                            .recv_timeout(Duration::from_secs(2))
                            .unwrap_or(false)
                    {
                        continue;
                    }
                    let result = engine
                        .transcribe(&task.audio_path)
                        .map_err(|error| error.to_string());
                    let _ = sender.send(Event::TranscriptionFinished {
                        id: task.id,
                        result,
                    });
                }
                Err(mpsc::RecvTimeoutError::Timeout) => engine.unload(),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

fn spawn_target_bridge(sender: mpsc::Sender<Event>) -> TargetHandle {
    let (tx, rx) = mpsc::channel();
    let handle = target::spawn(tx);
    std::thread::spawn(move || {
        for event in rx {
            let _ = sender.send(Event::Target(event));
        }
    });
    handle
}

fn spawn_hotkey_bridge(chord: String, sender: mpsc::Sender<Event>) {
    let (tx, rx) = mpsc::channel();
    global_hotkey::spawn(chord, tx);
    std::thread::spawn(move || {
        for event in rx {
            let _ = sender.send(Event::Hotkey(event));
        }
    });
}

fn spawn_ipc_listener(listener: UnixListener, sender: mpsc::Sender<Event>) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let sender = sender.clone();
            std::thread::spawn(move || serve_connection(stream, sender));
        }
    });
}

fn serve_connection(mut stream: UnixStream, sender: mpsc::Sender<Event>) {
    const MAX_REQUEST_BYTES: u64 = 64 * 1024;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut payload = Vec::new();
    let response = match (&mut stream)
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut payload)
    {
        Ok(_) if payload.len() as u64 > MAX_REQUEST_BYTES => {
            DaemonResponse::error("daemon request is too large")
        }
        Ok(_) => match serde_json::from_slice::<DaemonCommand>(&payload) {
            Ok(command) => {
                let (tx, rx) = mpsc::channel();
                if sender.send(Event::Request(command, tx)).is_err() {
                    DaemonResponse::error("daemon is stopping")
                } else {
                    rx.recv_timeout(Duration::from_secs(9))
                        .unwrap_or_else(|_| DaemonResponse::error("daemon request timed out"))
                }
            }
            Err(error) => DaemonResponse::error(format!("invalid request: {error}")),
        },
        Err(error) => DaemonResponse::error(error.to_string()),
    };
    if let Ok(encoded) = serde_json::to_vec(&response) {
        let _ = stream.write_all(&encoded);
    }
}

pub fn daemon_status() -> Result<(), crate::session::SessionError> {
    match crate::ipc::request(&DaemonCommand::Status, false) {
        Ok(response) if response.ok => {
            println!("daemon: running");
            println!("{}", response.message);
        }
        _ => println!("daemon: not running"),
    }
    Ok(())
}

fn message(error: impl std::fmt::Display) -> crate::session::SessionError {
    crate::session::SessionError::Message(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_transcripts_in_sequence_order() {
        let jobs = ["one", "two"]
            .into_iter()
            .enumerate()
            .map(|(index, text)| Job {
                id: index.to_string(),
                audio_path: String::new(),
                transcript: Some(text.into()),
                target: None,
            })
            .collect::<Vec<_>>();
        assert_eq!(join_transcripts(&jobs, " "), "one two");
        assert_eq!(join_transcripts(&jobs, "\n"), "one\ntwo");
    }
}
