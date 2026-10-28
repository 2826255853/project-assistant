//! Step 1.5: typed, single-writer JSONL. Only pass Tauri's backend-resolved
//! app_log_dir to initialize; no IPC may supply a directory or a log payload.
//! Windows is the supported persistence target. Other targets fail closed.
use crate::ipc::ErrorCode;
use serde::Serialize;
use std::{
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
mod windows;

pub const MAX_LINE_BYTES: usize = 4 * 1024;
pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_FILES: usize = 5;
pub const MAX_TOTAL_BYTES: u64 = 25 * 1024 * 1024;
pub const RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const FAILURE_CODE: &str = "PA_INTERNAL_UNEXPECTED";
const FILE_NAMES: [&str; MAX_FILES] = [
    "project-assistant.0.jsonl",
    "project-assistant.1.jsonl",
    "project-assistant.2.jsonl",
    "project-assistant.3.jsonl",
    "project-assistant.4.jsonl",
];
const LOCK_NAME: &str = "project-assistant.writer.lock";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}
impl LogLevel {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "error" => Some(Self::Error),
            "warn" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            _ => None,
        }
    }
    fn accepts(self, level: Self) -> bool {
        (level as u8) <= (self as u8)
    }
}

/// Safe diagnostics only. No path, raw OS error or content is returned.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStatus {
    pub enabled: bool,
    pub level: LogLevel,
    pub error_code: Option<&'static str>,
    pub file_count: usize,
    pub total_bytes: u64,
    pub audit_retention_days: u32,
    pub summary_collection_enabled: bool,
}

#[derive(Clone, Copy, Debug)]
enum Failure {
    Io,
    Identity,
    Security,
    Clock,
    Encoding,
    Unsupported,
}
type Result<T> = std::result::Result<T, Failure>;
fn now_ms() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|v| u64::try_from(v.as_millis()).ok())
        .ok_or(Failure::Clock)
}

/// This enum, rather than the supplied string, is serialized. Unknown commands
/// are discarded. Adding a command is an explicit whitelist change.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Command {
    ProjectsAddFromPicker,
    ProjectGet,
    ProjectsList,
    ProjectOpenDirectory,
    ProjectOpenFile,
    ProjectPickFile,
    ProjectRemove,
    DesktopInitialize,
    AppGetStatus,
    SettingsGet,
    SettingsUpdate,
    OverlayGet,
    OverlayControl,
    OverlayShowEntry,
    FrameworkProbe,
    LogsGetStatus,
    LogsClear,
    ProvidersList,
    ProviderSave,
    ProviderSetEnabled,
    ProviderDelete,
    ProviderModels,
    ProviderTest,
    ProviderSwitch,
    OperationGet,
    OperationCancel,
    CredentialPut,
    CredentialRemove,
    AuditCleanup,
    #[cfg(debug_assertions)]
    IpcFixture,
}
impl Command {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "projects_add_from_picker" => Self::ProjectsAddFromPicker,
            "project_get" => Self::ProjectGet,
            "projects_list" => Self::ProjectsList,
            "project_open_directory" => Self::ProjectOpenDirectory,
            "project_open_file" => Self::ProjectOpenFile,
            "project_pick_file" => Self::ProjectPickFile,
            "project_remove" => Self::ProjectRemove,
            "desktop_initialize" => Self::DesktopInitialize,
            "app_get_status" => Self::AppGetStatus,
            "settings_get" => Self::SettingsGet,
            "settings_update" => Self::SettingsUpdate,
            "overlay_get" => Self::OverlayGet,
            "overlay_control" => Self::OverlayControl,
            "overlay_show_entry" => Self::OverlayShowEntry,
            "framework_probe" => Self::FrameworkProbe,
            "logs_get_status" => Self::LogsGetStatus,
            "logs_clear" => Self::LogsClear,
            "providers_list" => Self::ProvidersList,
            "provider_save" => Self::ProviderSave,
            "provider_set_enabled" => Self::ProviderSetEnabled,
            "provider_delete" => Self::ProviderDelete,
            "provider_models" => Self::ProviderModels,
            "provider_test" => Self::ProviderTest,
            "provider_switch" => Self::ProviderSwitch,
            "operation_get" => Self::OperationGet,
            "operation_cancel" => Self::OperationCancel,
            "credential_put" => Self::CredentialPut,
            "credential_remove" => Self::CredentialRemove,
            "audit_cleanup" => Self::AuditCleanup,
            #[cfg(debug_assertions)]
            "ipc_fixture" => Self::IpcFixture,
            _ => return None,
        })
    }
}

