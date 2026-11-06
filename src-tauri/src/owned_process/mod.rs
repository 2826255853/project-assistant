//! S1-06: private, bounded stdio candidate adapter. No task/IPC entry is exposed.
//! The future profile service supplies a revision-bound application profile.
#![allow(dead_code)] // Internal integration surface is consumed by step 1.8.
mod home;
mod route;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::{Job, spawn_in_job};

use crate::{
    credentials::{self, SecretBuffer},
    ipc::ErrorCode,
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tauri::Manager;
use url::Url;

const PROVIDER: &str = "project_assistant_owned";
pub(crate) const ADAPTER_VERSION: &str = "owned-stdio-relay-v3";
const MAX_FRAME: u64 = 1024 * 1024;
/// Task streams are deliberately smaller than the transport frame limit. A
/// task can produce many frames, so the gateway applies both per-frame and
/// cumulative limits before any event is handed to the UI layer.
const MAX_TASK_CONTENT: usize = 2 * 1024 * 1024;
const MAX_TASK_EVENTS: usize = 10_000;
const MAX_TASK_EVENTS_PER_SECOND: u32 = 120;
const MAX_TASK_PROMPT: usize = 64 * 1024;
const TASK_RUNTIME: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    Unsafe,
    Endpoint,
    Executable,
    Protocol,
    Timeout,
    Disconnected,
    Cancelled,
    Cleanup,
    TestFailed,
}
type Result<T> = std::result::Result<T, Failure>;
impl Failure {
    fn code(self) -> ErrorCode {
        match self {
            Self::Unsafe => ErrorCode::ProviderSharedRouteUnsafe,
            Self::Endpoint => ErrorCode::ProviderEndpointInvalid,
            Self::Executable => ErrorCode::CodexNotInstalled,
            Self::Protocol => ErrorCode::CodexVersionUnsupported,
            Self::Timeout => ErrorCode::CodexRequestTimeout,
            Self::Disconnected => ErrorCode::CodexDisconnected,
            Self::Cancelled => ErrorCode::ProcessCancelled,
            Self::Cleanup => ErrorCode::ProcessExitFailed,
            Self::TestFailed => ErrorCode::ProviderTestFailed,
        }
    }
}

/// Events emitted by the owned task gateway. These are intentionally
/// structured values; raw protocol frames never cross this boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskEvent {
    Started {
        external_session_id: String,
        external_turn_id: String,
    },
    Running,
    ReplyDelta {
        text: String,
    },
    Progress {
        text: String,
    },
    FileChanges {
        changes: Vec<FileChange>,
    },
    ApprovalRequired {
        item_id: String,
        external_request_id: String,
        operation_kind: String,
        operation_fingerprint: String,
        display_summary: String,
    },
    ApprovalPoll,
    ApprovalExpired,
    OperationCompleted {
        succeeded: bool,
    },
    ApprovalAcknowledged {
        approval_id: String,
        consume_request_id: String,
    },
    ApprovalAbandoned {
        approval_id: String,
        consume_request_id: String,
    },
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileChange {
    pub relative_path: String,
    pub kind: String,
    pub diff: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskOutcome {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovalReply {
    pub approval_id: String,
    pub consume_request_id: String,
    pub accepted: bool,
}
type TaskEventResult = std::result::Result<Option<ApprovalReply>, ErrorCode>;

fn safe_approval_text(value: &str, limit: usize) -> bool {
    let lower = value.to_ascii_lowercase();
    !value.trim().is_empty()
        && value.len() <= limit
        && !value.chars().any(char::is_control)
        && !value.chars().any(|ch| matches!(ch, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        && ![
            "secret",
            "password",
            "api_key",
            "api-key",
            "bearer ",
            "sk-",
            "pa_owned_route_token",
            "synthetic-",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
}

#[derive(Debug, Clone)]
pub(crate) struct TaskRun {
    pub external_session_id: String,
    pub external_turn_id: String,
    pub events: Vec<TaskEvent>,
    pub reply: String,
    pub progress: String,
    pub progress_bytes: usize,
    pub outcome: TaskOutcome,
    pub exit: ExitEvidence,
    pub fingerprint: String,
    pub upstream_requests: usize,
    pub verified_responses: usize,
}

#[derive(Default)]
struct Cancellation {
    requested: AtomicBool,
    dispatch: Mutex<()>,
}
#[derive(Clone, Default)]
pub(crate) struct Cancel(Arc<Cancellation>);
impl Cancel {
    pub(crate) fn request_after<T>(
        &self,
        action: impl FnOnce() -> std::result::Result<(T, bool), ErrorCode>,
    ) -> std::result::Result<T, ErrorCode> {
        let _dispatch = self
            .0
            .dispatch
            .lock()
            .map_err(|_| ErrorCode::ProcessExitFailed)?;
        let (result, accepted) = action()?;
        if accepted {
            self.0.requested.store(true, Ordering::Release);
        }
        Ok(result)
    }
    pub fn cancel(&self) {
        let _dispatch = self
            .0
            .dispatch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.0.requested.store(true, Ordering::Release);
    }
    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.requested.load(Ordering::Acquire)
    }
    fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Failure::Cancelled)
        } else {
            Ok(())
        }
    }
}
fn random_token() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| Failure::Unsafe)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn file_hash(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|_| Failure::Executable)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| Failure::Executable)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

// No normal debug/release application can enable test route trust.
fn acceptance_selection(selection: Selection) -> Result<Selection> {
    #[cfg(test)]
    {
        let mut selection = selection;
        if std::env::var("PA_IPC_VERIFY").as_deref() == Ok("1")
            && [
                "PA_TASK_DESKTOP_FIXTURE_ENDPOINT",
                "PA_TASK_DESKTOP_SECOND_ENDPOINT",
            ]
            .iter()
            .any(|name| std::env::var(name).ok().as_deref() == Some(selection.endpoint.as_str()))
        {
            let url = Url::parse(&selection.endpoint).map_err(|_| Failure::Unsafe)?;
            if url.scheme() != "https" || url.host_str() != Some("127.0.0.1") {
                return Err(Failure::Unsafe);
            }
            selection.fixture = true;
            selection.fixture_ca = Some(
                fs::read(std::env::var_os("PA_TASK_DESKTOP_CA").ok_or(Failure::Unsafe)?)
                    .map_err(|_| Failure::Unsafe)?,
            );
        }
        Ok(selection)
    }
    #[cfg(not(test))]
    {
        Ok(selection)
    }
}

