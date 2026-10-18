//! Service-owned state and a process-wide event cursor. No external configuration probes.
use crate::{ipc, overlay};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

pub const EVENT_CHANNEL: &str = "project-assistant://event/v1";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub id: &'static str,
    pub state: &'static str,
    pub reason_code: Option<&'static str>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActiveContext {
    pub project_id: String,
    pub session_id: String,
    pub profile_id: String,
    pub model_id: String,
    pub effort: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualCcSwitchStatus {
    management: &'static str,
    integration: &'static str,
    read_supported: bool,
    switch_supported: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub api_version: u32,
    pub stream_id: String,
    pub sequence: u64,
    pub config_state: &'static str,
    pub services: Vec<ServiceStatus>,
    pub cc_switch: ManualCcSwitchStatus,
    pub active_context: Option<ActiveContext>,
    pub task_status: &'static str,
    pub overlay: overlay::OverlayView,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StatePayload {
    AppStateChanged,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppEvent {
    api_version: u32,
    event_id: String,
    stream_id: String,
    sequence: u64,
    occurred_at: u64,
    correlation_id: String,
    payload: StatePayload,
}

pub struct StateData {
    stream_id: String,
    sequence: u64,
    pub active_context: Option<ActiveContext>,
    #[cfg(debug_assertions)]
    pub last_event: Option<AppEvent>,
}
impl Default for StateData {
    fn default() -> Self {
        Self {
            stream_id: format!("stream-{}", ipc::correlation_id()),
            sequence: 0,
            active_context: None,
            #[cfg(debug_assertions)]
            last_event: None,
        }
    }
}
#[derive(Default)]
pub struct AppState(pub Mutex<StateData>);

impl StateData {
    fn next_event(&mut self, correlation_id: String) -> AppEvent {
        self.sequence += 1;
        let event = AppEvent {
            api_version: ipc::API_VERSION,
            event_id: format!("{}-{}", self.stream_id, self.sequence),
            stream_id: self.stream_id.clone(),
            sequence: self.sequence,
            occurred_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            correlation_id,
            payload: StatePayload::AppStateChanged,
        };
        #[cfg(debug_assertions)]
        {
            self.last_event = Some(event.clone());
        }
        event
    }

    fn snapshot(
        &self,
        mut overlay: overlay::OverlayView,
        private_draft: bool,
        storage: crate::storage::StorageHealth,
    ) -> AppStatus {
        if !private_draft {
            overlay.draft.clear();
        }
        AppStatus {
            api_version: ipc::API_VERSION,
            stream_id: self.stream_id.clone(),
            sequence: self.sequence,
            config_state: "unconfigured",
            active_context: self.active_context.clone(),
            task_status: "idle",
            overlay,
            cc_switch: ManualCcSwitchStatus {
                management: "user_manual",
                integration: "disabled",
                read_supported: false,
                switch_supported: false,
            },
            services: [
                ("desktop", "ready", None),
                ("ipc", "ready", None),
                ("events", "ready", None),
                ("overlay", "ready", None),
                ("storage", storage.state, storage.reason_code),
                (
                    "settings",
                    "unavailable",
                    Some("PA_SETTINGS_NOT_IMPLEMENTED"),
                ),
                ("logging", "unavailable", Some("PA_LOGGING_NOT_IMPLEMENTED")),
                (
                    "credentials",
                    "unavailable",
                    Some("PA_CREDENTIALS_NOT_IMPLEMENTED"),
                ),
                (
                    "providers",
                    "unavailable",
                    Some("PA_PROVIDER_NOT_IMPLEMENTED"),
                ),
                ("codex", "unavailable", Some("PA_CODEX_NOT_IMPLEMENTED")),
                (
                    "projects",
                    "unavailable",
                    Some("PA_PROJECTS_NOT_IMPLEMENTED"),
                ),
                ("tasks", "unavailable", Some("PA_TASKS_NOT_IMPLEMENTED")),
            ]
            .into_iter()
            .map(|(id, state, reason_code)| ServiceStatus {
                id,
                state,
                reason_code,
            })
            .collect(),
        }
    }
}

fn storage_state(app: &tauri::AppHandle) -> crate::storage::StorageHealth {
    app.state::<crate::storage::StorageState>().health()
}

pub fn status(app: &tauri::AppHandle, private_draft: bool) -> AppStatus {
    let state = app.state::<AppState>();
    let data = state.0.lock().unwrap();
    let mut result = data.snapshot(overlay::view(app), private_draft, storage_state(app));
    if !app.state::<crate::desktop::DesktopState>().phase0 {
        let logs = app.state::<crate::logging::LoggingState>().status();
        let logging = result.services.iter_mut().find(|s| s.id == "logging").unwrap();
        logging.state = if logs.enabled { "ready" } else { "degraded" };
        logging.reason_code = logs.error_code;
        let failure = *app
            .state::<crate::settings::SettingsState>()
            .failure
            .lock()
            .unwrap();
        let settings = result
            .services
            .iter_mut()
            .find(|s| s.id == "settings")
            .unwrap();
        settings.state = if failure.is_none() && storage_state(app).state == "ready" {
            "ready"
        } else {
            "unavailable"
        };
        settings.reason_code = failure.map(|code| match code {
            ipc::ErrorCode::SettingsCorrupt => "PA_SETTINGS_CORRUPT",
            ipc::ErrorCode::SettingsRecoveryFailed => "PA_SETTINGS_RECOVERY_FAILED",
            _ => "PA_SETTINGS_STORAGE_FAILED",
        });
    }
    if app.state::<crate::desktop::DesktopState>().phase0 {
        let service = result
            .services
            .iter_mut()
            .find(|s| s.id == "overlay")
            .unwrap();
        service.state = "unavailable";
        service.reason_code = Some("PA_OVERLAY_PHASE0_DISABLED");
    } else if result.overlay.window_error.is_some() || result.overlay.shortcut_error.is_some() {
        let service = result
            .services
            .iter_mut()
            .find(|s| s.id == "overlay")
            .unwrap();
        service.state = "degraded";
        service.reason_code = result
            .overlay
            .window_error
            .or(result.overlay.shortcut_error);
    }
    result
}

fn emit(app: &tauri::AppHandle, event: &AppEvent) {
    // Fixed, bounded invalidation only: no draft, Key, task prompt or raw exception.
    for label in ["main", "overlay"] {
        let _ = app.emit_to(
            tauri::EventTarget::WebviewWindow {
                label: label.into(),
            },
            EVENT_CHANNEL,
            event,
        );
    }
}

pub fn publish(app: &tauri::AppHandle, correlation_id: String) {
    let state = app.state::<AppState>();
    let mut data = state.0.lock().unwrap();
    // Serialize publishing and snapshot reads with the cursor lock.
    emit(app, &data.next_event(correlation_id));
}

#[cfg(debug_assertions)]
pub fn fixture(
    app: &tauri::AppHandle,
    action: &str,
    correlation_id: String,
) -> Result<(), ipc::ErrorCode> {
    let state = app.state::<AppState>();
    let mut data = state.0.lock().unwrap();
    match action {
        "context" => {
            data.active_context = Some(ActiveContext {
                project_id: "synthetic-project".into(),
                session_id: "synthetic-session".into(),
                profile_id: "synthetic-profile".into(),
                model_id: "synthetic-model".into(),
                effort: Some("high".into()),
            })
        }
        "clear" => data.active_context = None,
        "gap" => data.sequence += 1,
        "restart" => {
            data.stream_id = format!("stream-{}", ipc::correlation_id());
            data.sequence = 0;
        }
        "replay" => {
            if let Some(event) = &data.last_event {
                emit(app, event);
            }
            return Ok(());
        }
        _ => return Err(ipc::ErrorCode::InvalidRequest),
    }
    emit(app, &data.next_event(correlation_id));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streams_sequences_and_public_snapshots_are_safe() {
        let mut state = StateData::default();
        assert_ne!(state.stream_id, StateData::default().stream_id);
        let event = state.next_event(ipc::correlation_id());
        assert_eq!(event.sequence, 1);
        assert_eq!(state.next_event(ipc::correlation_id()).sequence, 2);
        let wire = serde_json::to_string(&event).unwrap();
        assert!(wire.len() < 16384);
        assert!(!wire.contains("draft"));
        let overlay = overlay::OverlayView {
            enabled: true,
            expanded: false,
            shortcut: "Ctrl+Alt+Space".into(),
            shortcut_error: None,
            window_error: None,
            draft: "synthetic-private-draft".into(),
            tray_ready: true,
        };
        let snapshot = state.snapshot(
            overlay.clone(),
            false,
            crate::storage::StorageHealth {
                state: "unavailable",
                reason_code: Some("PA_STORAGE_NOT_INITIALIZED"),
            },
        );
        assert!(snapshot.overlay.draft.is_empty());
        assert_eq!(
            state
                .snapshot(
                    overlay,
                    true,
                    crate::storage::StorageHealth {
                        state: "unavailable",
                        reason_code: Some("PA_STORAGE_NOT_INITIALIZED"),
                    },
                )
                .overlay
                .draft,
            "synthetic-private-draft"
        );
        assert_eq!(snapshot.sequence, 2);
        assert_eq!(snapshot.cc_switch.integration, "disabled");
        assert!(
            snapshot
                .services
                .iter()
                .filter(|s| s.state == "unavailable")
                .all(|s| s.reason_code.is_some())
        );
    }
}