/// Opaque server correlation; never deserialize a client ID into this type.
/// ipc::correlation_id emits ipc-<decimal pid>-<hex instance>-<decimal sequence>.
struct Correlation(String);
impl Correlation {
    fn server(value: &str) -> Option<Self> {
        if value.len() > 96 {
            return None;
        }
        let mut parts = value.split('-');
        if parts.next()? != "ipc" {
            return None;
        }
        let pid = parts.next()?;
        let instance = parts.next()?;
        let sequence = parts.next()?;
        if parts.next().is_some()
            || pid.parse::<u32>().ok()? != std::process::id()
            || instance.is_empty()
            || instance.len() > 32
            || !instance.bytes().all(|c| c.is_ascii_hexdigit())
            || sequence.is_empty()
            || !sequence.bytes().all(|c| c.is_ascii_digit())
            || sequence.parse::<u64>().is_err()
        {
            return None;
        }
        // Reconstruct canonical components: no free text survives validation.
        Some(Self(format!(
            "ipc-{}-{:x}-{}",
            std::process::id(),
            u128::from_str_radix(instance, 16).ok()?,
            sequence.parse::<u64>().ok()?
        )))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Event {
    IpcCompleted,
    LoggingDisabled,
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Succeeded,
    Failed,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Line<'a> {
    api_version: u32,
    occurred_at: u64,
    level: LogLevel,
    command: Command,
    event: Event,
    correlation_id: &'a str,
    outcome: Outcome,
    error_code: Option<ErrorCode>,
}
fn encode(
    command: Command,
    correlation: &Correlation,
    error: Option<ErrorCode>,
    at: u64,
) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(&Line {
        api_version: 1,
        occurred_at: at,
        level: if error.is_some() {
            LogLevel::Warn
        } else {
            LogLevel::Info
        },
        command,
        event: Event::IpcCompleted,
        correlation_id: &correlation.0,
        outcome: if error.is_some() {
            Outcome::Failed
        } else {
            Outcome::Succeeded
        },
        error_code: error,
    })
    .map_err(|_| Failure::Encoding)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_LINE_BYTES {
        return Err(Failure::Encoding);
    }
    Ok(bytes)
}

struct Inner {
    initialized: bool,
    disabled: bool,
    level: LogLevel,
    count: usize,
    bytes: u64,
    #[cfg(windows)]
    writer: Option<windows::Writer>,
}
impl Default for Inner {
    fn default() -> Self {
        Self {
            initialized: false,
            disabled: false,
            level: LogLevel::Info,
            count: 0,
            bytes: 0,
            #[cfg(windows)]
            writer: None,
        }
    }
}
impl Inner {
    fn snapshot(&self) -> LogStatus {
        LogStatus {
            enabled: self.initialized && !self.disabled,
            level: self.level,
            error_code: self.disabled.then_some(FAILURE_CODE),
            file_count: self.count,
            total_bytes: self.bytes,
            audit_retention_days: 90,
            summary_collection_enabled: false,
        }
    }
    fn disable(&mut self) {
        if !self.disabled {
            // Exactly one fixed stderr notice; never print the internal Failure.
            #[derive(Serialize)]
            #[serde(rename_all = "camelCase")]
            struct Notice {
                api_version: u32,
                occurred_at: u64,
                level: LogLevel,
                event: Event,
                outcome: Outcome,
                error_code: &'static str,
            }
            if let Ok(line) = serde_json::to_string(&Notice {
                api_version: 1,
                occurred_at: now_ms().unwrap_or(0),
                level: LogLevel::Error,
                event: Event::LoggingDisabled,
                outcome: Outcome::Failed,
                error_code: FAILURE_CODE,
            }) {
                eprintln!("{line}");
            }
        }
        self.disabled = true;
    }
    fn maintain(&mut self) {
        if !self.initialized || self.disabled {
            return;
        }
        #[cfg(windows)]
        {
            let result = now_ms()
                .and_then(|now| self.writer.as_mut().ok_or(Failure::Identity)?.maintain(now));
            match result {
                Ok((count, bytes)) => {
                    self.count = count;
                    self.bytes = bytes;
                }
                Err(_) => self.disable(),
            }
        }
    }
}

#[derive(Default)]
pub struct LoggingState {
    inner: Mutex<Inner>,
}
impl LoggingState {
    /// Initialize once per process. Failure is sticky even after clear/set_level.
    /// Caller must obtain app_log_dir from app.path().app_log_dir(), not IPC.
    pub fn initialize(&self, app_log_dir: &Path, level: &str) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        if state.initialized || state.disabled {
            return;
        }
        let Some(level) = LogLevel::parse(level) else {
            state.disable();
            return;
        };
        state.level = level;
        #[cfg(windows)]
        {
            match windows::Writer::open(app_log_dir) {
                Ok(writer) => {
                    state.writer = Some(writer);
                    state.initialized = true;
                    state.maintain();
                }
                Err(_) => state.disable(),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = app_log_dir;
            state.disable();
        }
    }