fn valid_project_identity(value: &str) -> bool {
    value.len() == 48 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn relative_change_path(cwd: &Path, value: &str) -> Result<String> {
    let normalized = value.replace('\\', "/");
    let root = cwd
        .to_str()
        .ok_or(Failure::Unsafe)?
        .trim_start_matches(r"\\?\")
        .replace('\\', "/");
    let relative = if normalized.as_bytes().get(1) == Some(&b':') {
        let prefix = format!("{}/", root.trim_end_matches('/'));
        if !normalized
            .to_lowercase()
            .starts_with(&prefix.to_lowercase())
        {
            return Err(Failure::Unsafe);
        }
        normalized
            .get(prefix.len()..)
            .ok_or(Failure::Unsafe)?
            .to_owned()
    } else {
        normalized
    };
    if relative.len() > crate::project_contract::MAX_RELATIVE_PATH_BYTES
        || !crate::project_contract::valid_relative_path(&relative)
    {
        return Err(Failure::Unsafe);
    }
    Ok(relative)
}

/// Constructed by backend executable selection, never from a webview spawn
/// request or a PATH lookup. Hash changes require a new explicit selection.
pub(crate) struct Executable {
    path: PathBuf,
    hash: String,
    _pin: File,
}
impl Executable {
    pub(crate) fn identity(&self) -> &str {
        &self.hash
    }
    pub fn inspect(path: &Path) -> std::result::Result<Self, ErrorCode> {
        let path = home::check_path(path).map_err(|_| ErrorCode::CodexNotInstalled)?;
        if !path.is_file() || path.extension().and_then(|v| v.to_str()) != Some("exe") {
            return Err(ErrorCode::CodexNotInstalled);
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1); // read only: deny replacement/write through spawn and exit
        }
        let pin = options
            .open(&path)
            .map_err(|_| ErrorCode::CodexNotInstalled)?;
        let hash = file_hash(&path).map_err(Failure::code)?;
        Ok(Self {
            path,
            hash,
            _pin: pin,
        })
    }
    fn verify(&self) -> Result<()> {
        home::check_path(&self.path)?;
        if file_hash(&self.path)? != self.hash {
            return Err(Failure::Executable);
        }
        Ok(())
    }
}

pub(crate) struct CatalogEntry {
    pub model_id: String,
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
}

/// Configuration-only discovery: private home, fixed route and no turn/start.
/// Catalog declarations are not remote authentication/model-acceptance evidence.
pub(crate) fn catalog_candidate(
    app: &tauri::AppHandle,
    executable: &Executable,
    profile_id: &str,
    revision: i64,
) -> std::result::Result<Vec<CatalogEntry>, ErrorCode> {
    let state = app.state::<OwnedProcessState>();
    let cancel = Cancel::default();
    let _operation = state.register_operation(cancel.clone())?;
    let _serial = state.lock_serial_cancellable(&cancel)?;
    if state.blocked.load(Ordering::Acquire) {
        return Err(ErrorCode::ProcessExitFailed);
    }
    let credentials = app.state::<credentials::CredentialsState>();
    let _credential = credentials
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    let p = app
        .state::<crate::storage::StorageState>()
        .with_storage(|db| db.profiles().process_profile(profile_id))
        .map_err(|_| ErrorCode::SettingsStorageFailed)?
        .ok_or(ErrorCode::CredentialProfileNotFound)?;
    if p.config_revision != revision {
        return Err(ErrorCode::RevisionConflict);
    }
    if !p.enabled
        || p.adapter_kind != "self_managed"
        || p.external_profile_id.is_some()
        || p.source_kind != "relay"
        || p.wire_api != "responses"
        || p.auth_kind != "api_key"
    {
        return Err(ErrorCode::ProviderSharedRouteUnsafe);
    }
    let selection = Selection {
        profile_id: profile_id.into(),
        config_revision: revision,
        credential_revision: p.credential_revision.max(1),
        endpoint: p.endpoint,
        origin: p.endpoint_origin,
        model: "catalog-discovery".into(),
        effort: "medium".into(),
        task_tools: false,
        fixture: false,
        #[cfg(test)]
        fixture_ca: None,
    };
    let root = state
        .root
        .lock()
        .map_err(|_| ErrorCode::ProcessExitFailed)?
        .clone()
        .ok_or(ErrorCode::ProviderSharedRouteUnsafe)?;
    // The catalog never needs site authentication. An unexpected CLI request
    // cannot leak a real credential; no external credential is inherited.
    let secret = SecretBuffer::new(random_token().map_err(Failure::code)?.into_bytes());
    let mut candidate = Candidate::start(
        &root,
        executable,
        selection,
        secret,
        None,
        cancel,
        Duration::from_secs(45),
    )
    .map_err(Failure::code)?;
    let result = read_catalog(&mut candidate);
    let exit = candidate.close().map_err(|e| {
        state.blocked.store(true, Ordering::Release);
        e.code()
    })?;
    if exit.forced || exit.code != Some(0) {
        return Err(ErrorCode::ProcessExitFailed);
    }
    result.map_err(Failure::code)
}

fn read_catalog(candidate: &mut Candidate) -> Result<Vec<CatalogEntry>> {
    candidate.rpc("initialize",json!({"clientInfo":{"name":"project_assistant_catalog","version":"0.1.0"},"capabilities":{"experimentalApi":false}}))?;
    candidate.send(json!({"method":"initialized","params":{}}))?;
    let config = candidate.rpc(
        "config/read",
        json!({"includeLayers":true,"cwd":candidate.cwd}),
    )?;
    candidate.verify_effective_config(&config)?;
    let mut entries = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen = std::collections::HashSet::new();
    loop {
        let page = candidate.rpc("model/list", json!({"cursor":cursor,"limit":100}))?;
        for v in page
            .get("data")
            .and_then(Value::as_array)
            .ok_or(Failure::Protocol)?
        {
            let m = v
                .get("model")
                .and_then(Value::as_str)
                .ok_or(Failure::Protocol)?;
            let mut efforts = v
                .get("supportedReasoningEfforts")
                .and_then(Value::as_array)
                .ok_or(Failure::Protocol)?
                .iter()
                .map(|e| {
                    e.get("reasoningEffort")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .ok_or(Failure::Protocol)
                })
                .collect::<Result<Vec<_>>>()?;
            let mut default = v
                .get("defaultReasoningEffort")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if !crate::providers::valid_model(m)
                || entries.len() >= 512
                || efforts.is_empty()
                || efforts.len() > 16
            {
                return Err(Failure::Protocol);
            }
            efforts.retain(|e| {
                matches!(
                    e.as_str(),
                    "none" | "minimal" | "low" | "medium" | "high" | "xhigh"
                )
            });
            efforts.sort();
            efforts.dedup();
            if efforts.is_empty() {
                continue;
            }
            if default.as_ref().is_some_and(|d| !efforts.contains(d)) {
                default = None;
            }
            entries.push(CatalogEntry {
                model_id: m.into(),
                efforts,
                default_effort: default,
            });
        }
        cursor = match page.get("nextCursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(v)) if v.len() <= 1024 => Some(v.clone()),
            _ => return Err(Failure::Protocol),
        };
        let Some(next) = &cursor else { break };
        if !seen.insert(next.clone()) || seen.len() > 32 {
            return Err(Failure::Protocol);
        }
    }
    if candidate.route.evidence.requests.load(Ordering::Acquire) != 0 {
        return Err(Failure::Unsafe);
    }
    Ok(entries)
}

#[derive(Clone, Serialize)]
struct Selection {
    profile_id: String,
    config_revision: i64,
    credential_revision: i64,
    endpoint: String,
    origin: String,
    model: String,
    effort: String,
    #[serde(skip)]
    task_tools: bool,
    fixture: bool,
    #[cfg(test)]
    #[serde(skip)]
    fixture_ca: Option<Vec<u8>>,
}
impl Selection {
    fn validate_endpoint(&self) -> Result<Url> {
        let url = Url::parse(&self.endpoint).map_err(|_| Failure::Endpoint)?;
        let host = url.host_str().ok_or(Failure::Endpoint)?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || self.endpoint.len() > 2048
            || url.origin().ascii_serialization() != self.origin
            || self.config_revision < 1
            || self.credential_revision < 1
            || self.profile_id.is_empty()
            || self.profile_id.len() > 128
            || !self
                .profile_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || self.model.is_empty()
            || self.model.len() > 128
            || self.model.chars().any(char::is_control)
            || !matches!(
                self.effort.as_str(),
                "none" | "minimal" | "low" | "medium" | "high" | "xhigh"
            )
            || host == "openai.com"
            || host.ends_with(".openai.com")
        {
            return Err(Failure::Endpoint);
        }
        if self.fixture {
            // Only unit acceptance can construct this branch; no environment or
            // debug build switch can enable plaintext production destinations.
            if !cfg!(test) || host != "127.0.0.1" || !matches!(url.scheme(), "http" | "https") {
                return Err(Failure::Unsafe);
            }
        } else if url.scheme() != "https" {
            return Err(Failure::Endpoint);
        }
        Ok(url)
    }
    fn fingerprint(&self, executable: &Executable) -> Result<String> {
        let mut hash = Sha256::new();
        hash.update(ADAPTER_VERSION);
        hash.update(&executable.hash);
        hash.update(serde_json::to_vec(self).map_err(|_| Failure::Protocol)?);
        Ok(format!("{:x}", hash.finalize()))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbeEvidence {
    pub fingerprint: String,
    pub requests: usize,
    pub verified_responses: usize,
    pub exit: ExitEvidence,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExitEvidence {
    pub pid: u32,
    pub code: Option<i32>,
    pub forced: bool,
    pub cleaned: bool,
}

#[derive(Default)]
pub(crate) struct OwnedProcessState {
    root: Mutex<Option<PathBuf>>,
    serial: Mutex<()>,
    blocked: AtomicBool,
    lifecycle: Mutex<OwnedLifecycle>,
}
#[derive(Default)]
struct OwnedLifecycle {
    closing: bool,
    operations: HashMap<String, Cancel>,
}
struct OwnedOperation<'a> {
    state: &'a OwnedProcessState,
    id: String,
}
impl Drop for OwnedOperation<'_> {
    fn drop(&mut self) {
        if let Ok(mut lifecycle) = self.state.lifecycle.lock() {
            lifecycle.operations.remove(&self.id);
        }
    }
}
impl OwnedProcessState {
    fn register_operation(
        &self,
        cancel: Cancel,
    ) -> std::result::Result<OwnedOperation<'_>, ErrorCode> {
        let mut lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| ErrorCode::ProcessExitFailed)?;
        if lifecycle.closing {
            return Err(ErrorCode::ProcessCancelled);
        }
        let id = crate::ipc::correlation_id();
        lifecycle.operations.insert(id.clone(), cancel);
        Ok(OwnedOperation { state: self, id })
    }

    pub(crate) fn begin_shutdown(&self) {
        if let Ok(mut lifecycle) = self.lifecycle.lock() {
            lifecycle.closing = true;
            for cancel in lifecycle.operations.values() {
                cancel.cancel();
            }
        }
    }

    pub(crate) fn wait_for_shutdown(&self) {
        while self
            .lifecycle
            .lock()
            .map(|lifecycle| !lifecycle.operations.is_empty())
            .unwrap_or(false)
        {
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn initialize(&self, app_data_dir: &Path) -> std::result::Result<(), ErrorCode> {
        let parent = home::check_path(app_data_dir).map_err(Failure::code)?;
        let root = parent.join("owned-processes");
        if !root.exists() {
            #[cfg(windows)]
            windows::private_directory(&root).map_err(Failure::code)?;
        }
        let root = home::check_path(&root).map_err(Failure::code)?;
        home::cleanup_stale(&root).map_err(Failure::code)?;
        *self.root.lock().map_err(|_| ErrorCode::ProcessExitFailed)? = Some(root);
        self.blocked.store(false, Ordering::Release);
        Ok(())
    }

    pub(crate) fn root(&self) -> std::result::Result<PathBuf, ErrorCode> {
        self.root
            .lock()
            .map_err(|_| ErrorCode::ProcessExitFailed)?
            .clone()
            .ok_or(ErrorCode::ProviderSharedRouteUnsafe)
    }

    pub(crate) fn is_blocked(&self) -> bool {
        self.blocked.load(Ordering::Acquire)
    }

    pub(crate) fn block(&self) {
        self.blocked.store(true, Ordering::Release);
    }

    pub(crate) fn lock_serial(&self) -> std::result::Result<MutexGuard<'_, ()>, ErrorCode> {
        self.serial.lock().map_err(|_| ErrorCode::ProcessExitFailed)
    }

    pub(crate) fn lock_serial_cancellable(
        &self,
        cancel: &Cancel,
    ) -> std::result::Result<MutexGuard<'_, ()>, ErrorCode> {
        let deadline = Instant::now() + Duration::from_secs(5 * 60);
        loop {
            if cancel.is_cancelled() {
                return Err(ErrorCode::ProcessCancelled);
            }
            if Instant::now() >= deadline {
                return Err(ErrorCode::CodexRequestTimeout);
            }
            match self.serial.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(std::sync::TryLockError::WouldBlock) => {
                    thread::sleep(Duration::from_millis(25))
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err(ErrorCode::ProcessExitFailed);
                }
            }
        }
    }
}

