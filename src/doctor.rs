//! Environment diagnostics (phase 7/10).

use crate::asr;
use crate::config;
use crate::hotkey::HotKey;
use crate::paths;
use crate::state::{load_state, process_is_alive, SessionStatus};
use std::process::Command;

pub fn run() -> Result<(), crate::session::SessionError> {
    paths::ensure_dirs().map_err(crate::session::SessionError::from)?;
    println!("sage doctor");
    println!("===========");

    check("binary", || {
        Ok(format!("{}", std::env::current_exe()?.display()))
    });

    check("config_dir", || Ok(paths::config_dir().display().to_string()));
    check("data_dir", || Ok(paths::data_dir().display().to_string()));

    let settings = config::load().unwrap_or_default();
    check("config", || {
        Ok(format!(
            "hotkey={} copy={} paste={}",
            settings.hotkey, settings.copy_to_clipboard, settings.auto_paste
        ))
    });

    match HotKey::parse(&settings.hotkey) {
        Ok(h) => check("hotkey_parse", || Ok(h.display())),
        Err(e) => println!("[FAIL] hotkey_parse: {e}"),
    }

    let model = asr::default_model_path();
    if model.exists() {
        let mb = std::fs::metadata(&model).map(|m| m.len() / 1_000_000).unwrap_or(0);
        println!("[ OK ] model: {} ({} MB)", model.display(), mb);
    } else {
        println!("[FAIL] model: missing at {} — run `sage model pull`", model.display());
    }

    // Mic
    match {
        use cpal::traits::HostTrait;
        cpal::default_host().default_input_device()
    } {
        Some(d) => {
            use cpal::traits::DeviceTrait;
            let name = d.name().unwrap_or_else(|_| "unknown".into());
            println!("[ OK ] microphone: {name}");
        }
        None => println!("[WARN] microphone: no default input device"),
    }

    // Clipboard
    match arboard::Clipboard::new() {
        Ok(mut cb) => match cb.get_text() {
            Ok(_) | Err(_) => println!("[ OK ] clipboard: accessible"),
        },
        Err(e) => println!("[WARN] clipboard: {e}"),
    }

    // Paste helpers
    for bin in ["wtype", "xdotool", "ydotool"] {
        if which(bin) {
            println!("[ OK ] paste helper: {bin}");
        }
    }
    if !["wtype", "xdotool", "ydotool"].iter().any(|b| which(b)) {
        println!("[WARN] paste helper: none found (auto_paste limited)");
    }

    // Session
    match load_state() {
        Ok(s) => {
            println!("[ OK ] session: {}", s.status);
            if s.status == SessionStatus::Recording {
                if let Some(pid) = s.worker_pid {
                    if process_is_alive(pid) {
                        println!("[ OK ] recorder worker pid {pid} alive");
                    } else {
                        println!("[WARN] recorder worker pid {pid} dead (stale state)");
                    }
                }
            }
        }
        Err(e) => println!("[FAIL] session: {e}"),
    }

    println!();
    println!("tips:");
    println!("  sage model pull");
    println!("  sage start / stop / last");
    println!("  sage config show");
    println!("  sage daemon   # long-running control loop");
    Ok(())
}

fn check(name: &str, f: impl FnOnce() -> std::io::Result<String>) {
    match f() {
        Ok(v) => println!("[ OK ] {name}: {v}"),
        Err(e) => println!("[FAIL] {name}: {e}"),
    }
}

fn which(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
