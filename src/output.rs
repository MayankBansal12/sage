//! Clipboard fallback for unsupported accessibility targets.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum OutputError {
    #[error("clipboard error: {0}")]
    Clipboard(String),
}

/// Copy text to the system clipboard.
pub fn copy_to_clipboard(text: &str) -> Result<(), OutputError> {
    let mut cb = arboard::Clipboard::new().map_err(|e| OutputError::Clipboard(e.to_string()))?;
    cb.set_text(text.to_string())
        .map_err(|e| OutputError::Clipboard(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Clipboard may fail on headless CI — just ensure it doesn't panic.
    #[test]
    fn copy_does_not_panic() {
        let _ = copy_to_clipboard("sage-test");
    }
}