/// Internal profile-service integration. The DB supplies every connection field;
/// a caller may only select the profile revision, model and effort. Holding the
/// existing credential-operation lock prevents replacement during the candidate.
pub(crate) fn probe_candidate(
    app: &tauri::AppHandle,
    executable: &Executable,
    profile_id: &str,
    expected_revision: i64,
    model: &str,
    effort: &str,
    cancel: Cancel,
) -> std::result::Result<ProbeEvidence, ErrorCode> {
    let state = app.state::<OwnedProcessState>();
    let _operation = state.register_operation(cancel.clone())?;
    let _serial = state.lock_serial_cancellable(&cancel)?;
    if state.blocked.load(Ordering::Acquire) {
        return Err(ErrorCode::ProcessExitFailed);
    }
    let credentials = app.state::<credentials::CredentialsState>();
    let _credential = credentials
        .operation
        .lock()
        .map_err(|_| ErrorCode::CredentialsStorageFailed)?;
    let selection = app
        .state::<crate::storage::StorageState>()
        .with_storage(|db| db.profiles().process_profile(profile_id))
        .map_err(|_| ErrorCode::SettingsStorageFailed)?
        .ok_or(ErrorCode::CredentialProfileNotFound)?;
    if selection.config_revision != expected_revision {
        return Err(ErrorCode::RevisionConflict);
    }
    if !selection.enabled
        || selection.adapter_kind != "self_managed"
        || selection.external_profile_id.is_some()
        || selection.source_kind != "relay"
        || selection.wire_api != "responses"
        || selection.auth_kind != "api_key"
    {
        return Err(ErrorCode::ProviderSharedRouteUnsafe);
    }
    let selected = Selection {
        profile_id: profile_id.into(),
        config_revision: selection.config_revision,
        credential_revision: selection.credential_revision,
        endpoint: selection.endpoint,
        origin: selection.endpoint_origin,
        model: model.into(),
        effort: effort.into(),
        task_tools: false,
        fixture: false,
        #[cfg(test)]
        fixture_ca: None,
    };
    selected.validate_endpoint().map_err(Failure::code)?;
    // Lock already held, so use the locked resolver rather than recursively
    // acquiring it. Origin/revision are checked again by the credential service.
    let secret = credentials::resolve_for_owned_locked(
        app,
        profile_id,
        selected.credential_revision,
        &selected.origin,
    )?;
    let root = state
        .root
        .lock()
        .map_err(|_| ErrorCode::ProcessExitFailed)?
        .clone()
        .ok_or(ErrorCode::ProviderSharedRouteUnsafe)?;
    let result = run_probe(
        &root,
        executable,
        selected,
        secret,
        None,
        cancel.clone(),
        Duration::from_secs(90),
    );
    if matches!(result, Err(Failure::Cleanup)) {
        state.blocked.store(true, Ordering::Release);
    }
    result.map_err(Failure::code)
}

fn run_probe(
    root: &Path,
    executable: &Executable,
    selection: Selection,
    secret: SecretBuffer,
    project: Option<&Path>,
    cancel: Cancel,
    timeout: Duration,
) -> Result<ProbeEvidence> {
    cancel.check()?;
    let selection = acceptance_selection(selection)?;
    selection.validate_endpoint()?;
    if let Some(project) = project {
        verify_project(project)?;
    }
    let mut candidate = Candidate::start(
        root, executable, selection, secret, project, cancel, timeout,
    )?;
    let result = candidate.probe();
    let exit = candidate.close()?;
    if result.is_ok() && (exit.forced || exit.code != Some(0)) {
        return Err(Failure::Cleanup);
    }
    result.map(
        |(fingerprint, requests, verified_responses)| ProbeEvidence {
            fingerprint,
            requests,
            verified_responses,
            exit,
        },
    )
}

