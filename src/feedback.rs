//! Lightweight user feedback (phase 7) — terminal bell as portable "sounds".

use crate::config::Settings;

pub enum Cue {
    Start,
    Stop,
    Cancel,
    Error,
}

pub fn play(cue: Cue, settings: &Settings) {
    if !settings.sound_effects {
        return;
    }
    // ASCII BEL — works over most terminals; no audio device required.
    let _ = cue;
    eprint!("\x07");
    let _ = std::io::Write::flush(&mut std::io::stderr());
}
