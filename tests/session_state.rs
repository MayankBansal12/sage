//! Phase 1 integration-style tests that do not require a real microphone.
//!
//! Audio capture is exercised manually (see README). These tests cover
//! state file behavior and CLI status when idle.

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
    // cargo sets CARGO_BIN_EXE_sage for integration tests
    PathBuf::from(env!("CARGO_BIN_EXE_sage"))
}

fn run_sage(data_dir: &PathBuf, args: &[&str]) -> std::process::Output {
    Command::new(sage_bin())
        .args(args)
        .env("SAGE_DATA_DIR", data_dir)
        .output()
        .expect("failed to run sage")
}

#[test]
fn status_when_idle_shows_idle() {
    let data = unique_data_dir();
    let out = run_sage(&data, &["status"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("status: idle"),
        "unexpected status output: {stdout}"
    );
    assert!(
        stdout.contains("last_recording: (none)"),
        "unexpected last_recording: {stdout}"
    );
}

#[test]
fn last_when_empty_fails() {
    let data = unique_data_dir();
    let out = run_sage(&data, &["last"]);
    assert!(!out.status.success(), "last should fail with no transcript");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("no transcript"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn paths_prints_layout() {
    let data = unique_data_dir();
    let out = run_sage(&data, &["paths"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "paths failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("data_dir:"), "missing data_dir: {stdout}");
    assert!(stdout.contains("config_dir:"), "missing config_dir: {stdout}");
    assert!(stdout.contains("recordings_dir:"), "missing recordings_dir: {stdout}");
    assert!(stdout.contains("models_dir:"), "missing models_dir: {stdout}");
    assert!(stdout.contains("temp_dir:"), "missing temp_dir: {stdout}");
}

#[test]
fn stop_when_idle_fails() {
    let data = unique_data_dir();
    let out = run_sage(&data, &["stop"]);
    assert!(!out.status.success(), "stop should fail when idle");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("not recording"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn double_start_is_rejected_if_fake_worker_alive() {
    // Simulate a live recording state with this process's PID (alive).
    let data = unique_data_dir();
    let run = data.join("run");
    fs::create_dir_all(&run).unwrap();
    let state = format!(
        r#"{{
  "status": "recording",
  "worker_pid": {},
  "started_at": "2026-01-01T00:00:00+00:00",
  "last_recording_path": null,
  "last_error": null
}}"#,
        std::process::id()
    );
    fs::write(run.join("state.json"), state).unwrap();

    let out = run_sage(&data, &["start"]);
    assert!(!out.status.success(), "second start should fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("already recording"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn stale_recording_state_is_reconciled_on_status() {
    let data = unique_data_dir();
    let run = data.join("run");
    fs::create_dir_all(&run).unwrap();
    // PID 1 may exist on Linux as init — use a very high unlikely PID.
    // If it somehow exists, test still checks status command succeeds.
    let dead_pid = 2147483646u32;
    let state = format!(
        r#"{{
  "status": "recording",
  "worker_pid": {dead_pid},
  "started_at": "2026-01-01T00:00:00+00:00",
  "last_recording_path": null,
  "last_error": null
}}"#
    );
    fs::write(run.join("state.json"), state).unwrap();

    let out = run_sage(&data, &["status"]);
    assert!(
        out.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Either reconciled to idle, or still recording if that PID is weirdly alive.
    assert!(
        stdout.contains("status: idle") || stdout.contains("status: recording"),
        "unexpected: {stdout}"
    );
    if !std::path::Path::new(&format!("/proc/{dead_pid}")).exists() {
        assert!(
            stdout.contains("status: idle"),
            "expected stale state reset to idle: {stdout}"
        );
    }
}
