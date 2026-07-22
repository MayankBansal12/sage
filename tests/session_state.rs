//! Integration tests (no microphone required).

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_data_dir() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("sage-test-{}-{}", std::process::id(), n));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test data dir");
    dir
}

fn sage_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_sage"))
}

fn run_sage(data_dir: &PathBuf, args: &[&str]) -> std::process::Output {
    let cfg = data_dir.join("cfg");
    Command::new(sage_bin())
        .args(args)
        .env("SAGE_DATA_DIR", data_dir)
        .env("SAGE_CONFIG_DIR", &cfg)
        .output()
        .expect("failed to run sage")
}

#[test]
fn status_when_idle_shows_idle() {
    let data = unique_data_dir();
    let out = run_sage(&data, &["status"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("status: idle"), "{stdout}");
}

#[test]
fn stop_when_idle_fails() {
    let data = unique_data_dir();
    let out = run_sage(&data, &["stop"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr)
        .to_lowercase()
        .contains("not recording"));
}

#[test]
fn last_when_empty_fails() {
    let data = unique_data_dir();
    let out = run_sage(&data, &["last"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr)
        .to_lowercase()
        .contains("no transcript"));
}

#[test]
fn cancel_when_idle_fails() {
    let data = unique_data_dir();
    let out = run_sage(&data, &["cancel"]);
    assert!(!out.status.success());
}

#[test]
fn config_set_and_show() {
    let data = unique_data_dir();
    let out = run_sage(&data, &["config", "set", "hotkey", "ctrl+shift+r"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let out = run_sage(&data, &["config", "get", "hotkey"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("ctrl+shift+r"));
    let out = run_sage(&data, &["shortcut", "show"]);
    assert!(out.status.success());
}

#[test]
fn history_empty_list() {
    let data = unique_data_dir();
    let out = run_sage(&data, &["history", "list"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("no history"));
}

#[test]
fn paths_prints_layout() {
    let data = unique_data_dir();
    let out = run_sage(&data, &["paths"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("history_file:"));
    assert!(stdout.contains("data_dir:"));
}

#[test]
fn double_start_is_rejected_if_fake_worker_alive() {
    let data = unique_data_dir();
    let run = data.join("run");
    fs::create_dir_all(&run).unwrap();
    let state = format!(
        r#"{{
  "status": "recording",
  "worker_pid": {},
  "started_at": "2026-01-01T00:00:00+00:00",
  "last_recording_path": null,
  "last_transcript": null,
  "last_error": null
}}"#,
        std::process::id()
    );
    fs::write(run.join("state.json"), state).unwrap();
    let out = run_sage(&data, &["start"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr)
        .to_lowercase()
        .contains("already recording"));
}

#[test]
fn doctor_runs() {
    let data = unique_data_dir();
    let out = run_sage(&data, &["doctor"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("sage doctor"));
}
