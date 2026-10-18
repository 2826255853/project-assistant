//! IPC v1 strict requests, caller checks, public errors and fixed log fields.
use serde::{Deserialize, Serialize};
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

pub const API_VERSION: u32 = 1;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request<T> {
    pub api_version: u32,
    pub params: T,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Validation,
    Permission,
    Conflict,
    Compatibility,
    Internal,
    Storage,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub enum ErrorCode {
    #[serde(rename = "PA_INTERNAL_UNEXPECTED")]
    InternalUnexpected,
    #[serde(rename = "PA_LOG_CLEANUP_FAILED")]
    LogCleanupFailed,
    #[serde(rename = "PA_AUDIT_WRITE_FAILED")]
    AuditWriteFailed,
    #[serde(rename = "PA_SETTINGS_INVALID_VALUE")]
    SettingsInvalidValue,
    #[serde(rename = "PA_CONFLICT_REVISION")]
    RevisionConflict,
    #[serde(rename = "PA_SETTINGS_CORRUPT")]
    SettingsCorrupt,
    #[serde(rename = "PA_SETTINGS_STORAGE_FAILED")]
    SettingsStorageFailed,
    #[serde(rename = "PA_SETTINGS_RECOVERY_FAILED")]
    SettingsRecoveryFailed,
    #[serde(rename = "PA_SETTINGS_AI_UNAVAILABLE")]
    SettingsAiUnavailable,
    #[serde(rename = "PA_IPC_INVALID_REQUEST")]
    InvalidRequest,
    #[serde(rename = "PA_IPC_VERSION_UNSUPPORTED")]
    VersionUnsupported,
    #[serde(rename = "PA_VALIDATION_EMPTY_MESSAGE")]
    EmptyMessage,
    #[serde(rename = "PA_VALIDATION_INPUT_TOO_LARGE")]
    InputTooLarge,
    #[serde(rename = "PA_DESKTOP_INITIALIZATION_FAILED")]
    DesktopInitializationFailed,
    #[serde(rename = "PA_IPC_PERMISSION_DENIED")]
    PermissionDenied,
    #[serde(rename = "PA_OVERLAY_CONTROL_FAILED")]
    OverlayControlFailed,
    #[serde(rename = "PA_SHORTCUT_INVALID")]
    ShortcutInvalid,
    #[serde(rename = "PA_SHORTCUT_CONFLICT")]
    ShortcutConflict,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub category: ErrorCategory,
    pub code: ErrorCode,
    pub message: &'static str,
    pub suggestion: &'static str,
    pub retryable: bool,
}

impl ErrorCode {
    pub fn public(self) -> AppError {
        let (category, message, suggestion) = match self {
            Self::InternalUnexpected => (
                ErrorCategory::Internal,
                "本地日志已停止写入。",
                "请检查磁盘空间和应用日志目录权限，处理后重新启动应用。",
            ),
            Self::LogCleanupFailed => (
                ErrorCategory::Storage,
                "应用日志清理失败。",
                "请检查应用日志目录权限；目录身份或链接校验失败时不会继续清理。",
            ),
            Self::AuditWriteFailed => (
                ErrorCategory::Storage,
                "安全审计保存失败，操作未提交。",
                "请检查磁盘空间和数据库权限，重新读取状态后再提交。",
            ),
            Self::SettingsInvalidValue => (
                ErrorCategory::Validation,
                "设置值无效。",
                "请检查允许值、摘要期限和编辑器绝对路径后重试。",
            ),
            Self::RevisionConflict => (
                ErrorCategory::Conflict,
                "设置已被其他入口修改。",
                "请先重新读取设置，再确认并提交；不会自动覆盖。",
            ),
            Self::SettingsCorrupt => (
                ErrorCategory::Storage,
                "本机设置数据损坏。",
                "原值已保留，请退出应用并按存储恢复说明检查数据库或旧设置文件。",
            ),
            Self::SettingsStorageFailed => (
                ErrorCategory::Storage,
                "设置读取或保存失败。",
                "请检查本机存储和权限后重试；原设置未被覆盖。",
            ),
            Self::SettingsRecoveryFailed => (
                ErrorCategory::Internal,
                "常驻设置恢复失败。",
                "请明确退出后重启应用，核对设置与常驻入口状态。",
            ),
            Self::SettingsAiUnavailable => (
                ErrorCategory::Compatibility,
                "AI 默认配置服务尚未启用。",
                "请等待配置服务就绪；当前保持未配置。",
            ),
            Self::InvalidRequest => (
                ErrorCategory::Validation,
                "请求格式无效。",
                "请刷新应用后重试；持续失败时更新应用。",
            ),
            Self::VersionUnsupported => (
                ErrorCategory::Compatibility,
                "前后端接口版本不兼容。",
                "请使用同一版本的前端与桌面程序。",
            ),
            Self::EmptyMessage => (
                ErrorCategory::Validation,
                "测试消息不能为空。",
                "请输入内容后重试。",
            ),
            Self::InputTooLarge => (
                ErrorCategory::Validation,
                "输入超出长度限制。",
                "请缩短输入后重试。",
            ),
            Self::PermissionDenied => (
                ErrorCategory::Permission,
                "当前窗口无权执行此操作。",
                "请使用应用主窗口中的对应入口。",
            ),
            Self::OverlayControlFailed => (
                ErrorCategory::Internal,
                "常驻入口操作失败。",
                "请从托盘重试；持续失败时重新启动应用。",
            ),
            Self::ShortcutInvalid => (
                ErrorCategory::Validation,
                "快捷键格式无效。",
                "请更换快捷键组合后重试。",
            ),
            Self::ShortcutConflict => (
                ErrorCategory::Conflict,
                "快捷键已被系统或其他应用占用。",
                "请更换快捷键组合；悬浮球仍可使用。",
            ),
            Self::DesktopInitializationFailed => (
                ErrorCategory::Internal,
                "桌面服务暂时不可用。",
                "请重试初始化；持续失败时关闭并重新启动应用。",
            ),
        };
        AppError {
            category,
            code: self,
            message,
            suggestion,
            retryable: matches!(self, Self::DesktopInitializationFailed | Self::RevisionConflict),
        }
    }
}

#[derive(Serialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Response<T> {
    Ok {
        api_version: u32,
        correlation_id: String,
        data: T,
    },
    Error {
        api_version: u32,
        correlation_id: String,
        error: AppError,
    },
}

