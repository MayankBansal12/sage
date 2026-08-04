//! Accessibility target capture and paste-on-focus delivery.

use atspi::events::{focus::FocusEvent, object::StateChangedEvent};
use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::{AccessibilityConnection, FocusEvents, Interface, ObjectRefOwned, Role, State};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TargetToken {
    pub bus_name: String,
    pub object_path: String,
    pub application_name: String,
    pub window_name: String,
    pub component_name: String,
    pub role: String,
    pub editable: bool,
}

impl TargetToken {
    pub fn key(&self) -> String {
        format!("{}:{}", self.bus_name, self.object_path)
    }

    pub fn label(&self) -> String {
        let app = if self.application_name.is_empty() {
            "application"
        } else {
            &self.application_name
        };
        let component = if self.component_name.is_empty() {
            &self.role
        } else {
            &self.component_name
        };
        format!("{app} / {component}")
    }

    /// Accessible names and window titles can change while the same control stays alive.
    /// The AT-SPI bus name and object path are the stable identity for its lifetime.
    pub fn same_control(&self, other: &Self) -> bool {
        self.bus_name == other.bus_name && self.object_path == other.object_path
    }
}

pub enum TargetEvent {
    FocusChanged(TargetToken),
    Available,
    Unavailable(String),
}

enum TargetCommand {
    Insert {
        target: TargetToken,
        text: String,
        response: mpsc::Sender<Result<(), String>>,
    },
    Capture {
        response: mpsc::Sender<Option<TargetToken>>,
    },
}

#[derive(Clone)]
pub struct TargetHandle {
    latest: Arc<Mutex<Option<TargetToken>>>,
    command_tx: mpsc::Sender<TargetCommand>,
}

impl TargetHandle {
    pub fn current(&self) -> Option<TargetToken> {
        self.latest.lock().ok().and_then(|value| value.clone())
    }

    pub fn insert(&self, target: TargetToken, text: String) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.command_tx
            .send(TargetCommand::Insert {
                target,
                text,
                response: tx,
            })
            .map_err(|_| "accessibility service is not running".to_string())?;
        rx.recv_timeout(Duration::from_secs(3))
            .map_err(|_| "accessibility insertion timed out".to_string())?
    }

    pub fn capture(&self) -> Option<TargetToken> {
        if let Some(target) = self.current() {
            return Some(target);
        }
        let (tx, rx) = mpsc::channel();
        self.command_tx
            .send(TargetCommand::Capture { response: tx })
            .ok()?;
        rx.recv_timeout(Duration::from_secs(3)).ok().flatten()
    }
}

pub fn spawn(event_tx: mpsc::Sender<TargetEvent>) -> TargetHandle {
    let latest = Arc::new(Mutex::new(None));
    let latest_thread = latest.clone();
    let (command_tx, command_rx) = mpsc::channel();

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = event_tx.send(TargetEvent::Unavailable(error.to_string()));
                return;
            }
        };
        runtime.block_on(run_monitor(latest_thread, command_rx, event_tx));
    });

    TargetHandle { latest, command_tx }
}

async fn run_monitor(
    latest: Arc<Mutex<Option<TargetToken>>>,
    command_rx: mpsc::Receiver<TargetCommand>,
    event_tx: mpsc::Sender<TargetEvent>,
) {
    let connection = match AccessibilityConnection::new().await {
        Ok(connection) => connection,
        Err(error) => {
            let _ = event_tx.send(TargetEvent::Unavailable(error.to_string()));
            return;
        }
    };

    if let Err(error) = connection.register_event::<FocusEvents>().await {
        let _ = event_tx.send(TargetEvent::Unavailable(error.to_string()));
        return;
    }
    if let Err(error) = connection.register_event::<StateChangedEvent>().await {
        let _ = event_tx.send(TargetEvent::Unavailable(error.to_string()));
        return;
    }
    let _ = event_tx.send(TargetEvent::Available);

    let mut events = Box::pin(connection.event_stream());
    let mut tick = tokio::time::interval(Duration::from_millis(40));

    loop {
        tokio::select! {
            event = events.next() => {
                let Some(Ok(event)) = event else { continue };
                let object = if let Ok(focus) = FocusEvent::try_from(event.clone()) {
                    Some(focus.item)
                } else if let Ok(state) = StateChangedEvent::try_from(event) {
                    (state.state == State::Focused && state.enabled).then_some(state.item)
                } else {
                    None
                };
                if let Some(object) = object {
                    if let Ok(token) = inspect_target(&connection, object).await {
                        if let Ok(mut value) = latest.lock() {
                            *value = Some(token.clone());
                        }
                        let _ = event_tx.send(TargetEvent::FocusChanged(token));
                    }
                }
            }
            _ = tick.tick() => {
                while let Ok(command) = command_rx.try_recv() {
                    match command {
                        TargetCommand::Insert { target, text, response } => {
                            let result = insert_text(&connection, &target, &text).await;
                            let _ = response.send(result);
                        }
                        TargetCommand::Capture { response } => {
                            let captured = tokio::time::timeout(
                                Duration::from_secs(2),
                                capture_current(&connection),
                            )
                            .await
                            .ok()
                            .flatten();
                            if let Some(target) = &captured {
                                if let Ok(mut value) = latest.lock() {
                                    *value = Some(target.clone());
                                }
                            }
                            let _ = response.send(captured);
                        }
                    }
                }
            }
        }
    }
}

