//! Clipboard + paste delivery (phases 4 & 6).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum OutputError {
    #[error("clipboard error: {0}")]
    Clipboard(String),
    #[error("paste error: {0}")]
    Paste(String),
}

/// Copy text to the system clipboard.
pub fn copy_to_clipboard(text: &str) -> Result<(), OutputError> {
    let mut cb = arboard::Clipboard::new().map_err(|e| OutputError::Clipboard(e.to_string()))?;
    cb.set_text(text.to_string())
        .map_err(|e| OutputError::Clipboard(e.to_string()))?;
    Ok(())
}

/// Best-effort paste into the focused app (clipboard must already hold the text).
///
/// Tries, in order: `wtype`, `xdotool`, `ydotool`.
pub fn paste_to_focused_app() -> Result<(), OutputError> {
    // Small delay so clipboard is ready for the target app.
    std::thread::sleep(std::time::Duration::from_millis(50));

    if try_cmd("wtype", &["-M", "ctrl", "-k", "v", "-m", "ctrl"]) {
        return Ok(());
    }
    if try_cmd("xdotool", &["key", "--clearmodifiers", "ctrl+v"]) {
        return Ok(());
    }
    if try_cmd("ydotool", &["key", "29:1", "47:1", "47:0", "29:0"]) {
        return Ok(());
    }
    Err(OutputError::Paste(
        "no paste helper found (install wtype, xdotool, or ydotool)".into(),
    ))
}

fn try_cmd(bin: &str, args: &[&str]) -> bool {
    std::process::Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Deliver transcript according to settings.
pub fn deliver(text: &str, copy: bool, auto_paste: bool) -> Vec<String> {
    let mut notes = Vec::new();
    if !copy {
        notes.push("clipboard: skipped (copy_to_clipboard=false)".into());
        return notes;
    }
    match copy_to_clipboard(text) {
        Ok(()) => notes.push("clipboard: copied".into()),
        Err(e) => notes.push(format!("clipboard: failed ({e})")),
    }
    if auto_paste {
        match paste_to_focused_app() {
            Ok(()) => notes.push("paste: sent Ctrl+V".into()),
            Err(e) => notes.push(format!("paste: skipped ({e})")),
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deliver_skip_copy() {
        let notes = deliver("hi", false, false);
        assert!(notes.iter().any(|n| n.contains("skipped")));
    }

    // Clipboard may fail on headless CI — just ensure it doesn't panic.
    #[test]
    fn copy_does_not_panic() {
        let _ = copy_to_clipboard("sage-test");
    }
}