/// Run one already-admitted task through the application-owned route. The
/// caller supplies the immutable snapshot fields and a verified project cwd;
/// this function never reads UI state or shared Codex configuration.
pub(crate) fn run_task(
    root: &Path,
    executable: &Executable,
    profile_id: &str,
    config_revision: i64,
    credential_revision: i64,
    endpoint: &str,
    origin: &str,
    model: &str,
    effort: &str,
    secret: SecretBuffer,
    project: &Path,
    project_identity: &str,
    prompt: &str,
    cancel: Cancel,
    mut on_event: impl FnMut(TaskEvent) -> TaskEventResult,
) -> std::result::Result<TaskRun, ErrorCode> {
    if prompt.is_empty()
        || prompt.len() > MAX_TASK_PROMPT
        || prompt
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t')
    {
        return Err(ErrorCode::InvalidRequest);
    }
    if !valid_project_identity(project_identity)
        || crate::project_paths::verify_task_directory_identity(project, project_identity).is_err()
    {
        return Err(ErrorCode::ProjectPathChanged);
    }
    let selection = Selection {
        profile_id: profile_id.to_owned(),
        config_revision,
        credential_revision,
        endpoint: endpoint.to_owned(),
        origin: origin.to_owned(),
        model: model.to_owned(),
        effort: effort.to_owned(),
        task_tools: true,
        fixture: false,
        #[cfg(test)]
        fixture_ca: None,
    };
    let mut candidate = Candidate::start(
        root,
        executable,
        selection,
        secret,
        Some(project),
        cancel.clone(),
        TASK_RUNTIME,
    )
    .map_err(Failure::code)?;
    candidate.expected_project_identity = Some(project_identity.to_owned());
    let callback_failure = std::cell::RefCell::new(None);
    let mut callback = |event| match on_event(event) {
        Ok(reply) => Ok(reply),
        Err(error) => {
            *callback_failure.borrow_mut() = Some(error);
            Err(error)
        }
    };
    let result = candidate.task(prompt, &mut callback);
    #[cfg(test)]
    if let Err(failure) = &result {
        eprintln!(
            "TASK_GATEWAY_FAILURE:{}:{:?}",
            candidate.stage,
            failure.code()
        );
    }
    let pid = candidate.child.id();
    let exit = candidate.close().map_err(Failure::code)?;
    if let Some(error) = callback_failure.into_inner() {
        return Err(error);
    }
    let result = result.map_err(Failure::code)?;
    if exit.forced || exit.code != Some(0) || !exit.cleaned {
        return Err(ErrorCode::ProcessExitFailed);
    }
    let mut result = result;
    result.exit.pid = pid;
    result.exit = exit;
    if result.outcome == TaskOutcome::Completed
        && (result.upstream_requests == 0 || result.verified_responses == 0)
    {
        return Err(ErrorCode::ProviderTestFailed);
    }
    Ok(result)
}

/// Loopback/temporary CA exist only in a test binary, never an application env flag.
#[cfg(all(test, windows))]
pub(crate) fn provider_acceptance_probe(
    root: &Path,
    exe: &Executable,
    id: &str,
    revision: i64,
    credential_revision: i64,
    endpoint: &str,
    model: &str,
    effort: &str,
    secret: SecretBuffer,
    ca: Vec<u8>,
    cancel: Cancel,
) -> std::result::Result<(), ErrorCode> {
    let selected = Selection {
        profile_id: id.into(),
        config_revision: revision,
        credential_revision,
        endpoint: endpoint.into(),
        origin: Url::parse(endpoint)
            .map_err(|_| ErrorCode::ProviderEndpointInvalid)?
            .origin()
            .ascii_serialization(),
        model: model.into(),
        effort: effort.into(),
        task_tools: false,
        fixture: true,
        fixture_ca: Some(ca),
    };
    let e = run_probe(
        root,
        exe,
        selected,
        secret,
        None,
        cancel,
        Duration::from_secs(25),
    )
    .map_err(Failure::code)?;
    if e.requests == 0
        || e.verified_responses == 0
        || !e.exit.cleaned
        || e.exit.forced
        || e.exit.code != Some(0)
    {
        return Err(ErrorCode::ProviderTestFailed);
    }
    Ok(())
}

#[cfg(all(test, windows))]
pub(crate) fn provider_acceptance_catalog(
    root: &Path,
    exe: &Executable,
    endpoint: &str,
    ca: Vec<u8>,
) -> std::result::Result<Vec<CatalogEntry>, ErrorCode> {
    let selected = Selection {
        profile_id: "catalog-acceptance".into(),
        config_revision: 1,
        credential_revision: 1,
        endpoint: endpoint.into(),
        origin: Url::parse(endpoint).unwrap().origin().ascii_serialization(),
        model: "catalog-discovery".into(),
        effort: "medium".into(),
        task_tools: false,
        fixture: true,
        fixture_ca: Some(ca),
    };
    let mut candidate = Candidate::start(
        root,
        exe,
        selected,
        SecretBuffer::new(random_token().map_err(Failure::code)?.into_bytes()),
        None,
        Cancel::default(),
        Duration::from_secs(45),
    )
    .map_err(Failure::code)?;
    let result = read_catalog(&mut candidate);
    let exit = candidate.close().map_err(Failure::code)?;
    if exit.forced || exit.code != Some(0) {
        return Err(ErrorCode::ProcessExitFailed);
    }
    result.map_err(Failure::code)
}

/// Until project configuration support is separately verified, any .codex layer
/// in cwd or an ancestor is rejected, including on-request/reparse directories.
fn verify_project(project: &Path) -> Result<PathBuf> {
    let project = home::check_path(project)?;
    if !project.is_dir() {
        return Err(Failure::Unsafe);
    }
    for ancestor in project.ancestors() {
        match fs::symlink_metadata(ancestor.join(".codex")) {
            Ok(_) => return Err(Failure::Unsafe),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Failure::Unsafe),
        }
        if fs::symlink_metadata(ancestor.join(".git")).is_ok() {
            break;
        }
    }
    Ok(project)
}

fn configuration(selection: &Selection, base_url: &str) -> String {
    // JSON string encoding is a subset of TOML basic strings for these validated
    // values. No external stanza, MCP, hook, plugin or trust data is imported.
    format!(
        "model_provider = \"{PROVIDER}\"\nmodel = {}\nmodel_reasoning_effort = {}\napproval_policy = \"on-request\"\nsandbox_mode = \"read-only\"\ncli_auth_credentials_store = \"file\"\nweb_search = \"disabled\"\n[history]\npersistence = \"none\"\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n[model_providers.{PROVIDER}]\nname = \"Project Assistant owned route\"\nbase_url = {}\nwire_api = \"responses\"\nenv_key = \"PA_OWNED_ROUTE_TOKEN\"\nrequires_openai_auth = false\nsupports_websockets = false\nrequest_max_retries = 0\nstream_max_retries = 0\nstream_idle_timeout_ms = 10000\n",
        json!(selection.model),
        json!(selection.effort),
        json!(base_url)
    )
}

fn stdout_frames(stdout: impl Read, sender: mpsc::SyncSender<Result<Value>>) {
    let mut reader = BufReader::new(stdout);
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        let read = (&mut reader)
            .take(MAX_FRAME + 1)
            .read_until(b'\n', &mut line);
        total += line.len() as u64;
        let value = match read {
            Ok(0) => break,
            Ok(_)
                if total <= MAX_FRAME
                    && line.len() as u64 <= MAX_FRAME
                    && line.ends_with(b"\n") =>
            {
                serde_json::from_slice(&line).map_err(|_| Failure::Protocol)
            }
            _ => Err(Failure::Protocol),
        };
        let failed = value.is_err();
        if sender.try_send(value).is_err() || failed {
            break;
        }
    }
}
fn stderr_drain(mut stderr: impl Read, sender: mpsc::SyncSender<Result<Value>>) {
    let mut bytes = [0; 8192];
    let mut total = 0;
    while let Ok(count) = stderr.read(&mut bytes) {
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_FRAME {
            let _ = sender.try_send(Err(Failure::Protocol));
            break;
        }
    }
}

