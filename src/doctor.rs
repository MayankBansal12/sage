//! Environment diagnostics (phase 7/10).

use crate::asr;
use crate::config;
use crate::hotkey::HotKey;
use crate::models;
use crate::paths;

pub fn run() -> Result<(), crate::session::SessionError> {
    paths::ensure_dirs().map_err(crate::session::SessionError::from)?;
    println!("sage doctor");
    println!("===========");

    check("binary", || {
        Ok(format!("{}", std::env::current_exe()?.display()))
    });

    check("config_dir", || {
        Ok(paths::config_dir().display().to_string())
    });
    check("data_dir", || Ok(paths::data_dir().display().to_string()));

    let settings = config::load()?;
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

    match asr::selected() {
        Ok(selection) => match models::installation_status(selection.file) {
            models::InstallationStatus::Verified => println!(
                "[ OK ] model: {} ({}, {:.1} MiB, checksum verified)",
                selection.model.name,
                selection.file.quant,
                selection.file.size_bytes as f64 / (1024.0 * 1024.0)
            ),
            models::InstallationStatus::Missing => println!(
                "[FAIL] model: {} is missing — run `sage model use {}`",
                selection.model.name, selection.model.id
            ),
            models::InstallationStatus::Corrupt => println!(
                "[FAIL] model: {} failed integrity verification — run `sage model pull {}`",
                selection.model.name, selection.model.id
            ),
        },
        Err(_) => println!(
            "[FAIL] model: none selected — run `sage model list`, then `sage model use <id>`"
        ),
    }

    // Mic
    let microphone = {
        use cpal::traits::HostTrait;
        cpal::default_host().default_input_device()
    };
    match microphone {
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

    match crate::ipc::request(&crate::ipc::DaemonCommand::Status, false) {
        Ok(response) if response.ok => println!("[ OK ] daemon: running"),
        _ => println!("[WARN] daemon: not running (it starts automatically on first toggle)"),
    }

    println!();
    println!("tips:");
    println!("  sage model list");
    println!("  sage model use <id>");
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
