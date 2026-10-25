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
    Environment,
    Conflict,
    Compatibility,
    Internal,
    Storage,
    Timeout,
    Cancelled,
    External,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorCode {
    #[serde(rename = "PA_PROVIDER_SELECTION_INVALID")]
    ProviderSelectionInvalid,
    #[serde(rename = "PA_PROVIDER_ADAPTER_UNAVAILABLE")]
    ProviderAdapterUnavailable,
    #[serde(rename = "PA_PROVIDER_DISABLED")]
    ProviderDisabled,
    #[serde(rename = "PA_PROVIDER_REFERENCED")]
    ProviderReferenced,
    #[serde(rename = "PA_PROVIDER_CAPABILITY_INVALID")]
    ProviderCapabilityInvalid,
    #[serde(rename = "PA_PROVIDER_QUEUE_FULL")]
    ProviderQueueFull,
    #[serde(rename = "PA_OPERATION_NOT_FOUND")]
    OperationNotFound,
    #[serde(rename = "PA_PROVIDER_ROLLBACK_FAILED")]
    ProviderRollbackFailed,
    #[serde(rename = "PA_PROVIDER_SHARED_ROUTE_UNSAFE")]
    ProviderSharedRouteUnsafe,
    #[serde(rename = "PA_PROVIDER_ENDPOINT_INVALID")]
    ProviderEndpointInvalid,
    #[serde(rename = "PA_PROVIDER_TEST_FAILED")]
    ProviderTestFailed,
    #[serde(rename = "PA_CODEX_NOT_INSTALLED")]
    CodexNotInstalled,
    #[serde(rename = "PA_CODEX_VERSION_UNSUPPORTED")]
    CodexVersionUnsupported,
    #[serde(rename = "PA_CODEX_REQUEST_TIMEOUT")]
    CodexRequestTimeout,
    #[serde(rename = "PA_CODEX_DISCONNECTED")]
    CodexDisconnected,
    #[serde(rename = "PA_PROCESS_CANCELLED")]
    ProcessCancelled,
    #[serde(rename = "PA_PROCESS_EXIT_FAILED")]
    ProcessExitFailed,
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
    #[serde(rename = "PA_CREDENTIALS_UNAVAILABLE")]
    CredentialsUnavailable,
    #[serde(rename = "PA_CREDENTIAL_INVALID")]
    CredentialInvalid,
    #[serde(rename = "PA_CREDENTIAL_PROFILE_NOT_FOUND")]
    CredentialProfileNotFound,
    #[serde(rename = "PA_CREDENTIAL_REVISION_CONFLICT")]
    CredentialRevisionConflict,
    #[serde(rename = "PA_CREDENTIAL_BUSY")]
    CredentialBusy,
    #[serde(rename = "PA_CREDENTIALS_STORAGE_FAILED")]
    CredentialsStorageFailed,
    #[serde(rename = "PA_PROJECT_NOT_FOUND")]
    ProjectNotFound,
    #[serde(rename = "PA_PROJECT_PICKER_FAILED")]
    ProjectPickerFailed,
    #[serde(rename = "PA_PROJECT_PATH_UNSAFE")]
    ProjectPathUnsafe,
    #[serde(rename = "PA_PROJECT_PATH_UNAVAILABLE")]
    ProjectPathUnavailable,
    #[serde(rename = "PA_PROJECT_PATH_CHANGED")]
    ProjectPathChanged,
    #[serde(rename = "PA_PROJECT_FILE_OUTSIDE_PROJECT")]
    ProjectFileOutsideProject,
    #[serde(rename = "PA_PROJECT_FILE_MISSING")]
    ProjectFileMissing,
    #[serde(rename = "PA_PROJECT_ACCESS_DENIED")]
    ProjectAccessDenied,
    #[serde(rename = "PA_PROJECT_OPEN_REJECTED")]
    ProjectOpenRejected,
    #[serde(rename = "PA_PROJECT_OPEN_TIMEOUT")]
    ProjectOpenTimeout,
    #[serde(rename = "PA_PROJECT_CONFLICT_REVISION")]
    ProjectConflictRevision,
}