async fn capture_current(connection: &AccessibilityConnection) -> Option<TargetToken> {
    let root = connection.root_accessible_on_registry().await.ok()?;
    let mut stack = root.get_children().await.ok()?;
    let mut visited = 0usize;
    while let Some(reference) = stack.pop() {
        visited += 1;
        if visited > 4000 {
            break;
        }
        if reference.is_null() {
            continue;
        }
        let proxy = match reference.as_accessible_proxy(connection.connection()).await {
            Ok(proxy) => proxy,
            Err(_) => continue,
        };
        let state = proxy.get_state().await.unwrap_or_default();
        if state.contains(State::Focused) {
            if let Ok(target) = inspect_target(connection, reference.clone()).await {
                if target.editable {
                    return Some(target);
                }
            }
        }
        if let Ok(mut children) = proxy.get_children().await {
            stack.append(&mut children);
        }
    }
    None
}

async fn inspect_target(
    connection: &AccessibilityConnection,
    object: ObjectRefOwned,
) -> Result<TargetToken, String> {
    let proxy = object
        .as_accessible_proxy(connection.connection())
        .await
        .map_err(|error| error.to_string())?;
    let interfaces = proxy
        .get_interfaces()
        .await
        .map_err(|error| error.to_string())?;
    let role = proxy.get_role().await.map_err(|error| error.to_string())?;
    let state = proxy.get_state().await.map_err(|error| error.to_string())?;
    let component_name = proxy.name().await.unwrap_or_default();
    let editable = interfaces.contains(Interface::EditableText)
        && interfaces.contains(Interface::Text)
        && state.contains(State::Editable)
        && role != Role::PasswordText;

    let application_name = match proxy.get_application().await {
        Ok(app) => match app.as_accessible_proxy(connection.connection()).await {
            Ok(app_proxy) => app_proxy.name().await.unwrap_or_default(),
            Err(_) => String::new(),
        },
        Err(_) => String::new(),
    };
    let window_name = find_window_name(connection, &proxy).await;

    Ok(TargetToken {
        bus_name: object.name().map(ToString::to_string).unwrap_or_default(),
        object_path: object.path_as_str().to_string(),
        application_name,
        window_name,
        component_name,
        role: format!("{role:?}"),
        editable,
    })
}

async fn find_window_name(
    connection: &AccessibilityConnection,
    proxy: &AccessibleProxy<'_>,
) -> String {
    let mut parent = proxy.parent().await.ok();
    for _ in 0..16 {
        let Some(reference) = parent else { break };
        if reference.is_null() {
            break;
        }
        let Ok(parent_proxy) = reference.as_accessible_proxy(connection.connection()).await else {
            break;
        };
        let role = parent_proxy.get_role().await.unwrap_or(Role::Unknown);
        if matches!(role, Role::Frame | Role::Window | Role::Dialog) {
            return parent_proxy.name().await.unwrap_or_default();
        }
        parent = parent_proxy.parent().await.ok();
    }
    String::new()
}

async fn insert_text(
    connection: &AccessibilityConnection,
    target: &TargetToken,
    text: &str,
) -> Result<(), String> {
    let proxy = AccessibleProxy::builder(connection.connection())
        .destination(target.bus_name.as_str())
        .map_err(|error| error.to_string())?
        .path(target.object_path.as_str())
        .map_err(|error| error.to_string())?
        .cache_properties(atspi::zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(|error| error.to_string())?;
    let role = proxy.get_role().await.map_err(|error| error.to_string())?;
    let state = proxy.get_state().await.map_err(|error| error.to_string())?;
    if role == Role::PasswordText {
        return Err("refusing to insert into a password field".into());
    }
    if !state.contains(State::Focused) || !state.contains(State::Editable) {
        return Err("captured target is no longer focused and editable".into());
    }
    let proxies = proxy.proxies().await.map_err(|error| error.to_string())?;
    let text_proxy = proxies.text().await.map_err(|error| error.to_string())?;
    let editable = proxies
        .editable_text()
        .await
        .map_err(|error| error.to_string())?;
    let caret = text_proxy
        .caret_offset()
        .await
        .map_err(|error| error.to_string())?;
    let mut insertion_offset = caret;
    if text_proxy.get_n_selections().await.unwrap_or(0) > 0 {
        if let Ok((start, end)) = text_proxy.get_selection(0).await {
            let selection_start = start.min(end);
            let selection_end = start.max(end);
            if selection_start != selection_end
                && editable
                    .delete_text(selection_start, selection_end)
                    .await
                    .map_err(|error| error.to_string())?
            {
                insertion_offset = selection_start;
            }
        }
    }
    let length = i32::try_from(text.chars().count())
        .map_err(|_| "transcript is too large to insert".to_string())?;
    if editable
        .insert_text(insertion_offset, text, length)
        .await
        .map_err(|error| error.to_string())?
    {
        Ok(())
    } else {
        Err("target rejected text insertion".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_identity_ignores_mutable_labels() {
        let original = TargetToken {
            bus_name: ":1.42".into(),
            object_path: "/org/app/field".into(),
            application_name: "Browser".into(),
            window_name: "Old tab title".into(),
            component_name: "Empty editor".into(),
            role: "Entry".into(),
            editable: true,
        };
        let mut changed = original.clone();
        changed.window_name = "New tab title".into();
        changed.component_name = "Editor with text".into();
        assert!(original.same_control(&changed));
    }
}