struct Candidate {
    child: Child,
    stdin: Option<ChildStdin>,
    #[cfg(windows)]
    job: Option<windows::Job>,
    home: home::Home,
    route: route::Route,
    messages: Receiver<Result<Value>>,
    readers: Vec<JoinHandle<()>>,
    selection: Selection,
    cwd: PathBuf,
    expected_project_identity: Option<String>,
    config_hash: String,
    fingerprint: String,
    cancel: Cancel,
    deadline: Instant,
    next_id: u64,
    terminal: VecDeque<Value>,
    deferred: VecDeque<Value>,
    closed: bool,
    stage: &'static str,
    active_turn: Option<(String, String)>,
    approval_ttl: Duration,
}

struct TaskAccumulator {
    events: Vec<TaskEvent>,
    total_events: usize,
    reply: String,
    progress_bytes: usize,
    progress: String,
    window_started: Instant,
    window_events: u32,
    approval_requests: std::collections::HashSet<String>,
}
impl TaskAccumulator {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            total_events: 0,
            reply: String::new(),
            progress_bytes: 0,
            progress: String::new(),
            window_started: Instant::now(),
            window_events: 0,
            approval_requests: Default::default(),
        }
    }
    fn rate(&mut self) -> Result<()> {
        let now = Instant::now();
        if now.duration_since(self.window_started) >= Duration::from_secs(1) {
            self.window_started = now;
            self.window_events = 0;
        }
        self.window_events = self.window_events.saturating_add(1);
        self.total_events = self.total_events.saturating_add(1);
        if self.window_events > MAX_TASK_EVENTS_PER_SECOND || self.total_events > MAX_TASK_EVENTS {
            return Err(Failure::Protocol);
        }
        Ok(())
    }
    fn push(
        &mut self,
        event: TaskEvent,
        on_event: &mut dyn FnMut(TaskEvent) -> TaskEventResult,
    ) -> Result<Option<ApprovalReply>> {
        self.rate()?;
        match &event {
            TaskEvent::ReplyDelta { text } | TaskEvent::Progress { text }
                if text
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) =>
            {
                return Err(Failure::Protocol);
            }
            TaskEvent::ReplyDelta { text } => {
                if text.len() > 64 * 1024
                    || self.reply.len().saturating_add(text.len()) > MAX_TASK_CONTENT
                {
                    return Err(Failure::Protocol);
                }
                self.reply.push_str(text);
            }
            TaskEvent::Progress { text } => {
                if text.len() > 64 * 1024 {
                    return Err(Failure::Protocol);
                }
                self.progress_bytes = self.progress_bytes.saturating_add(text.len());
                if self.progress_bytes.saturating_add(self.reply.len()) > MAX_TASK_CONTENT {
                    return Err(Failure::Protocol);
                }
                self.progress.push_str(text);
            }
            _ => {}
        }
        self.events.push(event.clone());
        on_event(event).map_err(|_| Failure::Protocol)
    }
}

