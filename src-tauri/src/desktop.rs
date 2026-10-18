//! Step 1.1 lifecycle only; business service status belongs to step 1.2.
use crate::ipc::{ErrorCode, Request};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct DesktopState {
    pub phase0: bool,
    fail_once: AtomicBool,
}

impl DesktopState {
    pub fn from_environment() -> Self {
        Self {
            phase0: cfg!(debug_assertions)
                && std::env::var("PA_DESKTOP_MODE").as_deref() == Ok("phase0"),
            fail_once: AtomicBool::new(
                cfg!(debug_assertions) && std::env::var("PA_SHELL_FAIL_ONCE").as_deref() == Ok("1"),
            ),
        }
    }

    pub fn initialize(&self, request: serde_json::Value) -> Result<DesktopReady, ErrorCode> {
        let request: Request<EmptyParams> =
            serde_json::from_value(request).map_err(|_| ErrorCode::InvalidRequest)?;
        if request.api_version != crate::ipc::API_VERSION {
            return Err(ErrorCode::VersionUnsupported);
        }
        if self.fail_once.swap(false, Ordering::SeqCst) {
            return Err(ErrorCode::DesktopInitializationFailed);
        }
        Ok(DesktopReady {
            mode: if self.phase0 { "phase0" } else { "normal" },
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyParams {}

#[derive(Serialize)]
pub struct DesktopReady {
    mode: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn startup_failure_is_fixed_and_retry_recovers() {
        let state = DesktopState {
            phase0: false,
            fail_once: AtomicBool::new(true),
        };
        let request = json!({"apiVersion": 1, "params": {}});
        assert!(matches!(
            state.initialize(request.clone()),
            Err(ErrorCode::DesktopInitializationFailed)
        ));
        assert_eq!(state.initialize(request).unwrap().mode, "normal");
    }

    #[test]
    fn invalid_requests_do_not_consume_failure_fixture() {
        let state = DesktopState {
            phase0: false,
            fail_once: AtomicBool::new(true),
        };
        assert!(matches!(
            state.initialize(json!({"apiVersion": 1, "params": {"secret": "synthetic"}})),
            Err(ErrorCode::InvalidRequest)
        ));
        assert!(matches!(
            state.initialize(json!({"apiVersion": 2, "params": {}})),
            Err(ErrorCode::VersionUnsupported)
        ));
        assert!(matches!(
            state.initialize(json!({"apiVersion": 1, "params": {}})),
            Err(ErrorCode::DesktopInitializationFailed)
        ));
    }
}
