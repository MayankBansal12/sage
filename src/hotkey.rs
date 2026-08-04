//! Global hotkey chord parsing and normalization.

use std::collections::BTreeSet;

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
            .map(|m| {
                match m {
                    Modifier::Ctrl => "ctrl",
                    Modifier::Shift => "shift",
                    Modifier::Alt => "alt",
                    Modifier::Super => "super",
                }
                .to_string()
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hotkey() {
        let hk = HotKey::parse("Ctrl+Shift+R").unwrap();
        assert_eq!(hk.key.as_deref(), Some("r"));
        assert!(hk.modifiers.contains(&Modifier::Ctrl));
        assert!(hk.modifiers.contains(&Modifier::Shift));
    }
}