impl Candidate {
    fn start(
        root: &Path,
        executable: &Executable,
        selection: Selection,
        secret: SecretBuffer,
        project: Option<&Path>,
        cancel: Cancel,
        timeout: Duration,
    ) -> Result<Self> {
        let selection = acceptance_selection(selection)?;
        executable.verify()?;
        let home = home::Home::create(root)?;
        let cwd = verify_project(project.unwrap_or(&home.path.join("project")))?;
        let route = route::Route::start(&selection, secret, cancel.clone(), timeout)?;
        let config = configuration(&selection, &route.base_url);
        let config_path = home.path.join("codex/config.toml");
        fs::write(&config_path, &config).map_err(|_| Failure::Unsafe)?;
        let config_hash = file_hash(&config_path)?;
        let fingerprint = selection.fingerprint(executable)?;
        let mut command = Command::new(&executable.path);
        command
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(&cwd)
            .env_clear()
            .env("CODEX_HOME", home.path.join("codex"))
            .env("USERPROFILE", home.path.join("user"))
            .env("HOME", home.path.join("user"))
            .env("APPDATA", home.path.join("appdata"))
            .env("LOCALAPPDATA", home.path.join("localappdata"))
            .env("PROGRAMDATA", home.path.join("appdata"))
            .env("TEMP", home.path.join("temp"))
            .env("TMP", home.path.join("temp"))
            .env("PA_OWNED_ROUTE_TOKEN", &route.token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // No inherited PATH, proxy, API key, provider, Rust logging, Node preload,
        // shell setup or cc-switch variable survives this explicit allowlist.
        let system = std::env::var_os("SystemRoot").ok_or(Failure::Unsafe)?;
        let system = home::check_path(Path::new(&system))?;
        command
            .env("SystemRoot", &system)
            .env("WINDIR", &system)
            .env("PATH", system.join("System32"));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0004); // CREATE_NO_WINDOW | CREATE_SUSPENDED
        }
        let mut child = command.spawn().map_err(|_| Failure::Executable)?;
        #[cfg(windows)]
        let job = match windows::Job::attach_suspended(&child) {
            Ok(job) => Some(job),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or(Failure::Protocol)?;
        let stderr = child.stderr.take().ok_or(Failure::Protocol)?;
        let (sender, messages) = mpsc::sync_channel(64);
        let error_sender = sender.clone();
        let output_reader = thread::spawn(move || stdout_frames(stdout, sender));
        let error_reader = thread::spawn(move || stderr_drain(stderr, error_sender));
        Ok(Self {
            child,
            stdin,
            #[cfg(windows)]
            job,
            home,
            route,
            messages,
            readers: vec![output_reader, error_reader],
            selection,
            cwd,
            expected_project_identity: None,
            config_hash,
            fingerprint,
            cancel,
            deadline: Instant::now() + timeout,
            next_id: 0,
            terminal: VecDeque::new(),
            deferred: VecDeque::new(),
            closed: false,
            stage: "created",
            active_turn: None,
            approval_ttl: Duration::from_secs(5 * 60),
        })
    }
    fn guard(&self) -> Result<()> {
        self.cancel.check()?;
        if self.route.evidence.unsafe_route.load(Ordering::Acquire) {
            return Err(Failure::Unsafe);
        }
        if Instant::now() >= self.deadline {
            return Err(Failure::Timeout);
        }
        verify_project(&self.cwd)?;
        if let Some(identity) = &self.expected_project_identity {
            crate::project_paths::verify_task_directory_identity(&self.cwd, identity)
                .map_err(|_| Failure::Unsafe)?;
        }
        if file_hash(&self.home.path.join("codex/config.toml"))? != self.config_hash
            || self.home.path.join("codex/auth.json").exists()
        {
            return Err(Failure::Unsafe);
        }
        Ok(())
    }
    fn send(&mut self, value: Value) -> Result<()> {
        let cancel = self.cancel.clone();
        let _dispatch = cancel.0.dispatch.lock().map_err(|_| Failure::Unsafe)?;
        self.guard()?;
        let input = self.stdin.as_mut().ok_or(Failure::Disconnected)?;
        serde_json::to_writer(&mut *input, &value).map_err(|_| Failure::Disconnected)?;
        input
            .write_all(b"\n")
            .and_then(|_| input.flush())
            .map_err(|_| Failure::Disconnected)
    }
    fn receive(&mut self) -> Result<Value> {
        self.receive_until(self.deadline)
    }
    fn receive_until(&mut self, deadline: Instant) -> Result<Value> {
        loop {
            self.guard()?;
            if let Some(value) = self.deferred.pop_front() {
                return Ok(value);
            }
            if Instant::now() >= deadline {
                return Err(Failure::Timeout);
            }
            match self.messages.recv_timeout(Duration::from_millis(25)) {
                Ok(value) => return value,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Failure::Disconnected),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self
                        .child
                        .try_wait()
                        .map_err(|_| Failure::Disconnected)?
                        .is_some()
                    {
                        return Err(Failure::Disconnected);
                    }
                }
            }
        }
    }
    fn defer_while_awaiting_approval(&mut self, thread_id: &str, turn_id: &str) -> Result<()> {
        loop {
            match self.messages.try_recv() {
                Ok(value) => {
                    let value = value?;
                    let method = value
                        .get("method")
                        .and_then(Value::as_str)
                        .ok_or(Failure::Protocol)?;
                    if value.get("id").is_some()
                        || matches!(
                            method,
                            "turn/completed" | "error" | "serverRequest/resolved"
                        )
                    {
                        return Err(Failure::Protocol);
                    }
                    if value
                        .pointer("/params/threadId")
                        .and_then(Value::as_str)
                        .is_some_and(|id| id != thread_id)
                        || value
                            .pointer("/params/turnId")
                            .and_then(Value::as_str)
                            .is_some_and(|id| id != turn_id)
                        || self.deferred.len() >= 64
                    {
                        return Err(Failure::Protocol);
                    }
                    self.deferred.push_back(value);
                }
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => return Err(Failure::Disconnected),
            }
        }
    }
    fn event(&mut self, message: Value) -> Result<()> {
        if let Some(id) = message.get("id") {
            if message
                .get("method")
                .and_then(Value::as_str)
                .is_some_and(|v| v.ends_with("/requestApproval"))
            {
                self.send(json!({"id":id,"result":{"decision":"decline"}}))?;
            } else {
                self.send(
                    json!({"id":id,"error":{"code":-32601,"message":"Unsupported client request"}}),
                )?;
            }
        } else if message.get("method").and_then(Value::as_str) == Some("turn/completed") {
            if self.terminal.len() >= 8 {
                return Err(Failure::Protocol);
            }
            self.terminal.push_back(message);
        }
        Ok(())
    }
    fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        let request_deadline = Instant::now() + Duration::from_secs(30);
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        loop {
            if Instant::now() >= request_deadline {
                return Err(Failure::Timeout);
            }
            let value = self.receive_until(request_deadline)?;
            if value.get("method").is_some() {
                self.event(value)?;
                continue;
            }
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                return Err(Failure::Protocol);
            }
            if value.get("error").is_some() {
                return Err(Failure::TestFailed);
            }
            return value.get("result").cloned().ok_or(Failure::Protocol);
        }
    }
    fn probe(&mut self) -> Result<(String, usize, usize)> {
        self.stage = "initialize";
        self.rpc("initialize", json!({"clientInfo":{"name":"project_assistant_owned","version":"0.1.0"},"capabilities":{"experimentalApi":false}}))?;
        self.send(json!({"method":"initialized","params":{}}))?;
        self.stage = "config";
        let read = self.rpc("config/read", json!({"includeLayers":true,"cwd":self.cwd}))?;
        self.verify_effective_config(&read)?;
        self.stage = "thread";
        let started = self.rpc("thread/start", json!({"cwd":self.cwd,"ephemeral":true,
            "model":self.selection.model,"modelProvider":PROVIDER,"sandbox":"read-only","approvalPolicy":"on-request",
            "config":{"model_reasoning_effort":self.selection.effort}}))?;
        if started.get("model").and_then(Value::as_str) != Some(&self.selection.model)
            || started.get("modelProvider").and_then(Value::as_str) != Some(PROVIDER)
            || started.get("reasoningEffort").and_then(Value::as_str)
                != Some(&self.selection.effort)
            || started
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(|v| Path::new(v).canonicalize().ok())
                .as_ref()
                != Some(&self.cwd)
        {
            return Err(Failure::Unsafe);
        }
        let thread_id = started
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or(Failure::Protocol)?
            .to_owned();
        // Guard is repeated immediately before the sole network-producing call.
        self.stage = "turn";
        let turn = self.rpc(
            "turn/start",
            json!({"threadId":thread_id,"model":self.selection.model,"effort":self.selection.effort,
            "input":[{"type":"text","text":"Do not use tools. Reply OK.","text_elements":[]}]}),
        )?;
        let turn_id = turn
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or(Failure::Protocol)?
            .to_owned();
        self.active_turn = Some((thread_id.clone(), turn_id.clone()));
        loop {
            if let Some(index) = self.terminal.iter().position(|event| {
                event.pointer("/params/threadId").and_then(Value::as_str) == Some(&thread_id)
                    && event.pointer("/params/turn/id").and_then(Value::as_str) == Some(&turn_id)
            }) {
                let terminal = self.terminal.remove(index).ok_or(Failure::Protocol)?;
                self.active_turn = None;
                self.guard()?;
                if terminal
                    .pointer("/params/turn/status")
                    .and_then(Value::as_str)
                    != Some("completed")
                {
                    return Err(Failure::TestFailed);
                }
                let count = self.route.evidence.verified.load(Ordering::Acquire);
                if count == 0 {
                    return Err(Failure::Unsafe);
                }
                return Ok((
                    self.fingerprint.clone(),
                    self.route.evidence.requests.load(Ordering::Acquire),
                    count,
                ));
            }
            let value = self.receive()?;
            if value.get("method").is_some() {
                self.event(value)?;
            }
        }
    }

    fn task(
        &mut self,
        prompt: &str,
        on_event: &mut dyn FnMut(TaskEvent) -> TaskEventResult,
    ) -> Result<TaskRun> {
        self.stage = "initialize";
        self.rpc(
            "initialize",
            json!({
                "clientInfo":{"name":"project_assistant_task","version":"0.1.0"},
                "capabilities":{"experimentalApi":false}
            }),
        )?;
        self.send(json!({"method":"initialized","params":{}}))?;
        self.stage = "config";
        let read = self.rpc("config/read", json!({"includeLayers":true,"cwd":self.cwd}))?;
        self.verify_effective_config(&read)?;
        self.stage = "thread";
        let started = self.rpc(
            "thread/start",
            json!({
                "cwd":self.cwd,
                "ephemeral":true,
                "model":self.selection.model,
                "modelProvider":PROVIDER,
                "sandbox":"read-only",
                "approvalPolicy":"on-request",
                "config":{"model_reasoning_effort":self.selection.effort}
            }),
        )?;
        if started.get("model").and_then(Value::as_str) != Some(&self.selection.model)
            || started.get("modelProvider").and_then(Value::as_str) != Some(PROVIDER)
            || started.get("reasoningEffort").and_then(Value::as_str)
                != Some(&self.selection.effort)
            || started
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(|v| Path::new(v).canonicalize().ok())
                .as_ref()
                != Some(&self.cwd)
        {
            return Err(Failure::Unsafe);
        }
        let thread_id = started
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or(Failure::Protocol)?
            .to_owned();
        let mut acc = TaskAccumulator::new();
        let mut pending_approval_ack: Option<(String, String, ApprovalReply)> = None;
        acc.push(TaskEvent::Running, on_event)?;
        self.stage = "turn";
        let turn = self.rpc(
            "turn/start",
            json!({
                "threadId":thread_id,
                "model":self.selection.model,
                "effort":self.selection.effort,
                "input":[{"type":"text","text":prompt,"text_elements":[]}]
            }),
        )?;
        let turn_id = turn
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or(Failure::Protocol)?
            .to_owned();
        self.active_turn = Some((thread_id.clone(), turn_id.clone()));
        acc.push(
            TaskEvent::Started {
                external_session_id: thread_id.clone(),
                external_turn_id: turn_id.clone(),
            },
            on_event,
        )?;
        loop {
            let value = if let Some(index) = self.terminal.iter().position(|event| {
                event.pointer("/params/threadId").and_then(Value::as_str) == Some(&thread_id)
                    && event.pointer("/params/turn/id").and_then(Value::as_str) == Some(&turn_id)
            }) {
                self.terminal.remove(index).ok_or(Failure::Protocol)?
            } else {
                self.receive()?
            };
            if value.get("method").is_none() {
                return Err(Failure::Protocol);
            }
            if let Some(outcome) = self.task_event(
                value,
                &thread_id,
                &turn_id,
                &mut acc,
                &mut pending_approval_ack,
                on_event,
            )? {
                self.active_turn = None;
                return Ok(TaskRun {
                    external_session_id: thread_id,
                    external_turn_id: turn_id,
                    events: acc.events,
                    reply: acc.reply,
                    progress_bytes: acc.progress_bytes,
                    progress: acc.progress,
                    outcome,
                    exit: ExitEvidence {
                        pid: 0,
                        code: None,
                        forced: false,
                        cleaned: false,
                    },
                    fingerprint: self.fingerprint.clone(),
                    upstream_requests: self.route.evidence.requests.load(Ordering::Acquire),
                    verified_responses: self.route.evidence.verified.load(Ordering::Acquire),
                });
            }
        }
    }

    fn task_event(
        &mut self,
        message: Value,
        thread_id: &str,
        turn_id: &str,
        acc: &mut TaskAccumulator,
        pending_approval_ack: &mut Option<(String, String, ApprovalReply)>,
        on_event: &mut dyn FnMut(TaskEvent) -> TaskEventResult,
    ) -> Result<Option<TaskOutcome>> {
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .ok_or(Failure::Protocol)?;
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        if method.ends_with("/requestApproval") {
            if params.get("threadId").and_then(Value::as_str) != Some(thread_id)
                || params.get("turnId").and_then(Value::as_str) != Some(turn_id)
            {
                return Err(Failure::Protocol);
            }
            let id = message.get("id").cloned().ok_or(Failure::Protocol)?;
            let _item_id = params
                .get("itemId")
                .and_then(Value::as_str)
                .filter(|v| {
                    !v.is_empty() && v.len() <= 96 && v.bytes().all(|b| b.is_ascii_graphic())
                })
                .ok_or(Failure::Protocol)?;
            // Current fileChange callbacks contain only grantRoot, not a bound
            // patch. They cannot grant an unseen patch or a session-wide root.
            let command = params
                .get("command")
                .and_then(Value::as_str)
                .filter(|value| {
                    safe_approval_text(value, 1800) && !value.contains(&self.route.token)
                });
            let cwd_matches = params
                .get("cwd")
                .and_then(Value::as_str)
                .and_then(|value| Path::new(value).canonicalize().ok())
                .as_ref()
                == Some(&self.cwd);
            let command_supported = method == "item/commandExecution/requestApproval"
                && params
                    .get("environmentId")
                    .is_none_or(|value| value.is_null() || value.as_str() == Some("local"))
                && params
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_none_or(|kind| kind == "command")
                && cwd_matches
                && command.is_some()
                && ["networkApprovalContext", "proposedNetworkPolicyAmendments"]
                    .iter()
                    .all(|key| params.get(*key).is_none_or(Value::is_null));
            let operation_kind = if command_supported {
                "command"
            } else {
                "unknown"
            };
            let display_summary = if command_supported {
                format!("单次执行命令：{}", command.unwrap())
            } else {
                "无法安全绑定的操作，已拒绝。".to_owned()
            };
            let request_bytes = serde_json::to_vec(&params).map_err(|_| Failure::Protocol)?;
            if request_bytes.len() > MAX_FRAME as usize {
                return Err(Failure::Protocol);
            }
            let mut digest = Sha256::new();
            digest.update(method.as_bytes());
            digest.update([0]);
            digest.update(request_bytes);
            digest.update([0]);
            digest.update(serde_json::to_vec(&id).map_err(|_| Failure::Protocol)?);
            digest.update([0]);
            digest.update(self.cwd.to_string_lossy().as_bytes());
            if let Some(identity) = &self.expected_project_identity {
                digest.update(identity.as_bytes());
            }
            let fingerprint = format!("{:x}", digest.finalize());
            let request_id = match &id {
                Value::String(value) => value.clone(),
                Value::Number(value) => value.to_string(),
                _ => return Err(Failure::Protocol),
            };
            if !acc.approval_requests.insert(request_id.clone()) {
                return Err(Failure::Protocol);
            }
            let item_id = _item_id.to_owned();
            if pending_approval_ack.is_some() {
                return Err(Failure::Protocol);
            }
            let mut reply = acc.push(
                TaskEvent::ApprovalRequired {
                    item_id: item_id.clone(),
                    external_request_id: request_id,
                    operation_kind: operation_kind.to_owned(),
                    operation_fingerprint: fingerprint,
                    display_summary,
                },
                on_event,
            )?;
            let expiry = Instant::now() + self.approval_ttl;
            while reply.is_none() {
                self.guard()?;
                self.defer_while_awaiting_approval(thread_id, turn_id)?;
                if self
                    .child
                    .try_wait()
                    .map_err(|_| Failure::Disconnected)?
                    .is_some()
                {
                    return Err(Failure::Disconnected);
                }
                if Instant::now() >= expiry {
                    on_event(TaskEvent::ApprovalExpired).map_err(|_| Failure::Protocol)?;
                    break;
                }
                reply = on_event(TaskEvent::ApprovalPoll).map_err(|_| Failure::Protocol)?;
                if reply.is_none() {
                    thread::sleep(Duration::from_millis(25));
                }
            }
            let decision = if reply.as_ref().is_some_and(|value| value.accepted) {
                "accept"
            } else {
                "decline"
            };
            self.defer_while_awaiting_approval(thread_id, turn_id)?;
            self.send(json!({"id":id,"result":{"decision":decision}}))?;
            if let Some(reply) = reply {
                *pending_approval_ack = Some((
                    item_id,
                    serde_json::to_string(&id).map_err(|_| Failure::Protocol)?,
                    reply,
                ));
            }
            return Ok(None);
        }
        if let Some(actual_thread) = params.get("threadId").and_then(Value::as_str) {
            if actual_thread != thread_id {
                return Err(Failure::Protocol);
            }
        }
        if let Some(actual_turn) = params.get("turnId").and_then(Value::as_str) {
            if actual_turn != turn_id {
                return Err(Failure::Protocol);
            }
        }
        if let Some(actual_turn) = params.pointer("/turn/id").and_then(Value::as_str) {
            if actual_turn != turn_id {
                return Err(Failure::Protocol);
            }
        }
        if method == "serverRequest/resolved" || method == "item/completed" {
            if let Some((expected_item, request_id, claim)) = pending_approval_ack.as_ref() {
                let request_matches = method == "serverRequest/resolved"
                    && params.get("threadId").and_then(Value::as_str) == Some(thread_id)
                    && params
                        .get("requestId")
                        .and_then(|value| serde_json::to_string(value).ok())
                        .as_ref()
                        == Some(request_id);
                let item_matches = method == "item/completed"
                    && params.get("threadId").and_then(Value::as_str) == Some(thread_id)
                    && params.get("turnId").and_then(Value::as_str) == Some(turn_id)
                    && params.pointer("/item/id").and_then(Value::as_str) == Some(expected_item);
                if request_matches || item_matches {
                    acc.push(
                        TaskEvent::ApprovalAcknowledged {
                            approval_id: claim.approval_id.clone(),
                            consume_request_id: claim.consume_request_id.clone(),
                        },
                        on_event,
                    )?;
                    *pending_approval_ack = None;
                }
            }
        }
        match method {
            "item/completed"
                if params.pointer("/item/type").and_then(Value::as_str) == Some("fileChange") =>
            {
                if params.get("threadId").and_then(Value::as_str) != Some(thread_id)
                    || params.get("turnId").and_then(Value::as_str) != Some(turn_id)
                    || params.pointer("/item/status").and_then(Value::as_str) != Some("completed")
                {
                    return Err(Failure::Protocol);
                }
                let items = params
                    .pointer("/item/changes")
                    .and_then(Value::as_array)
                    .ok_or(Failure::Protocol)?;
                if items.len() > 100 {
                    return Err(Failure::Protocol);
                }
                let mut changes = Vec::new();
                for item in items {
                    let path = item
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or(Failure::Protocol)?;
                    let relative = relative_change_path(&self.cwd, path)?;
                    let kind = match item.pointer("/kind/type").and_then(Value::as_str) {
                        Some("add") => "added",
                        Some("delete") => "deleted",
                        Some("update")
                            if item.pointer("/kind/move_path").is_none_or(Value::is_null) =>
                        {
                            "modified"
                        }
                        _ => return Err(Failure::Unsafe),
                    };
                    let diff = item
                        .get("diff")
                        .and_then(Value::as_str)
                        .ok_or(Failure::Protocol)?;
                    if diff.contains(&self.route.token)
                        || diff.len() > 64 * 1024
                        || diff
                            .chars()
                            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
                    {
                        return Err(Failure::Unsafe);
                    }
                    changes.push(FileChange {
                        relative_path: relative,
                        kind: kind.into(),
                        diff: diff.into(),
                    });
                }
                acc.push(TaskEvent::FileChanges { changes }, on_event)?;
            }
            "item/completed"
                if params.pointer("/item/type").and_then(Value::as_str)
                    == Some("commandExecution") =>
            {
                acc.push(
                    TaskEvent::OperationCompleted {
                        succeeded: params.pointer("/item/exitCode").and_then(Value::as_i64)
                            == Some(0),
                    },
                    on_event,
                )?;
            }
            "turn/started" => {
                acc.push(TaskEvent::Running, on_event)?;
            }
            "item/agentMessage/delta" => {
                let text = params
                    .get("delta")
                    .and_then(Value::as_str)
                    .ok_or(Failure::Protocol)?;
                if text.contains(&self.route.token) {
                    return Err(Failure::Unsafe);
                }
                acc.push(
                    TaskEvent::ReplyDelta {
                        text: text.to_owned(),
                    },
                    on_event,
                )?;
            }
            "item/commandExecution/outputDelta"
            | "item/fileChange/outputDelta"
            | "item/commandExecution/started"
            | "item/fileChange/started" => {
                let text = params
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if text.contains(&self.route.token) {
                    return Err(Failure::Unsafe);
                }
                acc.push(TaskEvent::Progress { text }, on_event)?;
            }
            "turn/completed" => {
                if let Some((_, _, claim)) = pending_approval_ack.take() {
                    acc.push(
                        TaskEvent::ApprovalAbandoned {
                            approval_id: claim.approval_id,
                            consume_request_id: claim.consume_request_id,
                        },
                        on_event,
                    )?;
                }
                let status = params
                    .pointer("/turn/status")
                    .and_then(Value::as_str)
                    .ok_or(Failure::Protocol)?;
                return match status {
                    "completed" => {
                        acc.push(TaskEvent::Completed, on_event)?;
                        Ok(Some(TaskOutcome::Completed))
                    }
                    "interrupted" | "cancelled" => {
                        acc.push(TaskEvent::Cancelled, on_event)?;
                        Ok(Some(TaskOutcome::Cancelled))
                    }
                    "failed" => {
                        acc.push(TaskEvent::Failed, on_event)?;
                        Ok(Some(TaskOutcome::Failed))
                    }
                    _ => Err(Failure::Protocol),
                };
            }
            "error" => {
                acc.push(TaskEvent::Failed, on_event)?;
                return Ok(Some(TaskOutcome::Failed));
            }
            _ => {
                // Unknown notifications carry no executable authority. Ignore
                // them after applying the event-rate bound; the next state is
                // still determined by a known terminal event or timeout.
                acc.rate()?;
            }
        }
        Ok(None)
    }
    fn verify_effective_config(&mut self, value: &Value) -> Result<()> {
        self.stage = "config-selection";
        let config = value.get("config").ok_or(Failure::Protocol)?;
        if config.get("model_provider").and_then(Value::as_str) != Some(PROVIDER)
            || config.get("model").and_then(Value::as_str) != Some(&self.selection.model)
            || config.get("model_reasoning_effort").and_then(Value::as_str)
                != Some(&self.selection.effort)
        {
            return Err(Failure::Unsafe);
        }
        self.stage = "config-provider";
        let provider = config
            .pointer(&format!("/model_providers/{PROVIDER}"))
            .ok_or(Failure::Unsafe)?;
        if provider.get("base_url").and_then(Value::as_str) != Some(&self.route.base_url)
            || provider.get("wire_api").and_then(Value::as_str) != Some("responses")
            || provider.get("env_key").and_then(Value::as_str) != Some("PA_OWNED_ROUTE_TOKEN")
            || provider
                .get("requires_openai_auth")
                .and_then(Value::as_bool)
                != Some(false)
        {
            return Err(Failure::Unsafe);
        }
        self.stage = "config-extensions";
        for key in ["mcp_servers", "plugins"] {
            if config
                .get(key)
                .is_some_and(|v| !v.is_null() && v.as_object().is_none_or(|o| !o.is_empty()))
            {
                return Err(Failure::Unsafe);
            }
        }
        if config
            .get("notify")
            .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
        {
            return Err(Failure::Unsafe);
        }
        // Loaded system/requirements/project layers cannot silently inject a
        // second route or executable extension. Only our exact user file and
        // the empty command-line layer are accepted.
        self.stage = "config-layers";
        let layers = value
            .get("layers")
            .and_then(Value::as_array)
            .ok_or(Failure::Protocol)?;
        for layer in layers {
            let name = layer
                .pointer("/name/type")
                .and_then(Value::as_str)
                .ok_or(Failure::Protocol)?;
            match name {
                "user" => {
                    let file = layer
                        .pointer("/name/file")
                        .and_then(Value::as_str)
                        .ok_or(Failure::Protocol)?;
                    if Path::new(file).canonicalize().ok()
                        != self.home.path.join("codex/config.toml").canonicalize().ok()
                    {
                        return Err(Failure::Unsafe);
                    }
                }
                "sessionFlags" | "commandLine" | "system"
                    if layer
                        .get("config")
                        .and_then(Value::as_object)
                        .is_some_and(|o| o.is_empty()) => {}
                _ => return Err(Failure::Unsafe),
            }
        }
        Ok(())
    }
    fn close(&mut self) -> Result<ExitEvidence> {
        self.close_inspecting(|_| {})
    }
    fn close_inspecting(&mut self, inspect: impl FnOnce(&Path)) -> Result<ExitEvidence> {
        if self.closed {
            return Err(Failure::Cleanup);
        }
        if let (Some((thread_id, turn_id)), Some(stdin)) =
            (self.active_turn.take(), self.stdin.as_mut())
        {
            self.next_id += 1;
            let message = json!({"id":self.next_id,"method":"turn/interrupt","params":{"threadId":thread_id,"turnId":turn_id}});
            let _ = serde_json::to_writer(&mut *stdin, &message)
                .and_then(|_| stdin.write_all(b"\n").map_err(serde_json::Error::io));
        }
        self.route.close()?;
        self.stdin.take();
        let started = Instant::now();
        let mut forced = false;
        let status = loop {
            if let Some(status) = self.child.try_wait().map_err(|_| Failure::Cleanup)? {
                break status;
            }
            if started.elapsed() > Duration::from_secs(2) {
                forced = true;
                #[cfg(windows)]
                if let Some(job) = &self.job {
                    job.terminate();
                }
                let _ = self.child.kill();
                break self.child.wait().map_err(|_| Failure::Cleanup)?;
            }
            thread::sleep(Duration::from_millis(10));
        };
        // Close the Job even after a graceful main-process exit; no descendant
        // can retain stdout/stderr or an owned resource past this boundary.
        #[cfg(windows)]
        self.job.take();
        self.route.close()?;
        for reader in self.readers.drain(..) {
            reader.join().map_err(|_| Failure::Cleanup)?;
        }
        inspect(&self.home.path);
        self.home.cleanup()?;
        self.closed = true;
        Ok(ExitEvidence {
            pid: self.child.id(),
            code: status.code(),
            forced,
            cleaned: true,
        })
    }
}
impl Drop for Candidate {
    fn drop(&mut self) {
        if !self.closed {
            #[cfg(windows)]
            if let Some(job) = &self.job {
                job.terminate();
            }
            let _ = self.child.kill();
            let _ = self.close();
        }
    }
}
