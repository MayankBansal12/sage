//! Long-running daemon (phase 5): stdin command loop + optional hotkey simulation via control file.
//!
//! Global OS hotkeys are environment-specific; this daemon always accepts:
//! - lines on stdin: start | stop | cancel | quit | status
//! - or `sage start/stop` against the same session state (already works)
//!
//! When `SAGE_HOTKEY_SIM=1`, also watches `run/hotkey_event` for test events:
//!   press / release / escape

use crate::config;
use crate::feedback::{self, Cue};
use crate::hotkey::{HotKey, HotKeyOutput, HotKeyProcessor, KeyEvent};
use crate::paths::{daemon_pid_path, ensure_dirs, run_dir};
use crate::session;
use crate::state::{load_state, process_is_alive, reconcile_stale_state};
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn cmd_daemon() -> Result<(), session::SessionError> {
    ensure_dirs().map_err(session::SessionError::from)?;
    let pid = std::process::id();
    fs::write(daemon_pid_path(), format!("{pid}\n")).map_err(session::SessionError::from)?;

    let settings = config::load().unwrap_or_default();
    let hotkey = HotKey::parse(&settings.hotkey).unwrap_or_else(|_| {
        HotKey::parse("ctrl+shift+space").expect("default hotkey")
    });
    let mut processor = HotKeyProcessor::new(hotkey.clone());
    processor.double_tap_lock = settings.double_tap_lock;
    processor.minimum_key_time =
        Duration::from_secs_f64(settings.minimum_key_time.max(0.0));

    println!("sage daemon running (pid {pid})");
    println!("  hotkey config: {}", hotkey.display());
    println!("  commands on stdin: start | stop | cancel | status | quit");
    println!("  (global OS grab not required; use CLI or stdin)");
    if std::env::var("SAGE_HOTKEY_SIM").ok().as_deref() == Some("1") {
        println!(
            "  hotkey sim: write press|release|escape to {}",
            hotkey_sim_path().display()
        );
    }

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let _ = ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    });

    // stdin reader thread
    let running2 = running.clone();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        let mut buf = String::new();
        while running2.load(Ordering::SeqCst) {
            buf.clear();
            match lock.read_line(&mut buf) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let _ = tx.send(buf.trim().to_string());
                }
                Err(_) => break,
            }
        }
    });

    while running.load(Ordering::SeqCst) {
        if std::env::var("SAGE_HOTKEY_SIM").ok().as_deref() == Some("1") {
            if let Some(ev) = drain_hotkey_sim(&hotkey) {
                if let Some(o) = processor.process(&ev, Instant::now()) {
                    handle_output(o, &settings)?;
                }
            }
        }

        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(cmd) => {
                if cmd.is_empty() {
                    continue;
                }
                match cmd.to_ascii_lowercase().as_str() {
                    "quit" | "exit" => break,
                    "start" => {
                        feedback::play(Cue::Start, &settings);
                        if let Err(e) = session::cmd_start() {
                            eprintln!("error: {e}");
                            feedback::play(Cue::Error, &settings);
                        }
                    }
                    "stop" => {
                        if let Err(e) = session::cmd_stop() {
                            eprintln!("error: {e}");
                            feedback::play(Cue::Error, &settings);
                        } else {
                            feedback::play(Cue::Stop, &settings);
                        }
                    }
                    "cancel" => {
                        if let Err(e) = session::cmd_cancel() {
                            eprintln!("error: {e}");
                        }
                        feedback::play(Cue::Cancel, &settings);
                    }
                    "status" => {
                        let _ = session::cmd_status();
                    }
                    other => eprintln!("unknown command: {other}"),
                }
                let _ = std::io::stdout().flush();
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // stdin closed — keep running for sim / external control
                std::thread::sleep(Duration::from_millis(100));
                if std::env::var("SAGE_HOTKEY_SIM").ok().as_deref() != Some("1") {
                    // without sim and without stdin, still useful as keepalive? exit
                    // Keep alive so `sage start` from another process works with shared state.
                    // User can kill via SIGINT.
                }
            }
        }
    }

    let _ = fs::remove_file(daemon_pid_path());
    println!("sage daemon stopped");
    Ok(())
}

fn handle_output(
    out: HotKeyOutput,
    settings: &config::Settings,
) -> Result<(), session::SessionError> {
    match out {
        HotKeyOutput::StartRecording => {
            feedback::play(Cue::Start, settings);
            session::cmd_start()
        }
        HotKeyOutput::StopRecording => {
            session::cmd_stop()?;
            feedback::play(Cue::Stop, settings);
            Ok(())
        }
        HotKeyOutput::Cancel => {
            session::cmd_cancel()?;
            feedback::play(Cue::Cancel, settings);
            Ok(())
        }
    }
}

fn hotkey_sim_path() -> PathBuf {
    run_dir().join("hotkey_event")
}

fn drain_hotkey_sim(hotkey: &HotKey) -> Option<KeyEvent> {
    let path = hotkey_sim_path();
    let raw = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(&path);
    let line = raw.lines().next()?.trim().to_ascii_lowercase();
    match line.as_str() {
        "press" => Some(KeyEvent {
            key: hotkey.key.clone(),
            modifiers: hotkey.modifiers.clone(),
            pressed: true,
        }),
        "release" => Some(KeyEvent {
            key: hotkey.key.clone(),
            modifiers: hotkey.modifiers.clone(),
            pressed: false,
        }),
        "escape" => Some(KeyEvent {
            key: Some("escape".into()),
            modifiers: BTreeSet::new(),
            pressed: true,
        }),
        _ => None,
    }
}

pub fn daemon_status() -> Result<(), session::SessionError> {
    ensure_dirs().map_err(session::SessionError::from)?;
    let path = daemon_pid_path();
    if !path.exists() {
        println!("daemon: not running");
        return Ok(());
    }
    let raw = fs::read_to_string(&path).map_err(session::SessionError::from)?;
    let pid: u32 = raw.trim().parse().unwrap_or(0);
    if pid > 0 && process_is_alive(pid) {
        println!("daemon: running (pid {pid})");
    } else {
        println!("daemon: stale pid file");
        let _ = fs::remove_file(path);
    }
    let st = reconcile_stale_state(load_state().map_err(session::SessionError::from)?)
        .map_err(session::SessionError::from)?;
    println!("session: {}", st.status);
    Ok(())
}
