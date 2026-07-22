//! Pure hotkey state machine (Hex-inspired, phase 5).
//!
//! Platform input is separate; this module only processes abstract events.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
    Super,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotKey {
    pub key: Option<String>, // lowercase letter/digit/name e.g. "r", "space"
    pub modifiers: BTreeSet<Modifier>,
}

impl HotKey {
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim().to_ascii_lowercase();
        if s.is_empty() {
            return Err("empty hotkey".into());
        }
        let mut modifiers = BTreeSet::new();
        let mut key: Option<String> = None;
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part {
                "ctrl" | "control" | "ctl" => {
                    modifiers.insert(Modifier::Ctrl);
                }
                "shift" => {
                    modifiers.insert(Modifier::Shift);
                }
                "alt" | "option" => {
                    modifiers.insert(Modifier::Alt);
                }
                "super" | "meta" | "cmd" | "win" | "windows" => {
                    modifiers.insert(Modifier::Super);
                }
                other => {
                    if key.is_some() {
                        return Err(format!("multiple keys in hotkey: {s}"));
                    }
                    key = Some(normalize_key(other));
                }
            }
        }
        if key.is_none() && modifiers.is_empty() {
            return Err("hotkey needs a key or modifiers".into());
        }
        Ok(Self { key, modifiers })
    }

    pub fn display(&self) -> String {
        let mut parts: Vec<String> = self
            .modifiers
            .iter()
            .map(|m| match m {
                Modifier::Ctrl => "ctrl",
                Modifier::Shift => "shift",
                Modifier::Alt => "alt",
                Modifier::Super => "super",
            }
            .to_string())
            .collect();
        if let Some(k) = &self.key {
            parts.push(k.clone());
        }
        parts.join("+")
    }
}