pub fn correlation_id() -> String {
    static INSTANCE: OnceLock<u128> = OnceLock::new();
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let instance = INSTANCE.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    });
    format!(
        "ipc-{}-{instance:x}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

impl<T> Response<T> {
    pub fn from_result(correlation_id: String, result: Result<T, ErrorCode>) -> Self {
        match result {
            Ok(data) => Self::Ok {
                api_version: API_VERSION,
                correlation_id,
                data,
            },
            Err(code) => Self::Error {
                api_version: API_VERSION,
                correlation_id,
                error: code.public(),
            },
        }
    }
}

// Fixed command/event/code fields only. No arbitrary string payload or raw error.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeLog<'a> {
    api_version: u32,
    occurred_at: u128,
    level: &'static str,
    command: &'static str,
    event: &'static str,
    correlation_id: &'a str,
    outcome: &'static str,
    error_code: Option<ErrorCode>,
}
pub fn probe_log(correlation_id: &str, error: Option<ErrorCode>) -> String {
    command_log("framework_probe", correlation_id, error)
}

pub fn command_log(
    command: &'static str,
    correlation_id: &str,
    error: Option<ErrorCode>,
) -> String {
    serde_json::to_string(&ProbeLog {
        api_version: API_VERSION,
        occurred_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        level: if error.is_some() { "warn" } else { "info" },
        command,
        event: "ipc_completed",
        correlation_id,
        outcome: if error.is_some() {
            "failed"
        } else {
            "succeeded"
        },
        error_code: error,
    })
    .expect("fixed log fields serialize")
}

pub fn parse_request<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
) -> Result<T, ErrorCode> {
    let request: Request<T> =
        serde_json::from_value(value).map_err(|_| ErrorCode::InvalidRequest)?;
    if request.api_version != API_VERSION {
        return Err(ErrorCode::VersionUnsupported);
    }
    Ok(request.params)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}

// Capability checks are supplemented with the actual caller origin, never a UI field.
pub fn authorize_origin(label: &str, url: &str, main_only: bool) -> Result<(), ErrorCode> {
    let owned = label == "main" || (!main_only && label == "overlay");
    let local = url == "tauri://localhost/"
        || url.starts_with("tauri://localhost/")
        || url.starts_with("http://tauri.localhost/")
        || url.starts_with("https://tauri.localhost/")
        || (cfg!(debug_assertions) && url.starts_with("http://localhost:1420/"));
    if owned && local {
        Ok(())
    } else {
        Err(ErrorCode::PermissionDenied)
    }
}

