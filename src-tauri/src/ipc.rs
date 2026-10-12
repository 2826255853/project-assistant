//! IPC v1 foundation; business status and events are added in step 1.2.
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
    Compatibility,
    Internal,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub enum ErrorCode {
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
                "测试消息超出长度限制。",
                "请将消息缩短至 4096 UTF-8 字节以内。",
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
            retryable: matches!(self, Self::DesktopInitializationFailed),
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
    serde_json::to_string(&ProbeLog {
        api_version: API_VERSION,
        occurred_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        level: if error.is_some() { "warn" } else { "info" },
        command: "framework_probe",
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