#[derive(Clone, Serialize)]
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
            Self::ProviderSelectionInvalid => (
                ErrorCategory::Validation,
                "服务源或模型选择无效。",
                "请重新读取服务源、修订和模型能力，不会自动换站。",
            ),
            Self::ProviderAdapterUnavailable => (
                ErrorCategory::Compatibility,
                "此历史或官方服务源不可激活。",
                "请在应用内手动重建非官方自管 HTTPS profile；不会导入外部配置。",
            ),
            Self::ProviderDisabled => (
                ErrorCategory::Conflict,
                "服务源已停用。",
                "请在主窗口启用后重新读取并测试。",
            ),
            Self::ProviderReferenced => (
                ErrorCategory::Conflict,
                "此服务源仍有历史或默认选择引用。",
                "请停用服务源，或先清除默认选择与凭据；历史快照不会改写。",
            ),
            Self::ProviderCapabilityInvalid => (
                ErrorCategory::Compatibility,
                "模型能力未验证或已过期。",
                "请刷新隔离模型目录并重新选择支持的强度。",
            ),
            Self::ProviderQueueFull => (
                ErrorCategory::Conflict,
                "配置操作队列已满。",
                "最多允许八项未完成操作；请等待或取消后重试。",
            ),
            Self::OperationNotFound => (
                ErrorCategory::Validation,
                "本应用操作不存在。",
                "请使用本应用返回的操作 ID 查询，不会自动重放变更。",
            ),
            Self::ProviderRollbackFailed => (
                ErrorCategory::Internal,
                "AI 配置恢复未通过核验，新任务已阻止。",
                "请检查服务源后主动测试并重新切换，或清除默认配置；不会静默恢复任务。",
            ),
            Self::ProviderSharedRouteUnsafe => (
                ErrorCategory::Conflict,
                "无法核验服务源路由隔离，操作已拒绝。",
                "请使用独立 HTTPS 服务源，移除项目覆盖和共享可变代理后重新测试。",
            ),
            Self::ProviderEndpointInvalid => (
                ErrorCategory::Validation,
                "服务地址不满足安全要求。",
                "请填写有效的非官方 HTTPS Responses 地址，不包含认证信息、查询或片段。",
            ),
            Self::ProviderTestFailed => (
                ErrorCategory::External,
                "候选连接测试失败，原配置保留。",
                "请检查站点、凭据和模型组合后重新测试。",
            ),
            Self::CodexNotInstalled => (
                ErrorCategory::Environment,
                "Codex 可执行文件不可用。",
                "请选择已安装且受控的 Codex 可执行文件。",
            ),
            Self::CodexVersionUnsupported => (
                ErrorCategory::Compatibility,
                "Codex 协议或生效配置不兼容。",
                "请使用经本应用验证的客户端版本后重新测试。",
            ),
            Self::CodexRequestTimeout => (
                ErrorCategory::Timeout,
                "候选进程请求超时。",
                "候选资源将清理；请检查站点后重新测试。",
            ),
            Self::CodexDisconnected => (
                ErrorCategory::External,
                "自有 Codex 进程已断开。",
                "请检查客户端与站点后重新测试。",
            ),
            Self::ProcessCancelled => (
                ErrorCategory::Cancelled,
                "候选测试已取消。",
                "候选资源将清理，原配置保留。",
            ),
            Self::ProcessExitFailed => (
                ErrorCategory::Internal,
                "自有进程或临时资源清理失败。",
                "新候选将保持不可用；请退出应用后检查自有资源目录。",
            ),
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
            Self::CredentialsUnavailable => (
                ErrorCategory::Environment,
                "系统凭据库当前不可用。",
                "请检查 Windows 凭据服务和当前用户权限后重试；不会降级保存到文件或数据库。",
            ),
            Self::CredentialInvalid => (
                ErrorCategory::Validation,
                "凭据或服务源绑定无效。",
                "请检查 profile、HTTPS 服务源和 Key 格式后重试。",
            ),
            Self::CredentialProfileNotFound => (
                ErrorCategory::Validation,
                "服务源 profile 不存在。",
                "请先读取可用 profile，再提交凭据。",
            ),
            Self::CredentialRevisionConflict => (
                ErrorCategory::Conflict,
                "服务源已被其他操作修改。",
                "请先重新读取 profile 状态，再提交凭据变更。",
            ),
            Self::CredentialBusy => (
                ErrorCategory::Conflict,
                "该服务源仍有任务占用。",
                "等待相关任务结束后重试；凭据未修改。",
            ),
            Self::CredentialsStorageFailed => (
                ErrorCategory::Storage,
                "凭据状态保存失败，原可用状态已保留或已标记不可用。",
                "请检查应用数据目录权限和 Windows 凭据服务后重新读取状态。",
            ),
            Self::ProjectNotFound => (
                ErrorCategory::Validation,
                "项目记录不存在或当前不可用于此操作。",
                "请重新读取项目列表后重试。",
            ),
            Self::ProjectPickerFailed => (
                ErrorCategory::Environment,
                "本地项目选择器不可用。",
                "请检查桌面权限后重试；取消选择不会保存项目。",
            ),
            Self::ProjectPathUnsafe => (
                ErrorCategory::Permission,
                "项目路径不在允许的本机范围内。",
                "请从应用目录选择器重新选择本机项目目录。",
            ),
            Self::ProjectPathUnavailable => (
                ErrorCategory::Environment,
                "已登记的项目目录当前不可访问。",
                "请检查目录是否存在及权限；不会自动绑定其他目录。",
            ),
            Self::ProjectPathChanged => (
                ErrorCategory::Conflict,
                "项目目录身份已变化，需要重新授权。",
                "请重新选择原项目目录并确认授权。",
            ),
            Self::ProjectFileOutsideProject => (
                ErrorCategory::Permission,
                "文件目标超出项目授权目录。",
                "请选择项目目录内的文件。",
            ),
            Self::ProjectFileMissing => (
                ErrorCategory::Validation,
                "项目文件不存在。",
                "请重新读取项目后选择现有文件。",
            ),
            Self::ProjectAccessDenied => (
                ErrorCategory::Permission,
                "操作系统拒绝访问项目目标。",
                "请检查当前用户对项目目录的访问权限。",
            ),
            Self::ProjectOpenRejected => (
                ErrorCategory::External,
                "系统未接受项目打开请求。",
                "请检查系统默认程序或编辑器设置后主动重试。",
            ),
            Self::ProjectOpenTimeout => (
                ErrorCategory::Timeout,
                "等待系统接受项目打开请求超时。",
                "请查询项目状态后由用户决定是否再次打开。",
            ),
            Self::ProjectConflictRevision => (
                ErrorCategory::Conflict,
                "项目记录已被其他操作修改。",
                "请重新读取项目后再提交，不会自动覆盖。",
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
            retryable: matches!(
                self,
                Self::DesktopInitializationFailed
                    | Self::RevisionConflict
                    | Self::CredentialBusy
                    | Self::CredentialRevisionConflict
                    | Self::CodexRequestTimeout
                    | Self::CodexDisconnected
                    | Self::ProviderTestFailed
                    | Self::ProjectPathUnavailable
                    | Self::ProjectOpenTimeout
                    | Self::ProjectConflictRevision
            ),
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
