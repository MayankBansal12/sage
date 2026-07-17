//! Transcript delivery — phase 4+ (clipboard / paste).
//!
//! Phase 1 stores the last recording path on disk via session state.
//! Phase 2 will store transcript text. This module will then copy/paste it.

#![allow(dead_code)]

use thiserror::Error;

#[derive(Debug, Error)]
pub enum OutputError {
    #[error("clipboard/output is not implemented yet (phase 4)")]
    NotImplemented,
    #[error("{0}")]
    Message(String),
}

/// Deliver text to the user (clipboard, later auto-paste).
pub struct Output;

impl Output {
    pub fn new() -> Self {
        Self
    }

    /// Copy text to the system clipboard (phase 4).
    pub fn copy_to_clipboard(&self, _text: &str) -> Result<(), OutputError> {
        Err(OutputError::NotImplemented)
    }

    /// Paste into the focused app (phase 6) — not implemented.
    pub fn paste_to_focused_app(&self, _text: &str) -> Result<(), OutputError> {
        Err(OutputError::NotImplemented)
    }
}

impl Default for Output {
    fn default() -> Self {
        Self::new()
    }
}