pub fn authorize(window: &tauri::WebviewWindow, main_only: bool) -> Result<(), ErrorCode> {
    let url = window.url().map_err(|_| ErrorCode::PermissionDenied)?;
    authorize_origin(window.label(), url.as_str(), main_only)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProbeParams {
    pub message: String,
}

#[derive(Serialize)]
pub struct ProbeReply {
    pub message: String,
    pub platform: &'static str,
    pub architecture: &'static str,
}

pub fn probe(request: serde_json::Value) -> Result<ProbeReply, ErrorCode> {
    let request: Request<ProbeParams> =
        serde_json::from_value(request).map_err(|_| ErrorCode::InvalidRequest)?;
    if request.api_version != API_VERSION {
        return Err(ErrorCode::VersionUnsupported);
    }
    if request.params.message.len() > 4096 {
        return Err(ErrorCode::InputTooLarge);
    }
    let message = request.params.message.trim();
    if message.is_empty() {
        return Err(ErrorCode::EmptyMessage);
    }
    Ok(ProbeReply {
        message: format!("Rust 已收到：{message}"),
        platform: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn actual_caller_labels_and_origins_are_required() {
        assert!(authorize_origin("main", "http://tauri.localhost/index.html", true).is_ok());
        assert!(authorize_origin("overlay", "http://tauri.localhost/index.html", false).is_ok());
        for (label, origin, main_only) in [
            ("other", "http://tauri.localhost/index.html", false),
            ("overlay", "http://tauri.localhost/index.html", true),
            ("main", "https://example.com/", false),
            ("main", "http://localhost:1420.evil.test/", false),
            ("main", "http://localhost:1421/", false),
            ("main", "http://tauri.localhost.evil.test/", false),
        ] {
            assert!(matches!(
                authorize_origin(label, origin, main_only),
                Err(ErrorCode::PermissionDenied)
            ));
        }
    }

    #[test]
    fn malformed_and_incompatible_requests_are_safe() {
        let secret = "synthetic-key-DO-NOT-LOG";
        for request in [
            json!({"apiVersion":1,"params":{"message":"ok","key":secret}}),
            json!({"apiVersion":1,"params":{"message":false}}),
            json!({"params":{}}),
            json!({"apiVersion":1,"params":{"message":"ok"},"correlationId":secret}),
        ] {
            let response = Response::<ProbeReply>::from_result(correlation_id(), probe(request));
            let wire = serde_json::to_string(&response).unwrap();
            assert!(wire.contains("PA_IPC_INVALID_REQUEST"));
            assert!(!wire.contains(secret));
        }
        assert!(matches!(
            probe(json!({"apiVersion":2,"params":{"message":"ok"}})),
            Err(ErrorCode::VersionUnsupported)
        ));
    }

    #[test]
    fn response_union_and_utf8_limits() {
        let id = correlation_id();
        let reply = Response::from_result(
            id.clone(),
            probe(json!({"apiVersion":1,"params":{"message":" 中文 & <> "}})),
        );
        let wire = serde_json::to_value(reply).unwrap();
        assert_eq!(wire["status"], "ok");
        assert_eq!(wire["apiVersion"], 1);
        assert_eq!(wire["correlationId"], id);
        assert_eq!(wire["data"]["message"], "Rust 已收到：中文 & <>");
        assert!(wire.get("error").is_none());
        assert!(matches!(
            probe(json!({"apiVersion":1,"params":{"message":" "}})),
            Err(ErrorCode::EmptyMessage)
        ));
        assert!(probe(json!({"apiVersion":1,"params":{"message":"a".repeat(4096)}})).is_ok());
        assert!(matches!(
            probe(json!({"apiVersion":1,"params":{"message":"中".repeat(1366)}})),
            Err(ErrorCode::InputTooLarge)
        ));
    }

    #[test]
    fn correlation_ids_are_unique_and_logs_exclude_payload() {
        let ids: std::collections::HashSet<_> = (0..1000).map(|_| correlation_id()).collect();
        assert_eq!(ids.len(), 1000);
        let id = correlation_id();
        let secret = "synthetic-key-DO-NOT-LOG\nforged-entry";
        let result = probe(json!({"apiVersion":1,"params":{"message":secret}}));
        let log = probe_log(&id, result.as_ref().err().copied());
        let value: serde_json::Value = serde_json::from_str(&log).unwrap();
        assert_eq!(value["correlationId"], id);
        assert!(!log.contains(secret));
        assert!(!log.contains("message"));
        assert!(!log.contains('\n'));
        let error = serde_json::to_value(Response::<()>::from_result(
            id,
            Err(ErrorCode::EmptyMessage),
        ))
        .unwrap();
        assert!(error.get("data").is_none());
        assert_eq!(error["error"]["retryable"], false);
    }
}