fn normalize_key(k: &str) -> String {
    match k {
        " " | "spc" => "space".into(),
        "return" | "enter" => "enter".into(),
        "esc" => "escape".into(),
        other => other.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Option<String>,
    pub modifiers: BTreeSet<Modifier>,
    pub pressed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotKeyOutput {
    StartRecording,
    StopRecording,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    PressAndHold,
    DoubleTapLock,
}

/// Pure processor — Hex-style press-hold + optional double-tap lock.
pub struct HotKeyProcessor {
    pub hotkey: HotKey,
    pub double_tap_lock: bool,
    pub double_tap_threshold: Duration,
    pub minimum_key_time: Duration,
    phase: Phase,
    press_started: Option<Instant>,
    last_release: Option<Instant>,
    dirty: bool,
}

impl HotKeyProcessor {
    pub fn new(hotkey: HotKey) -> Self {
        Self {
            hotkey,
            double_tap_lock: true,
            double_tap_threshold: Duration::from_millis(300),
            minimum_key_time: Duration::from_millis(200),
            phase: Phase::Idle,
            press_started: None,
            last_release: None,
            dirty: false,
        }
    }

    pub fn process(&mut self, event: &KeyEvent, now: Instant) -> Option<HotKeyOutput> {
        // Escape always cancels when active
        if event.pressed
            && event.key.as_deref() == Some("escape")
            && event.modifiers.is_empty()
            && self.phase != Phase::Idle
        {
            self.phase = Phase::Idle;
            self.press_started = None;
            self.dirty = true;
            return Some(HotKeyOutput::Cancel);
        }

        if self.dirty {
            if !event.pressed && event.key.is_none() && event.modifiers.is_empty() {
                self.dirty = false;
            } else if is_full_release(event) {
                self.dirty = false;
            } else {
                return None;
            }
        }

        let matches = chord_matches(&self.hotkey, event);

        match self.phase {
            Phase::Idle => {
                if event.pressed && matches {
                    self.phase = Phase::PressAndHold;
                    self.press_started = Some(now);
                    // Double-tap: if previous release was recent, enter lock after second press release
                    return Some(HotKeyOutput::StartRecording);
                }
                None
            }
            Phase::PressAndHold => {
                if !event.pressed && chord_released(&self.hotkey, event) {
                    let held = self
                        .press_started
                        .map(|t| now.duration_since(t))
                        .unwrap_or_default();
                    // Double-tap lock: quick second tap locks
                    if self.double_tap_lock {
                        if let Some(prev) = self.last_release {
                            if now.duration_since(prev) <= self.double_tap_threshold
                                && held <= self.double_tap_threshold
                            {
                                self.phase = Phase::DoubleTapLock;
                                self.press_started = None;
                                self.last_release = Some(now);
                                return None; // stay recording
                            }
                        }
                    }
                    self.last_release = Some(now);
                    self.phase = Phase::Idle;
                    self.press_started = None;
                    return Some(HotKeyOutput::StopRecording);
                }
                // Different key while holding — cancel if early
                if event.pressed && !matches {
                    let held = self
                        .press_started
                        .map(|t| now.duration_since(t))
                        .unwrap_or_default();
                    if held < self.minimum_key_time {
                        self.phase = Phase::Idle;
                        self.press_started = None;
                        self.dirty = true;
                        return Some(HotKeyOutput::Cancel);
                    }
                }
                None
            }
            Phase::DoubleTapLock => {
                if event.pressed && matches {
                    self.phase = Phase::Idle;
                    self.press_started = None;
                    return Some(HotKeyOutput::StopRecording);
                }
                None
            }
        }
    }
}

fn chord_matches(hotkey: &HotKey, event: &KeyEvent) -> bool {
    if !hotkey.modifiers.is_subset(&event.modifiers) {
        return false;
    }
    // Require exact modifiers for non-modifier-only
    if hotkey.modifiers != event.modifiers {
        // allow extra? Hex ignores after threshold; for match require equality
        if !hotkey.modifiers.is_empty() || hotkey.key.is_some() {
            if hotkey.modifiers != event.modifiers {
                return false;
            }
        }
    }
    match &hotkey.key {
        Some(k) => event.key.as_deref() == Some(k.as_str()),
        None => event.key.is_none() && !event.modifiers.is_empty(),
    }
}

fn chord_released(hotkey: &HotKey, event: &KeyEvent) -> bool {
    // Release of main key or any required modifier
    if let Some(k) = &hotkey.key {
        if event.key.as_deref() == Some(k.as_str()) && !event.pressed {
            return true;
        }
    }
    for m in &hotkey.modifiers {
        if !event.modifiers.contains(m) {
            return true;
        }
    }
    !event.pressed && event.key.is_none()
}

fn is_full_release(event: &KeyEvent) -> bool {
    !event.pressed && event.key.is_none() && event.modifiers.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(list: &[Modifier]) -> BTreeSet<Modifier> {
        list.iter().cloned().collect()
    }

    #[test]
    fn parse_hotkey() {
        let hk = HotKey::parse("Ctrl+Shift+R").unwrap();
        assert_eq!(hk.key.as_deref(), Some("r"));
        assert!(hk.modifiers.contains(&Modifier::Ctrl));
        assert!(hk.modifiers.contains(&Modifier::Shift));
    }

    #[test]
    fn press_hold_start_stop() {
        let hk = HotKey::parse("ctrl+r").unwrap();
        let mut p = HotKeyProcessor::new(hk);
        p.double_tap_lock = false;
        let t0 = Instant::now();
        let down = KeyEvent {
            key: Some("r".into()),
            modifiers: mods(&[Modifier::Ctrl]),
            pressed: true,
        };
        assert_eq!(p.process(&down, t0), Some(HotKeyOutput::StartRecording));
        let up = KeyEvent {
            key: Some("r".into()),
            modifiers: mods(&[Modifier::Ctrl]),
            pressed: false,
        };
        assert_eq!(
            p.process(&up, t0 + Duration::from_millis(500)),
            Some(HotKeyOutput::StopRecording)
        );
    }

    #[test]
    fn escape_cancels() {
        let hk = HotKey::parse("ctrl+r").unwrap();
        let mut p = HotKeyProcessor::new(hk);
        let t0 = Instant::now();
        let down = KeyEvent {
            key: Some("r".into()),
            modifiers: mods(&[Modifier::Ctrl]),
            pressed: true,
        };
        p.process(&down, t0);
        let esc = KeyEvent {
            key: Some("escape".into()),
            modifiers: BTreeSet::new(),
            pressed: true,
        };
        assert_eq!(p.process(&esc, t0), Some(HotKeyOutput::Cancel));
    }
}