    /// Only static registered command names and this process's server IDs pass.
    /// Raw request/response/error strings are deliberately impossible to attach.
    /// Failure never propagates into cancellation, approval or recovery actions.
    pub fn record(&self, command: &'static str, id: &str, error: Option<ErrorCode>) {
        let (Some(command), Some(correlation)) = (Command::parse(command), Correlation::server(id))
        else {
            return;
        };
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        if !state.initialized || state.disabled {
            return;
        }
        let level = if error.is_some() {
            LogLevel::Warn
        } else {
            LogLevel::Info
        };
        if !state.level.accepts(level) {
            return;
        }
        let result = now_ms().and_then(|now| {
            let line = encode(command, &correlation, error, now)?;
            #[cfg(windows)]
            {
                state
                    .writer
                    .as_mut()
                    .ok_or(Failure::Identity)?
                    .append(&line, now)
            }
            #[cfg(not(windows))]
            {
                let _ = line;
                Err::<(usize, u64), _>(Failure::Unsupported)
            }
        });
        match result {
            Ok((count, bytes)) => {
                state.count = count;
                state.bytes = bytes;
            }
            Err(_) => state.disable(),
        }
    }

    /// Also performs bounded retention maintenance, including while idle.
    /// The parent may call this hourly; only the five exact owned names are read.
    pub fn status(&self) -> LogStatus {
        match self.inner.lock() {
            Ok(mut state) => {
                state.maintain();
                state.snapshot()
            }
            Err(_) => LogStatus {
                enabled: false,
                level: LogLevel::Info,
                error_code: Some(FAILURE_CODE),
                file_count: 0,
                total_bytes: 0,
                audit_retention_days: 90,
                summary_collection_enabled: false,
            },
        }
    }

    /// Explicit user action; no audit/database/backup/lock/directory deletion.
    /// The IPC wrapper must authorize local main and obtain local confirmation.
    pub fn clear(&self) -> std::result::Result<LogStatus, ErrorCode> {
        let mut state = self.inner.lock().map_err(|_| ErrorCode::LogCleanupFailed)?;
        #[cfg(windows)]
        let result = state
            .writer
            .as_mut()
            .ok_or(Failure::Identity)
            .and_then(windows::Writer::clear);
        #[cfg(not(windows))]
        let result = Err::<(), _>(Failure::Unsupported);
        match result {
            Ok(()) => {
                state.count = 0;
                state.bytes = 0;
                Ok(state.snapshot())
            }
            Err(_) => {
                state.disable();
                Err(ErrorCode::LogCleanupFailed)
            }
        }
    }

    /// Call only after the settings transaction commits. Never re-enables I/O.
    pub fn set_level(&self, level: &str) {
        if let Ok(mut state) = self.inner.lock() {
            if let Some(level) = LogLevel::parse(level) {
                state.level = level;
            } else {
                state.disable();
            }
        }
    }
}
