//! Wayland global shortcut portal integration.

use crate::hotkey::{HotKey, Modifier};
use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use ashpd::desktop::CreateSessionOptions;
use futures_util::StreamExt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum GlobalHotkeyEvent {
    Activated,
    Available(String),
    Unavailable(String),
}

pub fn spawn(chord: String, sender: mpsc::Sender<GlobalHotkeyEvent>) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = sender.send(GlobalHotkeyEvent::Unavailable(error.to_string()));
                return;
            }
        };
        if let Err(error) = runtime.block_on(run(chord, sender.clone())) {
            let _ = sender.send(GlobalHotkeyEvent::Unavailable(error));
        }
    });
}

async fn run(chord: String, sender: mpsc::Sender<GlobalHotkeyEvent>) -> Result<(), String> {
    let portal = GlobalShortcuts::new()
        .await
        .map_err(|error| error.to_string())?;
    if portal.version() == 0 {
        return Err("desktop does not expose the Global Shortcuts portal".into());
    }
    let session = portal
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(|error| error.to_string())?;
    let trigger = portal_trigger(&chord)?;
    let shortcut = NewShortcut::new("toggle", "Toggle Sage voice recording")
        .preferred_trigger(Some(trigger.as_str()));
    let request = portal
        .bind_shortcuts(&session, &[shortcut], None, BindShortcutsOptions::default())
        .await
        .map_err(|error| error.to_string())?;
    let response = request.response().map_err(|error| error.to_string())?;
    if response.shortcuts().is_empty() {
        return Err("global shortcut was not approved by the desktop".into());
    }
    let description = response.shortcuts()[0].trigger_description().to_string();
    let _ = sender.send(GlobalHotkeyEvent::Available(description));

    let mut activated = portal
        .receive_activated()
        .await
        .map_err(|error| error.to_string())?;
    let mut last_activation: Option<Instant> = None;
    while let Some(event) = activated.next().await {
        if event.shortcut_id() == "toggle"
            && last_activation.is_none_or(|last| last.elapsed() >= Duration::from_millis(300))
        {
            last_activation = Some(Instant::now());
            let _ = sender.send(GlobalHotkeyEvent::Activated);
        }
    }
    Err("global shortcut portal session ended".into())
}

fn portal_trigger(chord: &str) -> Result<String, String> {
    let hotkey = HotKey::parse(chord)?;
    let mut parts = Vec::new();
    for modifier in &hotkey.modifiers {
        parts.push(
            match modifier {
                Modifier::Ctrl => "CTRL",
                Modifier::Shift => "SHIFT",
                Modifier::Alt => "ALT",
                Modifier::Super => "LOGO",
            }
            .to_string(),
        );
    }
    let key = hotkey
        .key
        .ok_or_else(|| "portal hotkeys require a non-modifier key".to_string())?;
    parts.push(key.to_uppercase());
    Ok(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_config_chord_to_portal_trigger() {
        assert_eq!(portal_trigger("ctrl+shift+r").unwrap(), "CTRL+SHIFT+R");
    }
}
